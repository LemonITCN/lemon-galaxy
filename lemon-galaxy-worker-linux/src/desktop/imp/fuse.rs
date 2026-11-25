//! 复制粘贴文件的虚拟目录（docs/protocol.md 8.4.1）：主控端复制的文件清单挂成只读目录树
//! `$XDG_RUNTIME_DIR/lemon-galaxy/clip/<清单 id 十六进制>/…`，剪贴板里放指向它的路径。
//! 文件管理器粘贴（复制）时才来读，读到哪传到哪，进度条就是文件管理器自己的「正在复制」。
//!
//! 只实现 FUSE 内核协议里只读要用的几条（不链接 libfuse，不加依赖）。挂载经 fusermount3 / fusermount
//! （setuid 程序，普通用户也能挂）：socketpair 一端经 `_FUSE_COMMFD` 交给它，挂好后它用 SCM_RIGHTS 把
//! /dev/fuse 的描述符传回来。带 `auto_unmount`：它留一个进程守着这条连接，我们退出（包括被 kill）时它负责卸载；
//! 不支持时下次挂载前先把残留的挂载点卸掉。
//!
//! 读：每个文件一个读线程，按顺序处理内核发来的读请求；对上游（主控端）保持一条从某个位置往后读的流，
//! 缓存最近读过的一段（内核预读偶尔乱序），往后跳一点就读过去，跳得远或往回跳才重新打开。

use galaxy_host::clipoffer::ClipSource;
use galaxy_proto::peer::ClipOffer;
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const ROOT: u64 = 1;
/// 同时保留几份清单（主控端也只认本会话最近 4 份）
pub const KEEP_OFFERS: usize = 4;
/// 读缓存：保留最近读过的这么多字节
const WINDOW: usize = 4 << 20;
/// 往后跳这么远以内直接读过去，再远就重新打开
const SKIP: u64 = 2 << 20;
/// 文件关掉后上游流再留一会（马上又打开同一个文件时接着用）
const LINGER: Duration = Duration::from_secs(10);
const CHUNK: usize = 256 * 1024;

// ---------------- 目录树 ----------------

/// 一份清单：目录 inode 是 `ino`，第 i 项的 inode 是 `ino + 1 + i`
pub struct OfferNode {
    pub id: u64,
    pub name: String,
    ino: u64,
    offer: ClipOffer,
    source: Arc<dyn ClipSource>,
    /// (父目录 inode, 名字) → 清单下标
    names: HashMap<(u64, String), u32>,
    /// 目录 inode → 子项下标（按清单顺序）
    children: HashMap<u64, Vec<u32>>,
    /// 目录 inode → 子目录个数（nlink 用）
    subdirs: HashMap<u64, u32>,
    /// 各项的父目录 inode
    parents: Vec<u64>,
    time: i64,
}

impl OfferNode {
    fn new(ino: u64, offer: ClipOffer, source: Arc<dyn ClipSource>, time: i64) -> Self {
        let mut names = HashMap::new();
        let mut children: HashMap<u64, Vec<u32>> = HashMap::new();
        let mut subdirs: HashMap<u64, u32> = HashMap::new();
        let mut parents = Vec::with_capacity(offer.entries.len());
        {
            let mut idx: HashMap<&str, u32> = HashMap::new();
            for (i, e) in offer.entries.iter().enumerate() {
                let i = i as u32;
                // 清单已校验过：父文件夹在前。万一不在，挂到清单根下
                let (parent, name) = match e.path.rsplit_once('/') {
                    Some((p, n)) => (idx.get(p).map(|j| ino + 1 + *j as u64).unwrap_or(ino), n),
                    None => (ino, e.path.as_str()),
                };
                names.insert((parent, name.to_string()), i);
                parents.push(parent);
                children.entry(parent).or_default().push(i);
                if e.is_dir {
                    *subdirs.entry(parent).or_default() += 1;
                    idx.insert(e.path.as_str(), i);
                }
            }
        }
        let name = format!("{:x}", offer.id);
        Self { id: offer.id, name, ino, offer, source, names, children, subdirs, parents, time }
    }

    fn entry_ino(&self, i: u32) -> u64 {
        self.ino + 1 + i as u64
    }

    /// 顶层项（复制的那几项）的名字
    pub fn top_names(&self) -> Vec<String> {
        self.offer.top_level().into_iter().map(|i| self.offer.entries[i].path.clone()).collect()
    }
}

#[derive(Clone)]
enum Node {
    Root,
    Offer(Arc<OfferNode>),
    Entry(Arc<OfferNode>, u32),
}

#[derive(Debug, PartialEq)]
struct Attr {
    ino: u64,
    size: u64,
    dir: bool,
    nlink: u32,
    mtime: i64,
}

