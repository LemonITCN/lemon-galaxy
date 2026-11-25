//! X11 剪贴板（CLIPBOARD 选择区）。单独一个线程、一个连接、一个隐藏窗口：
//! 取内容时向当前所有者要（支持 INCR 分段），放内容时自己当所有者、按请求给数据（大数据用 INCR 分段发），
//! 直到别的程序接管剪贴板。粘贴时才准备的文件列表（没有 FUSE 时的兜底）：请求来了在另一个线程里等它准备好，
//! 好了再回给请求方，选择区线程不被卡住。

use super::super::clipfmt::{self, LazyFiles};
use super::connect;
use galaxy_proto::peer::ClipboardData;
use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xproto::*;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::CURRENT_TIME;

enum Cmd {
    Get(mpsc::Sender<Option<ClipboardData>>),
    Set(ClipboardData),
    SetLazy(Arc<dyn LazyFiles>),
    /// 粘贴时才准备的内容好了：回给当时的请求方（类型, 数据；None 表示失败）
    Deliver(SelectionRequestEvent, Option<(Atom, Vec<u8>)>),
}

/// 我们放进剪贴板的内容
#[derive(Clone)]
enum Owned {
    Data(Vec<(Atom, Vec<u8>)>),
    Lazy(Vec<Atom>, Arc<dyn LazyFiles>),
}

pub struct Clipboard {
    tx: mpsc::Sender<Cmd>,
}

impl Clipboard {
    pub fn start() -> Result<Self, String> {
        let (conn, screen) = connect()?;
        let (tx, rx) = mpsc::channel();
        let mut st = State::new(conn, screen, tx.clone())?;
        std::thread::Builder::new().name("x11-clipboard".into()).spawn(move || st.run(rx)).map_err(|e| e.to_string())?;
        Ok(Self { tx })
    }

    pub fn get(&self) -> Option<ClipboardData> {
        let (tx, rx) = mpsc::channel();
        self.tx.send(Cmd::Get(tx)).ok()?;
        rx.recv_timeout(Duration::from_secs(4)).ok().flatten()
    }

    pub fn set(&self, d: ClipboardData) {
        let _ = self.tx.send(Cmd::Set(d));
    }

    pub fn set_lazy(&self, f: Arc<dyn LazyFiles>) {
        let _ = self.tx.send(Cmd::SetLazy(f));
    }
}

struct IncrSend {
    requestor: Window,
    property: Atom,
    kind: Atom,
    data: Vec<u8>,
    offset: usize,
    started: Instant,
}

struct State {
    conn: RustConnection,
    win: Window,
    atoms: HashMap<String, Atom>,
    /// 我们放的内容
    owned: Option<Owned>,
    /// 粘贴时才准备的内容好了以后经它回到本线程
    tx: mpsc::Sender<Cmd>,
    owned_time: u32,
    incr: Vec<IncrSend>,
    chunk: usize,
}

const NAMES: &[&str] = &["CLIPBOARD", "TARGETS", "TIMESTAMP", "MULTIPLE", "INCR", "ATOM", "INTEGER", "LG_CLIP", "UTF8_STRING", "STRING", "TEXT", "text/plain;charset=utf-8", "text/plain", "image/png", "text/uri-list", "x-special/gnome-copied-files"];

