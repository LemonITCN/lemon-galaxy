//! 复制粘贴文件：复制时只交换清单，粘贴时才传内容（docs/protocol.md 8.4.1）。
//!
//! - 本机复制 → 远端粘贴：[`PeerSession::clipboard_offer_files`] 把本机文件展开成清单发给被控端；
//!   对方粘贴时被控端开反向流来读，由 [`serve_reverse`] 应答（只认本会话最近几份清单里的文件）。
//! - 远端复制 → 本机粘贴：[`remote_offer`] 把远端剪贴板里的文件路径展开成清单（不下载），
//!   粘贴时 [`RemoteOffer::open`] 按项从任意位置读。
//!
//! 平台层（Windows 的 IStream、Linux 的 FUSE、macOS 的进度窗口线程）都是同步读，
//! 所以读取器是 [`BlockingReader`]：异步任务往通道里送数据块，调用方在自己的线程里阻塞读。

use crate::error::{ClientError, Result};
use crate::session::PeerSession;
use bytes::Bytes;
use galaxy_net::frame;
use galaxy_proto::peer::{ClipOffer, ClipOfferEntry, ClipReadReply, FileKind, FileMsg, FileOp, PeerError, ReverseOpen, StreamOpen, CLIP_OFFER_MAX_ENTRIES};
use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// 一次读多少
const CHUNK: usize = 256 * 1024;
/// 主控端为对方保留最近几份清单（对方可能还在粘贴上一份）
const KEEP_OFFERS: usize = 4;
/// 打开（等对方应答）最多等多久
const OPEN_TIMEOUT: Duration = Duration::from_secs(20);
/// 读的过程中多久没有新数据算断了
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

fn other(e: impl std::fmt::Display) -> ClientError {
    ClientError::Other(e.to_string())
}

// ---------------- 阻塞读取器 ----------------

enum Msg {
    Data(Bytes),
    End,
    Err(String),
}

/// 同步读取器：数据由运行时里的异步任务送来。丢掉它时任务发现通道关了就停，并关掉底下的流。
/// 在普通线程或 spawn_blocking 里读，不能在异步任务里读。
pub struct BlockingReader {
    rx: mpsc::Receiver<Msg>,
    cur: Bytes,
    done: bool,
}

/// 异步任务那头：往读取器送数据
pub struct ChunkTx(mpsc::Sender<Msg>);

impl ChunkTx {
    /// 送一块；读取器已经不要了返回 false
    pub async fn data(&self, b: Bytes) -> bool {
        self.0.send(Msg::Data(b)).await.is_ok()
    }
    pub async fn end(self) {
        let _ = self.0.send(Msg::End).await;
    }
    pub async fn fail(self, e: impl Into<String>) {
        let _ = self.0.send(Msg::Err(e.into())).await;
    }
    /// 读取器还要不要
    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }
}

impl BlockingReader {
    pub fn channel() -> (ChunkTx, Self) {
        let (tx, rx) = mpsc::channel(8);
        (ChunkTx(tx), Self { rx, cur: Bytes::new(), done: false })
    }
}

impl Read for BlockingReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if !self.cur.is_empty() {
                let n = buf.len().min(self.cur.len());
                buf[..n].copy_from_slice(&self.cur.split_to(n));
                return Ok(n);
            }
            if self.done || buf.is_empty() {
                return Ok(0);
            }
            // 在普通线程或 spawn_blocking 里调用；在异步任务里阻塞会被 tokio 拒绝（panic）
            match self.rx.blocking_recv() {
                Some(Msg::Data(b)) => self.cur = b,
                Some(Msg::End) => self.done = true,
                Some(Msg::Err(e)) => return Err(std::io::Error::other(e)),
                None => return Err(std::io::Error::other("传输中断")),
            }
        }
    }
}

/// 在普通线程（或 spawn_blocking）里等一个异步结果（打开时等对方应答用）。不能在异步任务里调。
pub fn wait_open<T: Send + 'static>(rt: &tokio::runtime::Handle, fut: impl std::future::Future<Output = std::result::Result<T, String>> + Send + 'static) -> std::result::Result<T, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    rt.spawn(async move {
        let r = tokio::time::timeout(OPEN_TIMEOUT, fut).await.unwrap_or_else(|_| Err("对方没有应答".into()));
        let _ = tx.send(r);
    });
    rx.recv().unwrap_or_else(|_| Err("传输中断".into()))
}

// ---------------- 本机复制 → 远端粘贴 ----------------

