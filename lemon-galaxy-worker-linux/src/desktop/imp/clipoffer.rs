//! 复制粘贴文件，粘贴时才传（docs/protocol.md 8.4.1），Linux 这一侧。
//!
//! 服务（root）拿着内核给的 [`ClipSource`]（经反向流向主控端读），把清单交给画面代理（会话用户身份）：
//! - 代理用 FUSE 把清单挂成只读目录（fuse.rs），剪贴板放指向它的 text/uri-list 与 x-special/gnome-copied-files（copy）。
//!   文件管理器粘贴时才来读，显示它自己的复制进度。
//! - 没有 FUSE（没有 /dev/fuse 或 fusermount3、挂载失败）：剪贴板放「粘贴时才准备」的文件列表，有人来取时
//!   先下载到 `~/.lemon-galaxy/clipboard/` 下的暂存目录（自己发进度通知，可取消），好了再给路径。
//! - 读文件内容走代理与服务之间的管道：代理 `ClipOpen` / `ClipRead`（一次一段、一次只有一个在途）/ `ClipClose`，
//!   服务在单独的线程里调 `ClipSource::open` 和读，读到的一段经 `ClipData` 回去。管道里不会堆大块数据，不挤画面和键鼠。
//! - `ClipboardGet` 不返回指向虚拟目录或暂存目录的路径（不然又被当成远端复制了文件传回去）。

use super::clipfmt::LazyFiles;
use super::fuse;
use super::ipc::{FromAgent, ToAgent};
use galaxy_host::clipoffer::ClipSource;
use galaxy_proto::peer::{ClipOffer, ClipboardData};
use parking_lot::{Condvar, Mutex};
use std::collections::{HashMap, VecDeque};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 一次读多少（管道里一条消息的最大数据量）
const CHUNK: u32 = 256 * 1024;
const OPEN_TIMEOUT: Duration = Duration::from_secs(90);
const READ_TIMEOUT: Duration = Duration::from_secs(120);

// ======================== 服务这一侧 ========================

/// 最近几份清单的来源（主控端也只认本会话最近 4 份）
#[derive(Default)]
pub struct Offers(Mutex<VecDeque<(u64, Arc<dyn ClipSource>)>>);

impl Offers {
    pub fn add(&self, id: u64, source: Arc<dyn ClipSource>) {
        let mut g = self.0.lock();
        g.retain(|(i, _)| *i != id);
        g.push_back((id, source));
        while g.len() > fuse::KEEP_OFFERS {
            g.pop_front();
        }
    }

    fn get(&self, id: u64) -> Option<Arc<dyn ClipSource>> {
        self.0.lock().iter().find(|(i, _)| *i == id).map(|(_, s)| s.clone())
    }
}

/// 一个画面代理的读取流（在读代理消息的线程里用，只做不阻塞的事）。代理退出、这个对象丢掉时所有流随之结束。
pub struct Streams {
    out: mpsc::Sender<ToAgent>,
    map: HashMap<u64, mpsc::Sender<u32>>,
}

impl Streams {
    pub fn new(out: mpsc::Sender<ToAgent>) -> Self {
        Self { out, map: HashMap::new() }
    }

    /// 处理读文件的消息；别的消息原样还回去
    pub fn handle(&mut self, offers: &Offers, m: FromAgent) -> Option<FromAgent> {
        match m {
            FromAgent::ClipOpen { handle, offer, index, offset } => {
                let (tx, rx) = mpsc::channel();
                self.map.insert(handle, tx);
                let (src, out) = (offers.get(offer), self.out.clone());
                let spawned = std::thread::Builder::new().name("clip-stream".into()).spawn(move || serve_stream(handle, src, index, offset, rx, out));
                if spawned.is_err() {
                    self.map.remove(&handle);
                    let _ = self.out.send(ToAgent::ClipOpened { handle, result: Err("系统资源不足".into()) });
                }
                None
            }
            FromAgent::ClipRead { handle, len } => {
                let alive = self.map.get(&handle).map(|t| t.send(len).is_ok()).unwrap_or(false);
                if !alive {
                    self.map.remove(&handle);
                    let _ = self.out.send(ToAgent::ClipData { handle, result: Err("读取已结束".into()) });
                }
                None
            }
            FromAgent::ClipClose { handle } => {
                // 丢掉发送端：读线程收不到下一个请求就退出，读取器随之丢掉（主控端那条流停下）
                self.map.remove(&handle);
                None
            }
            m => Some(m),
        }
    }
}