pub struct Tree {
    offers: Vec<Arc<OfferNode>>,
    next_ino: u64,
    start: i64,
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl Tree {
    pub fn new() -> Self {
        Self { offers: vec![], next_ino: ROOT + 1, start: now_secs() }
    }

    /// 加一份清单（同一个 id 的旧的换掉），只留最近几份
    pub fn add(&mut self, offer: ClipOffer, source: Arc<dyn ClipSource>) -> Arc<OfferNode> {
        self.offers.retain(|o| o.id != offer.id);
        let ino = self.next_ino;
        self.next_ino += 1 + offer.entries.len() as u64;
        let n = Arc::new(OfferNode::new(ino, offer, source, now_secs()));
        self.offers.push(n.clone());
        while self.offers.len() > KEEP_OFFERS {
            self.offers.remove(0);
        }
        n
    }

    fn node(&self, ino: u64) -> Option<Node> {
        if ino == ROOT {
            return Some(Node::Root);
        }
        for o in &self.offers {
            if ino == o.ino {
                return Some(Node::Offer(o.clone()));
            }
            if ino > o.ino && ino <= o.ino + o.offer.entries.len() as u64 {
                return Some(Node::Entry(o.clone(), (ino - o.ino - 1) as u32));
            }
        }
        None
    }

    fn lookup(&self, parent: u64, name: &str) -> Option<Node> {
        if parent == ROOT {
            return self.offers.iter().find(|o| o.name == name).map(|o| Node::Offer(o.clone()));
        }
        let o = self.offers.iter().find(|o| parent >= o.ino && parent <= o.ino + o.offer.entries.len() as u64)?;
        let i = *o.names.get(&(parent, name.to_string()))?;
        Some(Node::Entry(o.clone(), i))
    }

    fn attr(&self, n: &Node) -> Attr {
        match n {
            Node::Root => Attr { ino: ROOT, size: 0, dir: true, nlink: 2 + self.offers.len() as u32, mtime: self.start },
            Node::Offer(o) => Attr { ino: o.ino, size: 0, dir: true, nlink: 2 + o.subdirs.get(&o.ino).copied().unwrap_or(0), mtime: o.time },
            Node::Entry(o, i) => {
                let e = &o.offer.entries[*i as usize];
                let ino = o.entry_ino(*i);
                let nlink = if e.is_dir { 2 + o.subdirs.get(&ino).copied().unwrap_or(0) } else { 1 };
                Attr { ino, size: if e.is_dir { 0 } else { e.size }, dir: e.is_dir, nlink, mtime: if e.mtime > 0 { e.mtime } else { o.time } }
            }
        }
    }

    /// 目录里第 `k` 项（0 是「.」，1 是「..」）：(inode, 名字, 是不是目录)
    fn dir_item(&self, n: &Node, k: usize) -> Option<(u64, String, bool)> {
        let (me, parent) = match n {
            Node::Root => (ROOT, ROOT),
            Node::Offer(o) => (o.ino, ROOT),
            Node::Entry(o, i) => (o.entry_ino(*i), o.parents[*i as usize]),
        };
        match k {
            0 => return Some((me, ".".into(), true)),
            1 => return Some((parent, "..".into(), true)),
            _ => {}
        }
        let k = k - 2;
        match n {
            Node::Root => self.offers.get(k).map(|o| (o.ino, o.name.clone(), true)),
            Node::Offer(o) | Node::Entry(o, _) => {
                let i = *o.children.get(&me)?.get(k)?;
                let e = &o.offer.entries[i as usize];
                let name = e.path.rsplit('/').next().unwrap_or(&e.path).to_string();
                Some((o.entry_ino(i), name, e.is_dir))
            }
        }
    }
}

// ---------------- 读 ----------------

/// 一个文件的读取：对上游保持一条流，缓存最近一段
struct FileStream {
    source: Arc<dyn ClipSource>,
    index: u32,
    size: u64,
    reader: Option<Box<dyn Read + Send>>,
    /// 上游下一个字节的位置
    pos: u64,
    eof: bool,
    /// 缓存 [win_start, pos)
    win: Vec<u8>,
    win_start: u64,
    buf: Vec<u8>,
}

impl FileStream {
    fn new(source: Arc<dyn ClipSource>, index: u32, size: u64) -> Self {
        Self { source, index, size, reader: None, pos: 0, eof: false, win: vec![], win_start: 0, buf: vec![] }
    }