/// 本机发出的一份清单：每项对应的本机文件（文件夹为 None）。
pub struct LocalOffer {
    pub offer: ClipOffer,
    files: Vec<Option<PathBuf>>,
}

#[derive(Default)]
pub(crate) struct LocalOffers(parking_lot::Mutex<VecDeque<Arc<LocalOffer>>>);

impl LocalOffers {
    fn add(&self, o: Arc<LocalOffer>) {
        let mut q = self.0.lock();
        q.push_back(o);
        while q.len() > KEEP_OFFERS {
            q.pop_front();
        }
    }
    fn file(&self, offer_id: u64, index: u32) -> Option<PathBuf> {
        let q = self.0.lock();
        let o = q.iter().find(|o| o.offer.id == offer_id)?;
        o.files.get(index as usize)?.clone()
    }
}

fn mtime_secs(md: &std::fs::Metadata) -> i64 {
    md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// 把本机路径展开成清单（目录在内容前面，同一目录里按名字排序）。符号链接跟随，其他特殊文件跳过。
pub fn plan_local(paths: &[PathBuf]) -> Result<LocalOffer> {
    if paths.is_empty() {
        return Err(other("没有要复制的文件"));
    }
    let mut entries = vec![];
    let mut files = vec![];
    fn walk(p: &Path, rel: String, entries: &mut Vec<ClipOfferEntry>, files: &mut Vec<Option<PathBuf>>) -> Result<()> {
        let md = std::fs::metadata(p).map_err(|e| other(format!("读不了 {}：{e}", p.display())))?;
        if md.is_dir() {
            entries.push(ClipOfferEntry { path: rel.clone(), is_dir: true, size: 0, mtime: mtime_secs(&md) });
            files.push(None);
            let mut list: Vec<_> = std::fs::read_dir(p).map_err(|e| other(format!("读不了 {}：{e}", p.display())))?.filter_map(|e| e.ok()).collect();
            list.sort_by_key(|e| e.file_name());
            for e in list {
                let name = e.file_name().to_string_lossy().to_string();
                if name.contains(['/', '\\']) {
                    continue;
                }
                walk(&e.path(), format!("{rel}/{name}"), entries, files)?;
                if entries.len() > CLIP_OFFER_MAX_ENTRIES {
                    return Err(other(format!("要复制的内容超过 {CLIP_OFFER_MAX_ENTRIES} 项，请用文件管理传输")));
                }
            }
        } else if md.is_file() {
            entries.push(ClipOfferEntry { path: rel, is_dir: false, size: md.len(), mtime: mtime_secs(&md) });
            files.push(Some(p.to_path_buf()));
        }
        Ok(())
    }
    let mut tops = std::collections::HashSet::new();
    for p in paths {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
        // 两个顶层项同名（来自不同目录）时第二个加编号，粘贴方不会互相覆盖
        let mut top = name.clone();
        let mut n = 2;
        while !tops.insert(top.clone()) {
            top = numbered(&name, n);
            n += 1;
        }
        walk(p, top, &mut entries, &mut files)?;
    }
    let offer = ClipOffer { id: rand::random::<u64>() | 1, entries };
    offer.validate().map_err(other)?;
    Ok(LocalOffer { offer, files })
}

/// 「报告.docx」→「报告 2.docx」（访达的同名规则）
pub fn numbered(name: &str, n: u32) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} {n}.{ext}"),
        _ => format!("{name} {n}"),
    }
}

impl PeerSession {
    /// 把本机复制的文件 / 文件夹作为清单放到被控端剪贴板（被控端能力列表要有 "clipboard_offer"）。
    /// 只发清单；对方粘贴时才来读内容。返回发出的清单。
    pub async fn clipboard_offer_files(&self, paths: &[PathBuf]) -> Result<ClipOffer> {
        let paths = paths.to_vec();
        let local = tokio::task::spawn_blocking(move || plan_local(&paths)).await.map_err(other)??;
        let offer = local.offer.clone();
        self.clip_offers.add(Arc::new(local));
        match self.mgmt(galaxy_proto::peer::MgmtReq::ClipboardOffer(offer.clone())).await? {
            galaxy_proto::peer::MgmtResp::Ok => Ok(offer),
            galaxy_proto::peer::MgmtResp::Error(e) => Err(crate::session::peer_err(e)),
            _ => Err(ClientError::Protocol("应答错误".into())),
        }
    }
}

