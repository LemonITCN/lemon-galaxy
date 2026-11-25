//! 复制粘贴文件，被控端这一侧（docs/protocol.md 8.4.1）：主控端发来的剪贴板文件清单，内容等有人粘贴时
//! 经反向流（被控端向主控端开的流）去读。平台层拿到 [`ClipSource`]，把清单延迟放进系统剪贴板：
//! Windows 的 IStream、Linux 的 FUSE、macOS 的标记文件 + 进度窗口在真正粘贴时才调 [`ClipSource::open`]。

use galaxy_client::clipoffer::{wait_open, BlockingReader};
use galaxy_net::frame;
use galaxy_proto::peer::{ClipOffer, ClipReadReply, ReverseOpen};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// 清单内容的来源。
pub trait ClipSource: Send + Sync {
    /// 打开清单第 `index` 项（必须是文件）从 `offset` 读到末尾，返回（文件现在的大小，读取器）。
    /// 会阻塞到对方应答：在普通线程或 spawn_blocking 里调用，不能在异步任务里。读取器丢掉即取消。
    fn open(&self, index: u32, offset: u64) -> Result<(u64, Box<dyn Read + Send>), String>;
}

/// 经反向流向发清单的主控端读。会话断了以后打开、读都会报错。
pub struct QuicClipSource {
    conn: quinn::Connection,
    offer_id: u64,
    rt: tokio::runtime::Handle,
}

impl QuicClipSource {
    pub fn new(conn: quinn::Connection, offer_id: u64, rt: tokio::runtime::Handle) -> Self {
        Self { conn, offer_id, rt }
    }
}

const CHUNK: usize = 256 * 1024;
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

impl ClipSource for QuicClipSource {
    fn open(&self, index: u32, offset: u64) -> Result<(u64, Box<dyn Read + Send>), String> {
        let (tx, reader) = BlockingReader::channel();
        let (conn, offer_id) = (self.conn.clone(), self.offer_id);
        let size = wait_open(&self.rt, async move {
            let (mut s, mut r) = conn.open_bi().await.map_err(|e| format!("主控端已断开：{e}"))?;
            frame::write(&mut s, &ReverseOpen::ClipboardRead { offer_id, index, offset }).await.map_err(|e| e.to_string())?;
            let _ = s.finish();
            let size = match frame::read::<ClipReadReply>(&mut r).await.map_err(|e| e.to_string())? {
                Some(ClipReadReply::Ok { size }) => size,
                Some(ClipReadReply::Error(e)) => return Err(e.message),
                None => return Err("主控端不支持粘贴时才传文件（请升级主控端）".into()),
            };
            tokio::spawn(async move {
                loop {
                    match tokio::time::timeout(IDLE_TIMEOUT, r.read_chunk(CHUNK, true)).await {
                        Ok(Ok(Some(c))) => {
                            if !tx.data(c.bytes).await {
                                // 粘贴方不要了：让主控端别再发
                                let _ = r.stop(0u32.into());
                                return;
                            }
                        }
                        Ok(Ok(None)) => return tx.end().await,
                        Ok(Err(e)) => return tx.fail(format!("传输中断：{e}")).await,
                        Err(_) => return tx.fail("传输超时").await,
                    }
                }
            });
            Ok(size)
        })?;
        Ok((size, Box::new(reader)))
    }
}

/// 同名时按访达的规则换名：「报告.docx」→「报告 2.docx」→「报告 3.docx」…
pub fn free_name(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    if !p.exists() {
        return p;
    }
    (2u32..)
        .map(|n| dir.join(galaxy_client::clipoffer::numbered(name, n)))
        .find(|p| !p.exists())
        .expect("总能找到空名字")
}

/// 把整份清单粘贴进 `dir`（顶层项同名时加编号），返回写出的顶层路径。`progress(已写字节, 总字节)`；`cancel` 置位即停。
/// 平台层没有更好的做法时用（Linux 没有 FUSE 的兜底、端到端测试）。会阻塞，在普通线程里调用。
pub fn paste_to_dir(offer: &ClipOffer, source: &dyn ClipSource, dir: &Path, mut progress: impl FnMut(u64, u64), cancel: &AtomicBool) -> Result<Vec<PathBuf>, String> {
    offer.validate()?;
    let total = offer.total_bytes();
    let mut done = 0u64;
    let mut tops = vec![];
    // 顶层项（可能改了名）→ 实际路径；子项按清单里的相对路径接在它后面
    let mut roots: std::collections::HashMap<&str, PathBuf> = Default::default();
    progress(0, total);
    for (i, e) in offer.entries.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err("已取消".into());
        }
        let (top, rest) = e.path.split_once('/').map(|(a, b)| (a, Some(b))).unwrap_or((e.path.as_str(), None));
        let base = match rest {
            None => {
                let p = free_name(dir, top);
                tops.push(p.clone());
                roots.insert(top, p.clone());
                p
            }
            Some(rest) => roots.get(top).ok_or("清单顺序不对")?.join(rest),
        };
        if e.is_dir {
            std::fs::create_dir_all(&base).map_err(|x| format!("无法创建 {}：{x}", base.display()))?;
            continue;
        }
        let (_, mut r) = source.open(i as u32, 0)?;
        let mut f = std::fs::File::create(&base).map_err(|x| format!("无法创建 {}：{x}", base.display()))?;
        let mut buf = vec![0u8; CHUNK];
        loop {
            if cancel.load(Ordering::Relaxed) {
                drop(f);
                let _ = std::fs::remove_file(&base);
                return Err("已取消".into());
            }
            let n = r.read(&mut buf).map_err(|x| x.to_string())?;
            if n == 0 {
                break;
            }
            f.write_all(&buf[..n]).map_err(|x| format!("写入 {} 失败：{x}", base.display()))?;
            done += n as u64;
            progress(done.min(total), total);
        }
    }
    progress(total, total);
    Ok(tops)
}