    fn read_at(&mut self, off: u64, len: u32) -> Result<Vec<u8>, String> {
        let end = off.saturating_add(len as u64).min(self.size);
        if off >= end {
            return Ok(vec![]);
        }
        if self.reader.is_none() || off < self.win_start || off > self.pos.saturating_add(SKIP) {
            self.reader = None;
            let (_, r) = self.source.open(self.index, off)?;
            self.reader = Some(r);
            self.pos = off;
            self.win_start = off;
            self.win.clear();
            self.eof = false;
        }
        if self.buf.is_empty() {
            self.buf = vec![0u8; CHUNK];
        }
        while self.pos < end && !self.eof {
            let want = ((end - self.pos) as usize).min(CHUNK);
            let r = self.reader.as_mut().expect("上面刚打开");
            let n = loop {
                match r.read(&mut self.buf[..want]) {
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    x => break x,
                }
            };
            match n {
                Ok(0) => self.eof = true,
                Ok(n) => {
                    self.win.extend_from_slice(&self.buf[..n]);
                    self.pos += n as u64;
                }
                Err(e) => {
                    // 下次从头再开
                    self.reader = None;
                    return Err(e.to_string());
                }
            }
        }
        let a = (off - self.win_start) as usize;
        let b = (end.min(self.pos) - self.win_start) as usize;
        let out = if a < b { self.win[a..b].to_vec() } else { vec![] };
        // 攒到两倍再一次裁掉前面的，免得每次都搬动
        if self.win.len() > 2 * WINDOW {
            let cut = self.win.len() - WINDOW;
            self.win.drain(..cut);
            self.win_start += cut as u64;
        }
        Ok(out)
    }
}

struct ReadJob {
    unique: u64,
    off: u64,
    size: u32,
}

struct Slot {
    tx: mpsc::Sender<ReadJob>,
    opens: u32,
    idle_since: Instant,
}

type Slots = Arc<Mutex<HashMap<u64, Slot>>>;

// ---------------- 内核协议 ----------------

mod op {
    pub const LOOKUP: u32 = 1;
    pub const FORGET: u32 = 2;
    pub const GETATTR: u32 = 3;
    pub const SETATTR: u32 = 4;
    pub const OPEN: u32 = 14;
    pub const READ: u32 = 15;
    pub const STATFS: u32 = 17;
    pub const RELEASE: u32 = 18;
    pub const FLUSH: u32 = 25;
    pub const INIT: u32 = 26;
    pub const OPENDIR: u32 = 27;
    pub const READDIR: u32 = 28;
    pub const RELEASEDIR: u32 = 29;
    pub const INTERRUPT: u32 = 36;
    pub const DESTROY: u32 = 38;
    pub const BATCH_FORGET: u32 = 42;
}

const FUSE_ASYNC_READ: u32 = 1 << 0;
const FUSE_PARALLEL_DIROPS: u32 = 1 << 18;
const FUSE_MAX_PAGES: u32 = 1 << 22;
const FOPEN_KEEP_CACHE: u32 = 1 << 1;
/// 我们实现到的协议小版本（7.31，Linux 5.4 时代）；内核更老就按内核的
const MINOR: u32 = 31;
const HDR_IN: usize = 40;
const ENTRY_TTL: u64 = 60;
const ROOT_TTL: u64 = 1;

fn u32_at(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4).map(|x| u32::from_le_bytes(x.try_into().unwrap())).unwrap_or(0)
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    b.get(o..o + 8).map(|x| u64::from_le_bytes(x.try_into().unwrap())).unwrap_or(0)
}

#[derive(Default)]
struct Out(Vec<u8>);
impl Out {
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn zeros(&mut self, n: usize) -> &mut Self {
        self.0.resize(self.0.len() + n, 0);
        self
    }
    /// struct fuse_attr（88 字节）
    fn attr(&mut self, a: &Attr, uid: u32, gid: u32) -> &mut Self {
        let t = a.mtime.max(0) as u64;
        let mode = if a.dir { libc::S_IFDIR | 0o755 } else { libc::S_IFREG | 0o644 };
        self.u64(a.ino).u64(a.size).u64(a.size.div_ceil(512)).u64(t).u64(t).u64(t);
        self.u32(0).u32(0).u32(0).u32(mode).u32(a.nlink).u32(uid).u32(gid).u32(0).u32(CHUNK as u32).u32(0)
    }
    /// struct fuse_entry_out（128 字节）
    fn entry(&mut self, a: &Attr, ttl: u64, uid: u32, gid: u32) -> &mut Self {
        self.u64(a.ino).u64(0).u64(ttl).u64(ttl).u32(0).u32(0).attr(a, uid, gid)
    }
}

/// 回一条应答（一次 write，内核要求整条一起写）。请求已被打断时内核回 ENOENT，不用管。
fn reply(dev: &File, unique: u64, err: i32, data: &[u8]) {
    let mut v = Vec::with_capacity(16 + data.len());
    v.extend_from_slice(&((16 + data.len()) as u32).to_le_bytes());
    v.extend_from_slice(&(-err).to_le_bytes());
    v.extend_from_slice(&unique.to_le_bytes());
    v.extend_from_slice(data);
    let _ = (&*dev).write(&v);
}

struct Server {
    dev: Arc<File>,
    tree: Arc<RwLock<Tree>>,
    slots: Slots,
    uid: u32,
    gid: u32,
}

impl Server {
    fn run(self) {
        let mut buf = vec![0u8; 136 * 1024];
        loop {
            let n = match (&*self.dev).read(&mut buf) {
                Ok(n) => n,
                Err(e) => match e.raw_os_error() {
                    Some(libc::EINTR) | Some(libc::EAGAIN) | Some(libc::ENOENT) => continue,
                    // ENODEV：已卸载
                    _ => break,
                },
            };
            if n < HDR_IN {
                continue;
            }
            let req = &buf[..n];
            let opcode = u32_at(req, 4);
            let unique = u64_at(req, 8);
            let nodeid = u64_at(req, 16);
            let body = &req[HDR_IN..];
            self.handle(opcode, unique, nodeid, body);
        }
    }