/// 应答被控端开来的反向流（目前只有读剪贴板清单里的文件）。
pub(crate) async fn serve_reverse(sess: Arc<PeerSession>, mut send: quinn::SendStream, mut recv: quinn::RecvStream) -> anyhow::Result<()> {
    let Some(open) = tokio::time::timeout(Duration::from_secs(10), frame::read::<ReverseOpen>(&mut recv)).await?? else { return Ok(()) };
    match open {
        ReverseOpen::ClipboardRead { offer_id, index, offset } => {
            let Some(path) = sess.clip_offers.file(offer_id, index) else {
                frame::write(&mut send, &ClipReadReply::Error(PeerError::new("not_found", "复制的内容已经变了，请重新复制"))).await?;
                let _ = send.finish();
                return Ok(());
            };
            let opened = async {
                use tokio::io::AsyncSeekExt;
                let mut f = tokio::fs::File::open(&path).await?;
                let size = f.metadata().await?.len();
                f.seek(std::io::SeekFrom::Start(offset.min(size))).await?;
                std::io::Result::Ok((f, size))
            }
            .await;
            let (mut f, size) = match opened {
                Ok(x) => x,
                Err(e) => {
                    frame::write(&mut send, &ClipReadReply::Error(PeerError::new("io", format!("读不了 {}：{e}", path.display())))).await?;
                    let _ = send.finish();
                    return Ok(());
                }
            };
            frame::write(&mut send, &ClipReadReply::Ok { size }).await?;
            use tokio::io::AsyncReadExt;
            let mut buf = vec![0u8; CHUNK];
            loop {
                let n = f.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                // 对方 stop 了（取消粘贴）时这里报错，直接结束
                send.write_all(&buf[..n]).await?;
            }
            send.finish()?;
            // 等对方收完再丢掉流，免得最后一段被截断
            let _ = tokio::time::timeout(IDLE_TIMEOUT, send.stopped()).await;
        }
    }
    Ok(())
}

// ---------------- 远端复制 → 本机粘贴 ----------------

/// 远端剪贴板里文件的清单（`remote_offer` 展开），每项对应的远端路径。
pub struct RemoteOffer {
    pub offer: ClipOffer,
    remote: Vec<String>,
}

fn base_name(p: &str) -> String {
    let t = p.trim_end_matches(['/', '\\']);
    t.rsplit(['/', '\\']).next().unwrap_or(t).to_string()
}

/// 把远端剪贴板里的文件路径（`clipboard_get` 的 Files）展开成清单，不下载内容。
pub async fn remote_offer(session: &PeerSession, remotes: &[String]) -> Result<RemoteOffer> {
    if remotes.is_empty() {
        return Err(other("远端剪贴板里没有文件"));
    }
    let mut entries = vec![];
    let mut remote = vec![];
    let mut tops = std::collections::HashSet::new();
    for r in remotes {
        let name = base_name(r);
        let mut top = name.clone();
        let mut n = 2;
        while !tops.insert(top.clone()) {
            top = numbered(&name, n);
            n += 1;
        }
        // 深度优先，目录在内容前面
        let mut stack = vec![(r.clone(), top, None::<galaxy_proto::peer::FileEntry>)];
        while let Some((path, rel, known)) = stack.pop() {
            let st = match known {
                Some(e) => e,
                None => session.fs_stat(&path).await?,
            };
            match st.kind {
                FileKind::Dir => {
                    entries.push(ClipOfferEntry { path: rel.clone(), is_dir: true, size: 0, mtime: st.modified_at / 1000 });
                    remote.push(path.clone());
                    let mut list = session.fs_list(&path).await?;
                    list.retain(|e| matches!(e.kind, FileKind::Dir | FileKind::File) && !e.name.contains(['/', '\\']));
                    list.sort_by(|a, b| b.name.cmp(&a.name));
                    for e in list {
                        stack.push((e.path.clone(), format!("{rel}/{}", e.name), Some(e)));
                    }
                }
                FileKind::File => {
                    entries.push(ClipOfferEntry { path: rel, is_dir: false, size: st.size, mtime: st.modified_at / 1000 });
                    remote.push(path);
                }
                _ => {}
            }
            if entries.len() > CLIP_OFFER_MAX_ENTRIES {
                return Err(other(format!("远端复制的内容超过 {CLIP_OFFER_MAX_ENTRIES} 项，请用文件管理传输")));
            }
        }
    }
    let offer = ClipOffer { id: rand::random::<u64>() | 1, entries };
    offer.validate().map_err(other)?;
    Ok(RemoteOffer { offer, remote })
}