fn serve_stream(handle: u64, src: Option<Arc<dyn ClipSource>>, index: u32, offset: u64, rx: mpsc::Receiver<u32>, out: mpsc::Sender<ToAgent>) {
    let Some(src) = src else {
        let _ = out.send(ToAgent::ClipOpened { handle, result: Err("这份复制的内容已经过期，请在主控端重新复制".into()) });
        return;
    };
    let mut r = match src.open(index, offset) {
        Ok((size, r)) => {
            if out.send(ToAgent::ClipOpened { handle, result: Ok(size) }).is_err() {
                return;
            }
            r
        }
        Err(e) => {
            let _ = out.send(ToAgent::ClipOpened { handle, result: Err(e) });
            return;
        }
    };
    let mut buf = vec![0u8; CHUNK as usize];
    while let Ok(len) = rx.recv() {
        let n = (len as usize).clamp(1, buf.len());
        let res = loop {
            match r.read(&mut buf[..n]) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                x => break x,
            }
        };
        let (msg, end) = match res {
            Ok(k) => (ToAgent::ClipData { handle, result: Ok(buf[..k].to_vec()) }, k == 0),
            Err(e) => (ToAgent::ClipData { handle, result: Err(e.to_string()) }, true),
        };
        if out.send(msg).is_err() || end {
            return;
        }
    }
}

// ======================== 画面代理这一侧 ========================

enum Reply {
    Opened(Result<u64, String>),
    Data(Result<Vec<u8>, String>),
}

/// 经管道向服务读清单里的文件
pub struct Client {
    out: mpsc::SyncSender<FromAgent>,
    next: AtomicU64,
    waiting: Mutex<HashMap<u64, mpsc::Sender<Reply>>>,
}

impl Client {
    pub fn new(out: mpsc::SyncSender<FromAgent>) -> Arc<Self> {
        Arc::new(Self { out, next: AtomicU64::new(1), waiting: Mutex::new(HashMap::new()) })
    }

    /// 服务发来的读取结果（在读标准输入的线程里调，不阻塞）
    pub fn deliver(&self, m: ToAgent) {
        let (h, r) = match m {
            ToAgent::ClipOpened { handle, result } => (handle, Reply::Opened(result)),
            ToAgent::ClipData { handle, result } => (handle, Reply::Data(result)),
            _ => return,
        };
        if let Some(t) = self.waiting.lock().get(&h) {
            let _ = t.send(r);
        }
    }

    fn open(self: &Arc<Self>, offer: u64, index: u32, offset: u64) -> Result<(u64, IpcReader), String> {
        let handle = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.waiting.lock().insert(handle, tx);
        // 先建好读取器：下面出错返回时它负责收尾（ClipClose）
        let reader = IpcReader { client: self.clone(), handle, rx, buf: vec![], at: 0, eof: false };
        self.out.send(FromAgent::ClipOpen { handle, offer, index, offset }).map_err(|_| "画面代理正在退出".to_string())?;
        match reader.rx.recv_timeout(OPEN_TIMEOUT) {
            Ok(Reply::Opened(Ok(size))) => Ok((size, reader)),
            Ok(Reply::Opened(Err(e))) => Err(e),
            Ok(Reply::Data(_)) => Err("应答不对".into()),
            Err(_) => Err("等主控端应答超时".into()),
        }
    }
}

pub struct IpcReader {
    client: Arc<Client>,
    handle: u64,
    rx: mpsc::Receiver<Reply>,
    buf: Vec<u8>,
    at: usize,
    eof: bool,
}