    fn handle(&self, opcode: u32, unique: u64, nodeid: u64, body: &[u8]) {
        let dev = &*self.dev;
        match opcode {
            op::INIT => {
                let (major, minor, readahead, flags) = (u32_at(body, 0), u32_at(body, 4), u32_at(body, 8), u32_at(body, 12));
                if major != 7 {
                    return reply(dev, unique, libc::EPROTO, &[]);
                }
                let minor = minor.min(MINOR);
                let mut want = FUSE_ASYNC_READ | FUSE_PARALLEL_DIROPS;
                let big = flags & FUSE_MAX_PAGES != 0;
                if big {
                    want |= FUSE_MAX_PAGES;
                }
                let mut o = Out::default();
                o.u32(7).u32(minor).u32(readahead).u32(flags & want).u16(16).u16(12).u32(4096);
                if minor >= 23 {
                    // time_gran、max_pages（最多 1 MB 一次读）、其余补零到 64 字节
                    o.u32(1).u16(if big { 256 } else { 0 });
                    let pad = 64 - o.0.len();
                    o.zeros(pad);
                }
                reply(dev, unique, 0, &o.0);
            }
            op::LOOKUP => {
                let name = body.split(|b| *b == 0).next().unwrap_or_default();
                let name = String::from_utf8_lossy(name);
                let tree = self.tree.read();
                match tree.lookup(nodeid, &name) {
                    Some(n) => {
                        let a = tree.attr(&n);
                        let ttl = if nodeid == ROOT { ROOT_TTL } else { ENTRY_TTL };
                        let mut o = Out::default();
                        o.entry(&a, ttl, self.uid, self.gid);
                        reply(dev, unique, 0, &o.0);
                    }
                    None => reply(dev, unique, libc::ENOENT, &[]),
                }
            }
            op::GETATTR => {
                let tree = self.tree.read();
                match tree.node(nodeid) {
                    Some(n) => {
                        let a = tree.attr(&n);
                        let ttl = if nodeid == ROOT { ROOT_TTL } else { ENTRY_TTL };
                        let mut o = Out::default();
                        o.u64(ttl).u32(0).u32(0).attr(&a, self.uid, self.gid);
                        reply(dev, unique, 0, &o.0);
                    }
                    None => reply(dev, unique, libc::ENOENT, &[]),
                }
            }
            op::SETATTR => reply(dev, unique, libc::EROFS, &[]),
            op::OPEN => {
                let flags = u32_at(body, 0) as i32;
                if flags & libc::O_ACCMODE != libc::O_RDONLY || flags & libc::O_TRUNC != 0 {
                    return reply(dev, unique, libc::EROFS, &[]);
                }
                let node = self.tree.read().node(nodeid);
                match node {
                    Some(Node::Entry(o, i)) if !o.offer.entries[i as usize].is_dir => {
                        self.slot(nodeid, &o, i, true);
                        let mut out = Out::default();
                        out.u64(0).u32(FOPEN_KEEP_CACHE).u32(0);
                        reply(dev, unique, 0, &out.0);
                    }
                    Some(_) => reply(dev, unique, libc::EISDIR, &[]),
                    None => reply(dev, unique, libc::ENOENT, &[]),
                }
            }
            op::READ => {
                let (off, size) = (u64_at(body, 8), u32_at(body, 16));
                let tx = {
                    let g = self.slots.lock();
                    g.get(&nodeid).map(|s| s.tx.clone())
                };
                let tx = match tx {
                    Some(t) => t,
                    None => {
                        // 没打开就读（不会发生）：临时建一个
                        let node = self.tree.read().node(nodeid);
                        match node {
                            Some(Node::Entry(o, i)) if !o.offer.entries[i as usize].is_dir => self.slot(nodeid, &o, i, false),
                            _ => return reply(dev, unique, libc::EBADF, &[]),
                        }
                    }
                };
                if tx.send(ReadJob { unique, off, size }).is_err() {
                    reply(dev, unique, libc::EIO, &[]);
                }
            }
            op::RELEASE => {
                if let Some(s) = self.slots.lock().get_mut(&nodeid) {
                    s.opens = s.opens.saturating_sub(1);
                    if s.opens == 0 {
                        s.idle_since = Instant::now();
                    }
                }
                reply(dev, unique, 0, &[]);
            }
            op::OPENDIR => {
                let node = self.tree.read().node(nodeid);
                match node {
                    Some(Node::Entry(o, i)) if !o.offer.entries[i as usize].is_dir => reply(dev, unique, libc::ENOTDIR, &[]),
                    Some(_) => {
                        let mut out = Out::default();
                        out.u64(0).u32(0).u32(0);
                        reply(dev, unique, 0, &out.0);
                    }
                    None => reply(dev, unique, libc::ENOENT, &[]),
                }
            }
            op::READDIR => {
                let (start, size) = (u64_at(body, 8) as usize, u32_at(body, 16) as usize);
                let tree = self.tree.read();
                let Some(n) = tree.node(nodeid) else { return reply(dev, unique, libc::ENOENT, &[]) };
                let mut out = Vec::new();
                let mut k = start;
                while let Some((ino, name, dir)) = tree.dir_item(&n, k) {
                    let rec = (24 + name.len() + 7) & !7;
                    if out.len() + rec > size {
                        break;
                    }
                    let mut e = Out::default();
                    e.u64(ino).u64(k as u64 + 1).u32(name.len() as u32).u32(if dir { libc::DT_DIR as u32 } else { libc::DT_REG as u32 });
                    out.extend_from_slice(&e.0);
                    out.extend_from_slice(name.as_bytes());
                    out.resize(out.len() + (rec - 24 - name.len()), 0);
                    k += 1;
                }
                reply(dev, unique, 0, &out);
            }
            op::STATFS => {
                let tree = self.tree.read();
                let bytes: u64 = tree.offers.iter().map(|o| o.offer.total_bytes()).sum();
                let files: u64 = tree.offers.iter().map(|o| o.offer.entries.len() as u64 + 1).sum();
                let mut o = Out::default();
                o.u64(bytes.div_ceil(4096)).u64(0).u64(0).u64(files).u64(0).u32(4096).u32(255).u32(4096).u32(0).zeros(24);
                reply(dev, unique, 0, &o.0);
            }
            op::FLUSH | op::RELEASEDIR => reply(dev, unique, 0, &[]),
            // 不用回
            op::FORGET | op::BATCH_FORGET | op::INTERRUPT | op::DESTROY => {}
            // 其余（扩展属性、access、锁、写类操作…）：ENOSYS，内核自己兜底或报不支持
            _ => reply(dev, unique, libc::ENOSYS, &[]),
        }
    }