impl RemoteOffer {
    /// 打开第 `index` 项（文件）从 `offset` 读到末尾；在普通线程里调用，会等到对方应答。返回（文件现在的大小，读取器）。
    pub fn open(&self, rt: &tokio::runtime::Handle, session: Arc<PeerSession>, index: u32, offset: u64) -> std::result::Result<(u64, BlockingReader), String> {
        let e = self.offer.entries.get(index as usize).ok_or("没有这一项")?;
        if e.is_dir {
            return Err("这一项是文件夹".into());
        }
        let path = self.remote[index as usize].clone();
        let (tx, reader) = BlockingReader::channel();
        let (size_tx, size_rx) = tokio::sync::oneshot::channel::<u64>();
        let opened = wait_open(rt, async move {
            let (mut s, mut r) = session.open_stream(StreamOpen::Files(FileOp::Read { path, offset, len: None })).await.map_err(|e| e.to_string())?;
            let _ = s.finish();
            // 第一帧是大小；之后的数据交给后台任务
            let first: Option<FileMsg> = frame::read(&mut r).await.map_err(|e| e.to_string())?;
            let size = match first {
                Some(FileMsg::Size(n)) => n,
                Some(FileMsg::Error(e)) => return Err(e.message),
                Some(FileMsg::End) | None => {
                    tx.end().await;
                    return Ok(offset);
                }
                Some(_) => return Err("应答错误".into()),
            };
            let _ = size_tx.send(size);
            tokio::spawn(async move {
                loop {
                    let m: Option<FileMsg> = match tokio::time::timeout(IDLE_TIMEOUT, frame::read(&mut r)).await {
                        Ok(Ok(m)) => m,
                        Ok(Err(e)) => return tx.fail(e.to_string()).await,
                        Err(_) => return tx.fail("传输超时").await,
                    };
                    match m {
                        Some(FileMsg::Chunk(d)) => {
                            if !tx.data(Bytes::from(d)).await {
                                // 粘贴方不要了：关掉流
                                let _ = r.stop(0u32.into());
                                return;
                            }
                        }
                        Some(FileMsg::End) | None => return tx.end().await,
                        Some(FileMsg::Error(e)) => return tx.fail(e.message).await,
                        Some(_) => {}
                    }
                }
            });
            Ok(size)
        });
        drop(size_rx);
        opened.map(|size| (size, reader))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_local_orders_dirs_first_and_renames_duplicates() {
        let d = std::env::temp_dir().join(format!("lg-clipoffer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("x/a/b")).unwrap();
        std::fs::create_dir_all(d.join("y")).unwrap();
        std::fs::write(d.join("x/a/1.txt"), b"hello").unwrap();
        std::fs::write(d.join("x/a/b/2.txt"), b"world!").unwrap();
        std::fs::write(d.join("x/top.txt"), b"x").unwrap();
        std::fs::write(d.join("y/top.txt"), b"yy").unwrap();
        let p = plan_local(&[d.join("x/a"), d.join("x/top.txt"), d.join("y/top.txt")]).unwrap();
        let rels: Vec<_> = p.offer.entries.iter().map(|e| (e.path.as_str(), e.is_dir, e.size)).collect();
        assert_eq!(rels, vec![("a", true, 0), ("a/1.txt", false, 5), ("a/b", true, 0), ("a/b/2.txt", false, 6), ("top.txt", false, 1), ("top 2.txt", false, 2)]);
        assert_eq!(p.offer.total_bytes(), 14);
        assert_eq!(p.files[1].as_deref(), Some(d.join("x/a/1.txt").as_path()));
        assert!(p.files[0].is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn numbered_like_finder() {
        assert_eq!(numbered("报告.docx", 2), "报告 2.docx");
        assert_eq!(numbered("dir", 3), "dir 3");
        assert_eq!(numbered(".bashrc", 2), ".bashrc 2");
    }

    #[test]
    fn blocking_reader_reads_across_chunks_and_reports_errors() {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        let (tx, mut r) = BlockingReader::channel();
        rt.spawn(async move {
            tx.data(Bytes::from_static(b"abc")).await;
            tx.data(Bytes::from_static(b"defg")).await;
            tx.end().await;
        });
        let mut s = String::new();
        r.read_to_string(&mut s).unwrap();
        assert_eq!(s, "abcdefg");
        // 任务没说结束就没了：报错，不当成正常读完
        let (tx, mut r) = BlockingReader::channel();
        rt.spawn(async move {
            tx.data(Bytes::from_static(b"ab")).await;
            drop(tx);
        });
        let mut v = vec![];
        assert!(r.read_to_end(&mut v).is_err());
    }
}