impl Read for IpcReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        if self.at >= self.buf.len() {
            if self.eof {
                return Ok(0);
            }
            self.client.out.send(FromAgent::ClipRead { handle: self.handle, len: CHUNK }).map_err(|_| io::Error::other("画面代理正在退出"))?;
            match self.rx.recv_timeout(READ_TIMEOUT) {
                Ok(Reply::Data(Ok(d))) => {
                    if d.is_empty() {
                        self.eof = true;
                        return Ok(0);
                    }
                    self.buf = d;
                    self.at = 0;
                }
                Ok(Reply::Data(Err(e))) => return Err(io::Error::other(e)),
                Ok(Reply::Opened(_)) => return Err(io::Error::other("应答不对")),
                Err(_) => return Err(io::Error::other("传输超时")),
            }
        }
        let n = out.len().min(self.buf.len() - self.at);
        out[..n].copy_from_slice(&self.buf[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

impl Drop for IpcReader {
    fn drop(&mut self) {
        self.client.waiting.lock().remove(&self.handle);
        let _ = self.client.out.send(FromAgent::ClipClose { handle: self.handle });
    }
}

/// 代理里看到的清单来源：经管道向服务读
struct IpcSource {
    client: Arc<Client>,
    offer: u64,
}

impl ClipSource for IpcSource {
    fn open(&self, index: u32, offset: u64) -> Result<(u64, Box<dyn Read + Send>), String> {
        self.client.open(self.offer, index, offset).map(|(s, r)| (s, Box::new(r) as Box<dyn Read + Send>))
    }
}

/// 剪贴板里放什么
pub enum FileClip {
    /// 虚拟目录里的路径（有 FUSE）
    Paths(Vec<String>),
    /// 粘贴时才下载（没有 FUSE）
    Lazy(Arc<dyn LazyFiles>),
}

/// 画面代理里的复制粘贴文件：管虚拟目录（或兜底的暂存目录）与回声过滤
pub struct Agent {
    client: Arc<Client>,
    /// None：还没试过；Some(Err)：FUSE 不能用的原因（走兜底）
    fuse: Mutex<Option<Result<Arc<fuse::Mount>, String>>>,
    /// 兜底下载用过的暂存目录（ClipboardGet 不返回它们里面的路径）
    staged: Arc<Mutex<Vec<PathBuf>>>,
}

/// 虚拟目录挂在哪：$XDG_RUNTIME_DIR/lemon-galaxy/clip（没有时 /run/user/<uid>，再没有放 ~/.cache 下）
fn mount_dir() -> PathBuf {
    if let Some(r) = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(r).join("lemon-galaxy").join("clip");
    }
    let run = PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() }));
    if run.is_dir() {
        return run.join("lemon-galaxy").join("clip");
    }
    home().join(".cache").join("lemon-galaxy").join("clip")
}