    /// 这个文件的读线程（没有就建一个）；`open` 为真时记一次打开
    fn slot(&self, ino: u64, o: &Arc<OfferNode>, i: u32, open: bool) -> mpsc::Sender<ReadJob> {
        let mut g = self.slots.lock();
        if let Some(s) = g.get_mut(&ino) {
            if open {
                s.opens += 1;
            }
            return s.tx.clone();
        }
        let (tx, rx) = mpsc::channel::<ReadJob>();
        g.insert(ino, Slot { tx: tx.clone(), opens: open as u32, idle_since: Instant::now() });
        let e = &o.offer.entries[i as usize];
        let stream = FileStream::new(o.source.clone(), i, e.size);
        let (dev, slots) = (self.dev.clone(), self.slots.clone());
        let spawned = std::thread::Builder::new().name("clip-read".into()).spawn(move || read_worker(ino, stream, rx, dev, slots));
        if spawned.is_err() {
            g.remove(&ino);
        }
        tx
    }
}

fn read_worker(ino: u64, mut stream: FileStream, rx: mpsc::Receiver<ReadJob>, dev: Arc<File>, slots: Slots) {
    let serve = |stream: &mut FileStream, j: ReadJob| match stream.read_at(j.off, j.size) {
        Ok(d) => reply(&dev, j.unique, 0, &d),
        Err(e) => {
            eprintln!("粘贴时读文件失败：{e}");
            reply(&dev, j.unique, libc::EIO, &[]);
        }
    };
    loop {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(j) => serve(&mut stream, j),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let mut g = slots.lock();
                // 锁着再看一眼：有新请求先处理
                if let Ok(j) = rx.try_recv() {
                    drop(g);
                    serve(&mut stream, j);
                    continue;
                }
                let done = g.get(&ino).map(|s| s.opens == 0 && s.idle_since.elapsed() > LINGER).unwrap_or(true);
                if done {
                    g.remove(&ino);
                    return;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

// ---------------- 挂载 ----------------

/// fusermount3（libfuse 3）或 fusermount（libfuse 2）
fn find_helper() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(["/usr/bin", "/bin", "/usr/local/bin", "/usr/sbin", "/sbin"].map(PathBuf::from));
    for name in ["fusermount3", "fusermount"] {
        for d in &dirs {
            let p = d.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// FUSE 能不能用：有 /dev/fuse 和挂载程序。不能用时返回原因。
pub fn availability() -> Result<PathBuf, String> {
    if !Path::new("/dev/fuse").exists() {
        return Err("没有 /dev/fuse".into());
    }
    find_helper().ok_or_else(|| "没有 fusermount3（请安装 fuse3）".into())
}

/// /proc/self/mountinfo 里的挂载点（第 5 列，空格等字符是 \040 这样的八进制转义）
fn mount_points(info: &str) -> Vec<String> {
    info.lines()
        .filter_map(|l| l.split(' ').nth(4))
        .map(|p| {
            let b = p.as_bytes();
            let mut out = Vec::with_capacity(b.len());
            let mut i = 0;
            while i < b.len() {
                if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
                    out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0'));
                    i += 4;
                } else {
                    out.push(b[i]);
                    i += 1;
                }
            }
            String::from_utf8_lossy(&out).to_string()
        })
        .collect()
}

fn is_mounted(dir: &Path) -> bool {
    let info = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    let d = dir.to_string_lossy();
    mount_points(&info).iter().any(|p| *p == d)
}

fn run_unmount(helper: &Path, dir: &Path) -> bool {
    Command::new(helper).args(["-u", "-z", "-q"]).arg(dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

/// 从 socket 收一个 SCM_RIGHTS 传来的描述符
fn recv_fd(sock: &UnixStream) -> io::Result<File> {
    let mut data = [0u8; 1];
    let mut iov = libc::iovec { iov_base: data.as_mut_ptr() as *mut libc::c_void, iov_len: 1 };
    // 按 cmsghdr 对齐
    let mut cbuf = [0u64; 8];
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cbuf.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = std::mem::size_of_val(&cbuf) as _;
    loop {
        let n = unsafe { libc::recvmsg(sock.as_raw_fd(), &mut msg, 0) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        break;
    }
    unsafe {
        let c = libc::CMSG_FIRSTHDR(&msg);
        if c.is_null() || (*c).cmsg_level != libc::SOL_SOCKET || (*c).cmsg_type != libc::SCM_RIGHTS {
            return Err(io::Error::other("挂载程序没有传回描述符"));
        }
        let fd = std::ptr::read_unaligned(libc::CMSG_DATA(c) as *const libc::c_int);
        if fd < 0 {
            return Err(io::Error::other("描述符无效"));
        }
        libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        Ok(File::from_raw_fd(fd))
    }
}

/// 经 fusermount 挂上，返回 /dev/fuse 描述符与（auto_unmount 时）要一直拿着的连接
fn mount_with(helper: &Path, dir: &Path, auto_unmount: bool) -> Result<(File, Option<UnixStream>), String> {
    let (ours, theirs) = UnixStream::pair().map_err(|e| e.to_string())?;
    let fd = theirs.as_raw_fd();
    let mut opts = String::from("ro,nosuid,nodev,noexec,fsname=lemon-galaxy,subtype=lemon-galaxy");
    if auto_unmount {
        opts.push_str(",auto_unmount");
    }
    let mut cmd = Command::new(helper);
    // 标准错误直接用我们的（画面代理的标准错误接到服务日志，比我们活得久）：auto_unmount 时它在我们退出后才卸载，
    // 要是接到我们自己的管道，我们一死它一写就被 SIGPIPE 杀掉、来不及卸载
    cmd.arg("-o").arg(&opts).arg("--").arg(dir).env("_FUSE_COMMFD", fd.to_string()).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::inherit());
    // 子进程里把这一端的 CLOEXEC 去掉（fork 之后、exec 之前只做 fcntl）
    unsafe {
        cmd.pre_exec(move || {
            let fl = libc::fcntl(fd, libc::F_GETFD);
            if fl >= 0 {
                libc::fcntl(fd, libc::F_SETFD, fl & !libc::FD_CLOEXEC);
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|e| format!("启动 {} 失败：{e}", helper.display()))?;
    drop(theirs);
    match recv_fd(&ours) {
        Ok(dev) => {
            // auto_unmount 时它一直在，退出后收尸
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            // 进程退出时内核按描述符从小到大关：让 /dev/fuse 先关（连接断开）、这条连接后关，
            // fusermount 被唤醒时看到的才是「已断开」的挂载点，才会卸载（反过来它看到连接还在、不卸）
            let ours = unsafe {
                let n = libc::fcntl(ours.as_raw_fd(), libc::F_DUPFD_CLOEXEC, dev.as_raw_fd() + 1);
                if n >= 0 {
                    UnixStream::from_raw_fd(n)
                } else {
                    ours
                }
            };
            Ok((dev, auto_unmount.then_some(ours)))
        }
        Err(e) => {
            drop(ours);
            let st = child.wait().map(|s| s.to_string()).unwrap_or_default();
            Err(format!("{} 挂载失败（{st}，{e}；原因见日志里 fusermount 的输出）", helper.display()))
        }
    }
}

/// 挂好的虚拟目录。丢掉不会卸载，要卸载调 [`Mount::unmount`]（进程退出时 fusermount 也会卸）。
pub struct Mount {
    pub dir: PathBuf,
    tree: Arc<RwLock<Tree>>,
    helper: PathBuf,
    comm: Mutex<Option<UnixStream>>,
}

impl Mount {
    pub fn start(dir: &Path) -> Result<Arc<Mount>, String> {
        let helper = availability()?;
        // 上次没卸掉的（代理被 kill 且 fusermount 不支持 auto_unmount）先卸掉
        if is_mounted(dir) {
            run_unmount(&helper, dir);
        }
        std::fs::create_dir_all(dir).map_err(|e| format!("建不了 {}：{e}", dir.display()))?;
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
            // 上一级是我们自己的 lemon-galaxy 目录时也收紧（别动 /tmp 之类的公共目录）
            if let Some(p) = dir.parent().filter(|p| p.file_name().is_some_and(|n| n == "lemon-galaxy")) {
                let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700));
            }
        }
        let (dev, comm) = match mount_with(&helper, dir, true) {
            Ok(x) => x,
            // 老的 fusermount 不认 auto_unmount：不带它再试
            Err(e1) => mount_with(&helper, dir, false).map_err(|e2| if e1 == e2 { e1 } else { format!("{e1}；{e2}") })?,
        };
        let tree = Arc::new(RwLock::new(Tree::new()));
        let server = Server { dev: Arc::new(dev), tree: tree.clone(), slots: Default::default(), uid: unsafe { libc::getuid() }, gid: unsafe { libc::getgid() } };
        std::thread::Builder::new().name("clip-fuse".into()).spawn(move || server.run()).map_err(|e| e.to_string())?;
        Ok(Arc::new(Mount { dir: dir.to_path_buf(), tree, helper, comm: Mutex::new(comm) }))
    }

    /// 放进一份清单，返回顶层项的路径（放进剪贴板的就是它们）
    pub fn add(&self, offer: ClipOffer, source: Arc<dyn ClipSource>) -> Vec<PathBuf> {
        let n = self.tree.write().add(offer, source);
        let base = self.dir.join(&n.name);
        n.top_names().into_iter().map(|t| base.join(t)).collect()
    }

    pub fn unmount(&self) {
        run_unmount(&self.helper, &self.dir);
        self.comm.lock().take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use galaxy_proto::peer::ClipOfferEntry;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 内存里的来源，记下打开了几次
    pub struct Mem(pub Vec<Vec<u8>>, pub AtomicUsize);
    impl ClipSource for Mem {
        fn open(&self, index: u32, offset: u64) -> Result<(u64, Box<dyn Read + Send>), String> {
            self.1.fetch_add(1, Ordering::Relaxed);
            let d = self.0.get(index as usize).ok_or("没有")?.clone();
            let size = d.len() as u64;
            Ok((size, Box::new(io::Cursor::new(d[offset as usize..].to_vec()))))
        }
    }

    fn entry(path: &str, dir: bool, size: u64) -> ClipOfferEntry {
        ClipOfferEntry { path: path.into(), is_dir: dir, size, mtime: 1_700_000_000 }
    }

    fn sample() -> (ClipOffer, Vec<Vec<u8>>) {
        let big: Vec<u8> = (0..(9u32 << 20)).map(|i| (i * 7 % 251) as u8).collect();
        let offer = ClipOffer {
            id: 0xabc,
            entries: vec![entry("资料", true, 0), entry("资料/子", true, 0), entry("资料/子/big.bin", false, big.len() as u64), entry("资料/a.txt", false, 5), entry("b c.txt", false, 3)],
        };
        (offer, vec![vec![], vec![], big, b"hello".to_vec(), b"xyz".to_vec()])
    }

    #[test]
    fn tree_layout() {
        let (offer, data) = sample();
        let mut t = Tree::new();
        let n = t.add(offer, Arc::new(Mem(data, AtomicUsize::new(0))));
        assert_eq!(n.name, "abc");
        assert_eq!(n.top_names(), vec!["资料".to_string(), "b c.txt".to_string()]);
        let Some(Node::Offer(o)) = t.lookup(ROOT, "abc") else { panic!() };
        // 清单根：「.」「..」、资料、b c.txt
        let items: Vec<_> = (0..).map_while(|k| t.dir_item(&Node::Offer(o.clone()), k)).map(|(_, n, d)| (n, d)).collect();
        assert_eq!(items, vec![(".".into(), true), ("..".into(), true), ("资料".into(), true), ("b c.txt".into(), false)]);
        let dir = t.lookup(o.ino, "资料").unwrap();
        let a = t.attr(&dir);
        assert!(a.dir);
        assert_eq!(a.nlink, 3);
        let sub = t.lookup(a.ino, "子").unwrap();
        let sa = t.attr(&sub);
        let f = t.lookup(sa.ino, "big.bin").unwrap();
        let fa = t.attr(&f);
        assert_eq!((fa.size, fa.dir, fa.mtime), (9 << 20, false, 1_700_000_000));
        // 「..」指回上一级
        assert_eq!(t.dir_item(&sub, 1).unwrap().0, a.ino);
        assert!(t.lookup(a.ino, "nope").is_none());
        assert!(matches!(t.node(fa.ino), Some(Node::Entry(_, 2))));
    }

    #[test]
    fn keeps_last_offers_only() {
        let mut t = Tree::new();
        let mut first = 0;
        for id in 1..=6u64 {
            let n = t.add(ClipOffer { id, entries: vec![entry("f", false, 1)] }, Arc::new(Mem(vec![b"x".to_vec()], AtomicUsize::new(0))));
            if id == 1 {
                first = n.ino;
            }
        }
        assert_eq!(t.offers.len(), KEEP_OFFERS);
        assert!(t.lookup(ROOT, "1").is_none());
        assert!(t.lookup(ROOT, "6").is_some());
        assert!(t.node(first).is_none());
    }

    #[test]
    fn stream_reuses_and_reopens() {
        let (_, data) = sample();
        let big = data[2].clone();
        let src = Arc::new(Mem(data, AtomicUsize::new(0)));
        let mut s = FileStream::new(src.clone(), 2, big.len() as u64);
        let opens = || src.1.load(Ordering::Relaxed);
        // 顺序读
        let mut off = 0u64;
        while off < 3 << 20 {
            let d = s.read_at(off, 128 * 1024).unwrap();
            assert_eq!(d, big[off as usize..off as usize + d.len()]);
            off += d.len() as u64;
        }
        assert_eq!(opens(), 1);
        // 乱序（预读）：往后一点、再回到缓存里的位置
        assert_eq!(s.read_at(off + 256 * 1024, 4096).unwrap(), big[(off + 256 * 1024) as usize..][..4096]);
        assert_eq!(s.read_at(off, 4096).unwrap(), big[off as usize..][..4096]);
        assert_eq!(opens(), 1);
        // 往回跳但还在缓存里：不重新打开
        assert_eq!(s.read_at(5, 10).unwrap(), big[5..15]);
        assert_eq!(opens(), 1);
        // 往后跳很远、再往回跳到缓存外：都重新打开
        assert_eq!(s.read_at(8 << 20, 100).unwrap(), big[8 << 20..][..100]);
        assert_eq!(opens(), 2);
        assert_eq!(s.read_at(5, 10).unwrap(), big[5..15]);
        assert_eq!(opens(), 3);
        // 读到末尾之外
        let end = big.len() as u64;
        assert_eq!(s.read_at(end - 3, 100).unwrap(), big[end as usize - 3..]);
        assert!(s.read_at(end, 100).unwrap().is_empty());
        // 缓存不会无限长
        assert!(s.win.len() <= 2 * WINDOW + CHUNK);
    }

    #[test]
    fn parses_mountinfo() {
        let info = "36 35 98:0 / /mnt1 rw,noatime - ext3 /dev/root rw\n40 30 0:50 / /run/user/1000/lemon\\040galaxy/clip ro - fuse.lemon-galaxy lemon-galaxy ro\n";
        assert_eq!(mount_points(info), vec!["/mnt1".to_string(), "/run/user/1000/lemon galaxy/clip".to_string()]);
    }

    /// 挂上后一直不退（LG_TEST_FUSE_HOLD=<目录>）：用来看被 kill -9 后 fusermount 有没有自动卸载
    #[test]
    fn hold_mount() {
        let Some(dir) = std::env::var_os("LG_TEST_FUSE_HOLD") else { return };
        let dir = PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        let m = Mount::start(&dir).expect("挂载");
        m.add(ClipOffer { id: 1, entries: vec![entry("x.txt", false, 1)] }, Arc::new(Mem(vec![b"x".to_vec()], AtomicUsize::new(0))));
        println!("mounted {}", dir.display());
        std::thread::sleep(Duration::from_secs(600));
    }

    /// 真挂载（要 /dev/fuse 与 fusermount3，容器里加 --device /dev/fuse --cap-add SYS_ADMIN）：LG_TEST_FUSE=1 时跑
    #[test]
    fn real_mount() {
        if std::env::var("LG_TEST_FUSE").ok().as_deref() != Some("1") {
            return;
        }
        let dir = std::env::temp_dir().join(format!("lg-fuse-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let m = Mount::start(&dir).expect("挂载");
        let (offer, data) = sample();
        let src = Arc::new(Mem(data.clone(), AtomicUsize::new(0)));
        let tops = m.add(offer, src.clone());
        assert_eq!(tops, vec![dir.join("abc/资料"), dir.join("abc/b c.txt")]);
        assert!(is_mounted(&dir));
        // 列目录、属性
        let mut names: Vec<String> = std::fs::read_dir(dir.join("abc/资料")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        names.sort();
        assert_eq!(names, vec!["a.txt".to_string(), "子".to_string()]);
        let md = std::fs::metadata(dir.join("abc/资料/子/big.bin")).unwrap();
        assert_eq!(md.len(), 9 << 20);
        assert!(std::fs::metadata(dir.join("abc/资料")).unwrap().is_dir());
        // 读：整文件、从中间读
        assert_eq!(std::fs::read(dir.join("abc/b c.txt")).unwrap(), b"xyz");
        assert_eq!(std::fs::read(dir.join("abc/资料/子/big.bin")).unwrap(), data[2]);
        {
            use std::os::unix::fs::FileExt;
            let f = File::open(dir.join("abc/资料/子/big.bin")).unwrap();
            let mut b = vec![0u8; 1000];
            f.read_exact_at(&mut b, 7_000_001).unwrap();
            assert_eq!(b, data[2][7_000_001..7_001_001]);
        }
        // 只读
        assert!(std::fs::write(dir.join("abc/new.txt"), b"x").is_err());
        assert!(std::fs::OpenOptions::new().write(true).open(dir.join("abc/b c.txt")).is_err());
        // cp -r 整棵树，和原内容比
        let dst = std::env::temp_dir().join(format!("lg-fuse-dst-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dst);
        std::fs::create_dir_all(&dst).unwrap();
        let st = Command::new("cp").arg("-r").arg(dir.join("abc/资料")).arg(&dst).status().unwrap();
        assert!(st.success());
        assert_eq!(std::fs::read(dst.join("资料/子/big.bin")).unwrap(), data[2]);
        assert_eq!(std::fs::read(dst.join("资料/a.txt")).unwrap(), b"hello");
        let _ = std::fs::remove_dir_all(&dst);
        // 不存在的
        assert!(std::fs::metadata(dir.join("abc/nope")).is_err());
        m.unmount();
        assert!(!is_mounted(&dir));
        let _ = std::fs::remove_dir(&dir);
    }
}