impl State {
    fn new(conn: RustConnection, screen: usize, tx: mpsc::Sender<Cmd>) -> Result<Self, String> {
        let root = conn.setup().roots[screen].root;
        let win = conn.generate_id().map_err(|e| e.to_string())?;
        conn.create_window(0, win, root, -10, -10, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE)).map_err(|e| e.to_string())?;
        let mut atoms = HashMap::new();
        for n in NAMES {
            if let Some(a) = conn.intern_atom(false, n.as_bytes()).ok().and_then(|c| c.reply().ok()) {
                atoms.insert(n.to_string(), a.atom);
            }
        }
        // 一次 ChangeProperty 最多能带多少；留余量
        let chunk = (conn.maximum_request_bytes().saturating_sub(1024)).clamp(4096, 256 * 1024);
        let _ = conn.flush();
        Ok(Self { conn, win, atoms, owned: None, tx, owned_time: CURRENT_TIME, incr: vec![], chunk })
    }

    fn atom(&mut self, name: &str) -> Atom {
        if let Some(a) = self.atoms.get(name) {
            return *a;
        }
        let a = self.conn.intern_atom(false, name.as_bytes()).ok().and_then(|c| c.reply().ok()).map(|r| r.atom).unwrap_or(x11rb::NONE);
        self.atoms.insert(name.to_string(), a);
        a
    }

    fn atom_name(&self, a: Atom) -> String {
        if let Some((k, _)) = self.atoms.iter().find(|(_, v)| **v == a) {
            return k.clone();
        }
        self.conn.get_atom_name(a).ok().and_then(|c| c.reply().ok()).map(|r| String::from_utf8_lossy(&r.name).to_string()).unwrap_or_default()
    }

    fn run(&mut self, rx: mpsc::Receiver<Cmd>) {
        loop {
            match rx.try_recv() {
                Ok(Cmd::Get(reply)) => {
                    let d = self.fetch();
                    let _ = reply.send(d);
                    continue;
                }
                Ok(Cmd::Set(d)) => {
                    let offers = clipfmt::offers(&d).into_iter().map(|(m, data)| (self.atom(&m), data)).collect();
                    self.own(Owned::Data(offers));
                    continue;
                }
                Ok(Cmd::SetLazy(f)) => {
                    let types = clipfmt::LAZY_TYPES.iter().map(|m| self.atom(m)).collect();
                    self.own(Owned::Lazy(types, f));
                    continue;
                }
                Ok(Cmd::Deliver(r, data)) => {
                    let property = if r.property == x11rb::NONE { r.target } else { r.property };
                    let ok = match data {
                        Some((kind, d)) => self.put(&r, property, kind, &d),
                        None => false,
                    };
                    self.notify(&r, property, ok);
                    continue;
                }
                Err(mpsc::TryRecvError::Disconnected) => return,
                Err(mpsc::TryRecvError::Empty) => {}
            }
            if self.pump(Duration::from_millis(50)).is_err() {
                return;
            }
        }
    }

    /// 等并处理事件最多 `d`，返回处理过的事件（取内容时看 SelectionNotify / PropertyNotify）
    fn pump(&mut self, d: Duration) -> Result<Vec<Event>, ()> {
        let mut seen = vec![];
        let mut first = true;
        loop {
            let ev = match self.conn.poll_for_event() {
                Ok(Some(e)) => e,
                Ok(None) => {
                    if !first {
                        return Ok(seen);
                    }
                    first = false;
                    let mut pfd = libc::pollfd { fd: self.conn.stream().as_raw_fd(), events: libc::POLLIN, revents: 0 };
                    unsafe { libc::poll(&mut pfd, 1, d.as_millis() as i32) };
                    if pfd.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
                        return Err(());
                    }
                    continue;
                }
                Err(_) => return Err(()),
            };
            match &ev {
                Event::SelectionRequest(r) => self.serve(r),
                Event::SelectionClear(c) if c.owner == self.win => {
                    self.owned = None;
                }
                Event::PropertyNotify(p) if p.state == Property::DELETE && p.window != self.win => self.incr_continue(p.window, p.atom),
                _ => {}
            }
            seen.push(ev);
        }
    }

    // ---------------- 放内容 ----------------

    /// 拿一个服务器时间戳（ICCCM 不建议用 CurrentTime 抢所有权）
    fn timestamp(&mut self) -> u32 {
        let prop = self.atom("LG_CLIP");
        let _ = self.conn.change_property8(PropMode::APPEND, self.win, prop, AtomEnum::STRING, &[]);
        let _ = self.conn.flush();
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            let Ok(evs) = self.pump(Duration::from_millis(50)) else { break };
            for e in evs {
                if let Event::PropertyNotify(p) = e {
                    if p.window == self.win && p.atom == prop {
                        return p.time;
                    }
                }
            }
        }
        CURRENT_TIME
    }

    fn own(&mut self, o: Owned) {
        self.owned_time = self.timestamp();
        self.owned = Some(o);
        let sel = self.atom("CLIPBOARD");
        let _ = self.conn.set_selection_owner(self.win, sel, self.owned_time);
        let _ = self.conn.flush();
    }

    fn serve(&mut self, r: &SelectionRequestEvent) {
        let property = if r.property == x11rb::NONE { r.target } else { r.property };
        let targets = self.atom("TARGETS");
        let timestamp = self.atom("TIMESTAMP");
        let mut ok = false;
        if r.selection == self.atom("CLIPBOARD") {
            match self.owned.clone() {
                Some(owned) if r.target == targets => {
                    let mut list = vec![targets, timestamp];
                    match &owned {
                        Owned::Data(v) => list.extend(v.iter().map(|(a, _)| *a)),
                        Owned::Lazy(t, _) => list.extend(t.iter().copied()),
                    }
                    ok = self.conn.change_property32(PropMode::REPLACE, r.requestor, property, AtomEnum::ATOM, &list).is_ok();
                }
                Some(_) if r.target == timestamp => {
                    ok = self.conn.change_property32(PropMode::REPLACE, r.requestor, property, AtomEnum::INTEGER, &[self.owned_time]).is_ok();
                }
                Some(Owned::Data(v)) => {
                    if let Some((kind, data)) = v.iter().find(|(a, _)| *a == r.target) {
                        ok = self.put(r, property, *kind, data);
                    }
                }
                Some(Owned::Lazy(types, files)) if types.contains(&r.target) => {
                    // 先准备文件（可能要下载一阵），好了经 Deliver 回到本线程再答复
                    let (req, kind, mime, tx) = (*r, r.target, self.atom_name(r.target), self.tx.clone());
                    std::thread::spawn(move || {
                        let data = files.paths().and_then(|p| clipfmt::files_data(&mime, p)).map(|d| (kind, d));
                        let _ = tx.send(Cmd::Deliver(req, data));
                    });
                    return;
                }
                _ => {}
            }
        }
        self.notify(r, property, ok);
    }

    /// 把数据放到请求方的属性上（大数据用 INCR 分段）
    fn put(&mut self, r: &SelectionRequestEvent, property: Atom, kind: Atom, data: &[u8]) -> bool {
        if data.len() > self.chunk {
            // INCR：先告诉对方总大小，对方每删一次属性我们就写下一段
            let incr = self.atom("INCR");
            let _ = self.conn.change_window_attributes(r.requestor, &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE));
            let ok = self.conn.change_property32(PropMode::REPLACE, r.requestor, property, incr, &[data.len() as u32]).is_ok();
            self.incr.retain(|x| x.started.elapsed() < Duration::from_secs(30));
            self.incr.push(IncrSend { requestor: r.requestor, property, kind, data: data.to_vec(), offset: 0, started: Instant::now() });
            ok
        } else {
            self.conn.change_property8(PropMode::REPLACE, r.requestor, property, kind, data).is_ok()
        }
    }

    fn notify(&mut self, r: &SelectionRequestEvent, property: Atom, ok: bool) {
        let ev = SelectionNotifyEvent { response_type: SELECTION_NOTIFY_EVENT, sequence: 0, time: r.time, requestor: r.requestor, selection: r.selection, target: r.target, property: if ok { property } else { x11rb::NONE } };
        let _ = self.conn.send_event(false, r.requestor, EventMask::NO_EVENT, ev);
        let _ = self.conn.flush();
    }

    fn incr_continue(&mut self, win: Window, prop: Atom) {
        let Some(i) = self.incr.iter().position(|x| x.requestor == win && x.property == prop) else { return };
        let chunk = self.chunk;
        let x = &mut self.incr[i];
        let end = (x.offset + chunk).min(x.data.len());
        let part = x.data[x.offset..end].to_vec();
        let (kind, done) = (x.kind, part.is_empty());
        x.offset = end;
        let _ = self.conn.change_property8(PropMode::REPLACE, win, prop, kind, &part);
        if done {
            self.incr.remove(i);
            let _ = self.conn.change_window_attributes(win, &ChangeWindowAttributesAux::new().event_mask(EventMask::NO_EVENT));
        }
        let _ = self.conn.flush();
    }

    // ---------------- 取内容 ----------------

    /// 请求把选择区转成 `target` 放到我们窗口的属性上，返回 (类型, 数据)
    fn convert(&mut self, target: Atom) -> Option<(Atom, Vec<u8>)> {
        let sel = self.atom("CLIPBOARD");
        let prop = self.atom("LG_CLIP");
        let incr = self.atom("INCR");
        let _ = self.conn.delete_property(self.win, prop);
        self.conn.convert_selection(self.win, sel, target, prop, CURRENT_TIME).ok()?;
        self.conn.flush().ok()?;
        let deadline = Instant::now() + Duration::from_millis(1500);
        let mut notified = false;
        while !notified && Instant::now() < deadline {
            for e in self.pump(Duration::from_millis(50)).ok()? {
                if let Event::SelectionNotify(n) = e {
                    if n.requestor == self.win && n.selection == sel {
                        if n.property == x11rb::NONE {
                            return None;
                        }
                        notified = true;
                    }
                }
            }
        }
        if !notified {
            return None;
        }
        let r = self.conn.get_property(true, self.win, prop, AtomEnum::ANY, 0, u32::MAX / 4).ok()?.reply().ok()?;
        if r.type_ != incr {
            return Some((r.type_, r.value));
        }
        // INCR：每次属性有新值就读走（读走时删掉，对方再写下一段），读到空值结束
        let mut data = Vec::new();
        let mut kind = x11rb::NONE;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if Instant::now() > deadline {
                return None;
            }
            let evs = self.pump(Duration::from_millis(100)).ok()?;
            let fresh = evs.iter().any(|e| matches!(e, Event::PropertyNotify(p) if p.window == self.win && p.atom == prop && p.state == Property::NEW_VALUE));
            if !fresh {
                continue;
            }
            let r = self.conn.get_property(true, self.win, prop, AtomEnum::ANY, 0, u32::MAX / 4).ok()?.reply().ok()?;
            let _ = self.conn.flush();
            if r.value.is_empty() {
                break;
            }
            kind = r.type_;
            data.extend_from_slice(&r.value);
        }
        Some((kind, data))
    }

    fn fetch(&mut self) -> Option<ClipboardData> {
        let sel = self.atom("CLIPBOARD");
        let owner = self.conn.get_selection_owner(sel).ok()?.reply().ok()?.owner;
        if owner == x11rb::NONE {
            return None;
        }
        if owner == self.win {
            // 自己放的：直接按第一种类型解析；粘贴时才准备的文件列表不算（是主控端复制的，不能传回去）
            let Some(Owned::Data(v)) = self.owned.as_ref() else { return None };
            let (a, d) = v.first()?.clone();
            let name = self.atom_name(a);
            return clipfmt::parse(&name, d);
        }
        let targets_atom = self.atom("TARGETS");
        let (_, raw) = self.convert(targets_atom)?;
        let atoms: Vec<Atom> = raw.chunks_exact(4).map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]])).collect();
        let names: Vec<String> = atoms.iter().map(|a| self.atom_name(*a)).collect();
        let want = clipfmt::pick(&names)?;
        let target = self.atom(&want);
        let (_, data) = self.convert(target)?;
        clipfmt::parse(&want, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Slow(Vec<String>);
    impl LazyFiles for Slow {
        fn paths(&self) -> Option<Vec<String>> {
            std::thread::sleep(Duration::from_millis(1500));
            Some(self.0.clone())
        }
    }

    fn xclip(target: &str) -> String {
        let o = std::process::Command::new("xclip").args(["-o", "-selection", "clipboard", "-t", target]).output().expect("xclip");
        String::from_utf8_lossy(&o.stdout).to_string()
    }

    /// 要 X 服务器与 xclip（容器里 Xvfb）：LG_TEST_X11=1 时跑
    #[test]
    fn serves_static_and_lazy_files() {
        if std::env::var("LG_TEST_X11").ok().as_deref() != Some("1") {
            return;
        }
        let c = Clipboard::start().unwrap();
        c.set(ClipboardData::Files(vec!["/run/user/1000/lemon-galaxy/clip/7/a b.txt".into()]));
        std::thread::sleep(Duration::from_millis(300));
        assert!(xclip("TARGETS").contains("x-special/gnome-copied-files"));
        assert_eq!(xclip("text/uri-list"), "file:///run/user/1000/lemon-galaxy/clip/7/a%20b.txt\r\n");
        assert_eq!(xclip("x-special/gnome-copied-files"), "copy\nfile:///run/user/1000/lemon-galaxy/clip/7/a%20b.txt");
        // 粘贴时才准备：等它准备好（1.5 秒）再回；期间选择区线程照常（TARGETS 立刻有）
        c.set_lazy(Arc::new(Slow(vec!["/home/u/.lemon-galaxy/clipboard/1-7/资料".into()])));
        std::thread::sleep(Duration::from_millis(300));
        let t0 = Instant::now();
        let targets = xclip("TARGETS");
        assert!(targets.contains("text/uri-list") && !targets.contains("UTF8_STRING"), "{targets}");
        assert!(t0.elapsed() < Duration::from_millis(1000));
        let got = xclip("text/uri-list");
        assert!(t0.elapsed() >= Duration::from_millis(1400));
        assert_eq!(got, "file:///home/u/.lemon-galaxy/clipboard/1-7/%E8%B5%84%E6%96%99\r\n");
        // 自己放的粘贴时才准备的列表不当成剪贴板内容（不能传回主控端）
        assert!(c.get().is_none());
    }
}