fn home() -> PathBuf {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// 兜底下载的暂存区（与主控端老做法的上传暂存区同一个地方，一天前的内容两边都会顺手清掉）
fn staging_root() -> PathBuf {
    home().join(".lemon-galaxy").join("clipboard")
}

/// 路径在不在 `dirs` 里的某个目录下（或就是它）
fn under(path: &str, dirs: &[PathBuf]) -> bool {
    let p = Path::new(path);
    dirs.iter().any(|d| p.starts_with(d))
}

/// 去掉指向我们自己的虚拟目录 / 暂存目录的路径；都去掉了就当剪贴板里没有文件
fn filter_own(d: Option<ClipboardData>, own: &[PathBuf]) -> Option<ClipboardData> {
    match d {
        Some(ClipboardData::Files(v)) => {
            let rest: Vec<String> = v.into_iter().filter(|p| !under(p, own)).collect();
            (!rest.is_empty()).then_some(ClipboardData::Files(rest))
        }
        other => other,
    }
}

impl Agent {
    pub fn new(out: mpsc::SyncSender<FromAgent>) -> Arc<Self> {
        Arc::new(Self { client: Client::new(out), fuse: Mutex::new(None), staged: Default::default() })
    }

    pub fn deliver(&self, m: ToAgent) {
        self.client.deliver(m)
    }

    fn mount(&self) -> Result<Arc<fuse::Mount>, String> {
        let mut g = self.fuse.lock();
        if let Some(r) = g.as_ref() {
            return r.clone();
        }
        let r = if std::env::var("LEMON_GALAXY_CLIP_FUSE").ok().as_deref() == Some("0") { Err("已用 LEMON_GALAXY_CLIP_FUSE=0 关掉".to_string()) } else { fuse::Mount::start(&mount_dir()) };
        match &r {
            Ok(m) => eprintln!("复制粘贴文件：虚拟目录已挂在 {}", m.dir.display()),
            Err(e) => eprintln!("复制粘贴文件：FUSE 不可用（{e}），改为有人粘贴时先下载到 {} 再粘贴", staging_root().display()),
        }
        *g = Some(r.clone());
        r
    }

    /// 准备一份清单：有 FUSE 给虚拟目录里的路径，没有给粘贴时才下载的文件列表
    pub fn prepare(&self, offer: ClipOffer) -> FileClip {
        let source: Arc<dyn ClipSource> = Arc::new(IpcSource { client: self.client.clone(), offer: offer.id });
        match self.mount() {
            Ok(m) => FileClip::Paths(m.add(offer, source).into_iter().map(|p| p.to_string_lossy().to_string()).collect()),
            Err(_) => FileClip::Lazy(Download::new(offer, source, staging_root(), self.staged.clone())),
        }
    }

    /// ClipboardGet 的结果去掉回声
    pub fn filter(&self, d: Option<ClipboardData>) -> Option<ClipboardData> {
        let mut own = self.staged.lock().clone();
        if let Some(Ok(m)) = self.fuse.lock().as_ref() {
            own.push(m.dir.clone());
        }
        own.push(mount_dir());
        filter_own(d, &own)
    }

    /// 代理退出前：卸掉虚拟目录
    pub fn shutdown(&self) {
        if let Some(Ok(m)) = self.fuse.lock().as_ref() {
            m.unmount();
        }
    }
}

// ---------------- 没有 FUSE 时的兜底：粘贴时先下载 ----------------

enum DlState {
    Idle,
    Running,
    Done(Vec<String>),
}

struct Download {
    offer: ClipOffer,
    source: Arc<dyn ClipSource>,
    root: PathBuf,
    staged: Arc<Mutex<Vec<PathBuf>>>,
    st: Mutex<DlState>,
    cv: Condvar,
}

impl Download {
    fn new(offer: ClipOffer, source: Arc<dyn ClipSource>, root: PathBuf, staged: Arc<Mutex<Vec<PathBuf>>>) -> Arc<Self> {
        Arc::new(Self { offer, source, root, staged, st: Mutex::new(DlState::Idle), cv: Condvar::new() })
    }

    fn run(&self) -> Result<Vec<String>, String> {
        clean_old(&self.root);
        let dir = self.root.join(format!("{}-{:x}", galaxy_proto::now_ms(), self.offer.id));
        std::fs::create_dir_all(&dir).map_err(|e| format!("建不了暂存目录 {}：{e}", dir.display()))?;
        self.staged.lock().push(dir.clone());
        let n = self.offer.top_level().len();
        let total = self.offer.total_bytes();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut note = Notifier::new(cancel.clone());
        let zh = super::zh();
        let title = if zh { "正在接收复制的文件" } else { "Receiving copied files" };
        note.show(title, &progress_text(n, 0, total, zh), Some(0));
        let mut last = Instant::now();
        let r = galaxy_host::clipoffer::paste_to_dir(
            &self.offer,
            &*self.source,
            &dir,
            |done, total| {
                if last.elapsed() >= Duration::from_secs(2) {
                    last = Instant::now();
                    let pct = (done.saturating_mul(100) / total.max(1)) as i32;
                    note.show(title, &progress_text(n, done, total, zh), Some(pct));
                }
            },
            &cancel,
        );
        match r {
            Ok(tops) => {
                let body = if zh { format!("已接收 {n} 个项目，正在粘贴。如果没有粘贴出来，请再粘贴一次。") } else { format!("Received {n} item(s). If nothing was pasted, paste again.") };
                note.show(if zh { "文件已接收" } else { "Files received" }, &body, None);
                Ok(tops.into_iter().map(|p| p.to_string_lossy().to_string()).collect())
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                note.show(if zh { "接收复制的文件失败" } else { "Could not receive copied files" }, &e, None);
                Err(e)
            }
        }
    }
}

impl LazyFiles for Download {
    /// 第一次有人要时开始下载（在调用的线程里），同时来要的等着同一次下载；失败了下次再要重新下
    fn paths(&self) -> Option<Vec<String>> {
        {
            let mut g = self.st.lock();
            loop {
                match &*g {
                    DlState::Done(p) => return Some(p.clone()),
                    DlState::Running => {
                        self.cv.wait(&mut g);
                        // 等的那次失败了：这次粘贴算失败，不再另起一次
                        if matches!(*g, DlState::Idle) {
                            return None;
                        }
                    }
                    DlState::Idle => {
                        *g = DlState::Running;
                        break;
                    }
                }
            }
        }
        let r = self.run();
        let mut g = self.st.lock();
        let out = match r {
            Ok(p) => {
                *g = DlState::Done(p.clone());
                Some(p)
            }
            Err(e) => {
                eprintln!("粘贴时下载复制的文件失败：{e}");
                *g = DlState::Idle;
                None
            }
        };
        self.cv.notify_all();
        out
    }
}

/// 清掉暂存区里一天前的内容
fn clean_old(root: &Path) {
    let Ok(rd) = std::fs::read_dir(root) else { return };
    for e in rd.flatten() {
        let old = e.metadata().and_then(|m| m.modified()).map(|t| t.elapsed().map(|d| d > Duration::from_secs(24 * 3600)).unwrap_or(false)).unwrap_or(false);
        if old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

fn human(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

fn progress_text(n: usize, done: u64, total: u64, zh: bool) -> String {
    let pct = done.saturating_mul(100) / total.max(1);
    if zh {
        format!("{n} 个项目，{} / {}（{pct}%）。接收完才会粘贴。", human(done), human(total))
    } else {
        format!("{n} item(s), {} / {} ({pct}%). Pasting starts when done.", human(done), human(total))
    }
}

/// 桌面通知（org.freedesktop.Notifications），同一条反复更新；带「取消」按钮。会话总线不可用时什么也不做。
struct Notifier {
    conn: Option<zbus::blocking::Connection>,
    id: u32,
    cancel: Arc<AtomicBool>,
    listening: bool,
}

const N_BUS: &str = "org.freedesktop.Notifications";
const N_PATH: &str = "/org/freedesktop/Notifications";

impl Notifier {
    fn new(cancel: Arc<AtomicBool>) -> Self {
        Self { conn: zbus::blocking::Connection::session().ok(), id: 0, cancel, listening: false }
    }

    /// `progress` 为 Some 时是进行中（带取消按钮、进度提示），None 是最后一条
    fn show(&mut self, summary: &str, body: &str, progress: Option<i32>) {
        let Some(conn) = self.conn.clone() else { return };
        let zh = super::zh();
        let actions: Vec<&str> = if progress.is_some() { vec!["cancel", if zh { "取消" } else { "Cancel" }] } else { vec![] };
        let mut hints: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
        hints.insert("urgency", zbus::zvariant::Value::U8(1));
        if let Some(p) = progress {
            hints.insert("value", zbus::zvariant::Value::I32(p.clamp(0, 100)));
        }
        let r = conn.call_method(Some(N_BUS), N_PATH, Some(N_BUS), "Notify", &("LemonGalaxy", self.id, "folder-download", summary, body, actions, hints, if progress.is_some() { 0i32 } else { -1i32 }));
        if let Ok(id) = r.and_then(|m| m.body().deserialize::<u32>()) {
            self.id = id;
        }
        if progress.is_some() && !self.listening && self.id != 0 {
            self.listening = true;
            let (cancel, id) = (self.cancel.clone(), self.id);
            std::thread::spawn(move || {
                let Ok(rule) = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(N_BUS).and_then(|b| b.member("ActionInvoked")).map(|b| b.build()) else { return };
                let Ok(it) = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, Some(8)) else { return };
                for m in it.flatten() {
                    if let Ok((nid, key)) = m.body().deserialize::<(u32, String)>() {
                        if nid == id && key == "cancel" {
                            cancel.store(true, Ordering::Relaxed);
                            return;
                        }
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use galaxy_proto::peer::ClipOfferEntry;

    struct Mem(Vec<Vec<u8>>);
    impl ClipSource for Mem {
        fn open(&self, index: u32, offset: u64) -> Result<(u64, Box<dyn Read + Send>), String> {
            let d = self.0.get(index as usize).ok_or("没有这一项")?.clone();
            Ok((d.len() as u64, Box::new(io::Cursor::new(d[offset as usize..].to_vec()))))
        }
    }

    /// 代理与服务在同一个进程里用通道接起来（代替标准输入输出）
    fn wire(offers: Arc<Offers>) -> Arc<Client> {
        let (out_tx, out_rx) = mpsc::sync_channel::<FromAgent>(16);
        let (to_tx, to_rx) = mpsc::channel::<ToAgent>();
        let client = Client::new(out_tx);
        std::thread::spawn(move || {
            let mut s = Streams::new(to_tx);
            while let Ok(m) = out_rx.recv() {
                assert!(s.handle(&offers, m).is_none());
            }
        });
        let c = client.clone();
        std::thread::spawn(move || {
            while let Ok(m) = to_rx.recv() {
                c.deliver(m);
            }
        });
        client
    }

    #[test]
    fn reads_through_pipe() {
        let big: Vec<u8> = (0..(1u32 << 20) + 12345).map(|i| (i % 253) as u8).collect();
        let offers = Arc::new(Offers::default());
        offers.add(7, Arc::new(Mem(vec![vec![], big.clone()])));
        let client = wire(offers.clone());
        let src = IpcSource { client: client.clone(), offer: 7 };
        let (size, mut r) = src.open(1, 0).unwrap();
        assert_eq!(size, big.len() as u64);
        let mut got = vec![];
        r.read_to_end(&mut got).unwrap();
        assert_eq!(got, big);
        drop(r);
        // 从中间读
        let (_, mut r) = src.open(1, 1000).unwrap();
        let mut b = [0u8; 10];
        r.read_exact(&mut b).unwrap();
        assert_eq!(b[..], big[1000..1010]);
        drop(r);
        // 过期的清单、读不了的项
        assert!(IpcSource { client: client.clone(), offer: 99 }.open(0, 0).err().unwrap().contains("过期"));
        assert!(src.open(5, 0).is_err());
        // 读取器都丢掉后不留等待项
        std::thread::sleep(Duration::from_millis(50));
        assert!(client.waiting.lock().is_empty());
    }

    #[test]
    fn offers_keep_last_four() {
        let o = Offers::default();
        for i in 0..6 {
            o.add(i, Arc::new(Mem(vec![])));
        }
        assert!(o.get(1).is_none());
        assert!(o.get(2).is_some() && o.get(5).is_some());
    }

    #[test]
    fn echo_filter() {
        let own = vec![PathBuf::from("/run/user/1000/lemon-galaxy/clip"), PathBuf::from("/home/a/.lemon-galaxy/clipboard/1-7")];
        let f = |v: &[&str]| filter_own(Some(ClipboardData::Files(v.iter().map(|s| s.to_string()).collect())), &own);
        assert!(f(&["/run/user/1000/lemon-galaxy/clip/7/a.txt"]).is_none());
        assert!(f(&["/home/a/.lemon-galaxy/clipboard/1-7/资料"]).is_none());
        // 名字只是前缀相同的不算
        assert!(matches!(f(&["/run/user/1000/lemon-galaxy/clipx/a"]), Some(ClipboardData::Files(v)) if v.len() == 1));
        assert!(matches!(f(&["/home/a/x.txt", "/run/user/1000/lemon-galaxy/clip/7/a"]), Some(ClipboardData::Files(v)) if v == vec!["/home/a/x.txt".to_string()]));
        assert!(matches!(filter_own(Some(ClipboardData::Text("x".into())), &own), Some(ClipboardData::Text(_))));
    }

    #[test]
    fn fallback_downloads_once_on_paste() {
        let root = std::env::temp_dir().join(format!("lg-clip-dl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let offer = ClipOffer { id: 0x5a, entries: vec![ClipOfferEntry { path: "d".into(), is_dir: true, size: 0, mtime: 0 }, ClipOfferEntry { path: "d/x.bin".into(), is_dir: false, size: 3, mtime: 0 }] };
        let staged = Arc::new(Mutex::new(vec![]));
        let dl = Download::new(offer, Arc::new(Mem(vec![vec![], b"abc".to_vec()])), root.clone(), staged.clone());
        // 两个请求同时来：只下载一次，拿到同一组路径
        let d2 = dl.clone();
        let t = std::thread::spawn(move || d2.paths());
        let p1 = dl.paths().unwrap();
        let p2 = t.join().unwrap().unwrap();
        assert_eq!(p1, p2);
        assert_eq!(p1.len(), 1);
        assert!(p1[0].ends_with("-5a/d"));
        assert_eq!(std::fs::read(Path::new(&p1[0]).join("x.bin")).unwrap(), b"abc");
        assert_eq!(staged.lock().len(), 1);
        assert!(under(&p1[0], &staged.lock()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn sizes() {
        assert_eq!(human(512), "512 B");
        assert_eq!(human(3 << 20), "3.0 MB");
        assert!(progress_text(2, 1 << 20, 4 << 20, true).contains("25%"));
    }
}