/// 端到端测试（环境变量 LEMON_GALAXY_TEST_CLIP_DIR）：收到清单立刻「粘贴」进那个目录，再从中间位置读一次核对，
/// 结果写到目录里的 `.lg-clip-result`（`ok <顶层项数>` 或 `error: 原因`）。
pub fn test_paste(offer: ClipOffer, source: std::sync::Arc<dyn ClipSource>, dir: PathBuf) {
    std::thread::spawn(move || {
        let r = (|| -> Result<usize, String> {
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let tops = paste_to_dir(&offer, &*source, &dir, |_, _| {}, &AtomicBool::new(false))?;
            // 断点续读：从第一个够大的文件中间读，和完整读到的后半段比对
            if let Some((i, e)) = offer.entries.iter().enumerate().find(|(_, e)| !e.is_dir && e.size >= 2) {
                let half = e.size / 2;
                let (size, mut r) = source.open(i as u32, half)?;
                let mut tail = vec![];
                r.read_to_end(&mut tail).map_err(|x| x.to_string())?;
                let top = e.path.split('/').next().unwrap_or_default();
                let local = tops.iter().find(|p| p.file_name().map(|n| n == top).unwrap_or(false)).ok_or("找不到粘贴出的文件")?;
                let full = std::fs::read(local.parent().unwrap().join(&e.path)).map_err(|x| x.to_string())?;
                if size != e.size || full[half as usize..] != tail[..] {
                    return Err(format!("从中间读到的内容不对（{} 字节）", tail.len()));
                }
            }
            Ok(tops.len())
        })();
        let text = match r {
            Ok(n) => format!("ok {n}\n"),
            Err(e) => format!("error: {e}\n"),
        };
        let _ = std::fs::write(dir.join(".lg-clip-result"), text);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use galaxy_proto::peer::ClipOfferEntry;
    use std::sync::Arc;

    /// 内存里的来源：清单第 i 项的内容是 contents[i]
    struct Mem(Vec<Vec<u8>>);
    impl ClipSource for Mem {
        fn open(&self, index: u32, offset: u64) -> Result<(u64, Box<dyn Read + Send>), String> {
            let d = self.0.get(index as usize).ok_or("没有")?.clone();
            let size = d.len() as u64;
            Ok((size, Box::new(std::io::Cursor::new(d[offset as usize..].to_vec()))))
        }
    }

    fn entry(path: &str, dir: bool, size: u64) -> ClipOfferEntry {
        ClipOfferEntry { path: path.into(), is_dir: dir, size, mtime: 0 }
    }

    #[test]
    fn pastes_tree_and_renames_like_finder() {
        let d = std::env::temp_dir().join(format!("lg-host-clip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("资料")).unwrap();
        std::fs::write(d.join("a.txt"), b"old").unwrap();
        let offer = ClipOffer { id: 7, entries: vec![entry("资料", true, 0), entry("资料/x.bin", false, 5), entry("a.txt", false, 3)] };
        let src = Mem(vec![vec![], b"hello".to_vec(), b"new".to_vec()]);
        let mut last = (0, 0);
        let tops = paste_to_dir(&offer, &src, &d, |a, b| last = (a, b), &AtomicBool::new(false)).unwrap();
        assert_eq!(tops, vec![d.join("资料 2"), d.join("a 2.txt")]);
        assert_eq!(std::fs::read(d.join("资料 2/x.bin")).unwrap(), b"hello");
        assert_eq!(std::fs::read(d.join("a 2.txt")).unwrap(), b"new");
        assert_eq!(std::fs::read(d.join("a.txt")).unwrap(), b"old");
        assert_eq!(last, (8, 8));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn test_paste_writes_result_and_checks_offset_read() {
        let d = std::env::temp_dir().join(format!("lg-host-clip-t-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let offer = ClipOffer { id: 9, entries: vec![entry("dir", true, 0), entry("dir/big.bin", false, 10)] };
        test_paste(offer, Arc::new(Mem(vec![vec![], b"0123456789".to_vec()])), d.clone());
        let res = d.join(".lg-clip-result");
        for _ in 0..100 {
            if res.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(std::fs::read_to_string(&res).unwrap(), "ok 1\n");
        assert_eq!(std::fs::read(d.join("dir/big.bin")).unwrap(), b"0123456789");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rejects_unsafe_offer() {
        let d = std::env::temp_dir();
        let offer = ClipOffer { id: 1, entries: vec![entry("../evil", false, 1)] };
        assert!(paste_to_dir(&offer, &Mem(vec![b"x".to_vec()]), &d, |_, _| {}, &AtomicBool::new(false)).is_err());
    }
}
