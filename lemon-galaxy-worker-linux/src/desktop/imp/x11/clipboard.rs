//! X11 剪贴板（CLIPBOARD 选择区）。单独一个线程、一个连接、一个隐藏窗口：
//! 取内容时向当前所有者要（支持 INCR 分段），放内容时自己当所有者、按请求给数据（大数据用 INCR 分段发），
//! 直到别的程序接管剪贴板。

use super::super::clipfmt;
use super::connect;
use galaxy_proto::peer::ClipboardData;
use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::sync::mpsc;
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
}

pub struct Clipboard {
    tx: mpsc::Sender<Cmd>,
}

impl Clipboard {
    pub fn start() -> Result<Self, String> {
        let (conn, screen) = connect()?;
        let (tx, rx) = mpsc::channel();
        let mut st = State::new(conn, screen)?;
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
    /// 我们放的内容：(目标类型, 数据)
    owned: Option<Vec<(Atom, Vec<u8>)>>,
    owned_time: u32,
    incr: Vec<IncrSend>,
    chunk: usize,
}

const NAMES: &[&str] = &["CLIPBOARD", "TARGETS", "TIMESTAMP", "MULTIPLE", "INCR", "ATOM", "INTEGER", "LG_CLIP", "UTF8_STRING", "STRING", "TEXT", "text/plain;charset=utf-8", "text/plain", "image/png", "text/uri-list", "x-special/gnome-copied-files"];

impl State {
    fn new(conn: RustConnection, screen: usize) -> Result<Self, String> {
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
        Ok(Self { conn, win, atoms, owned: None, owned_time: CURRENT_TIME, incr: vec![], chunk })
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
                    self.own(d);
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

    fn own(&mut self, d: ClipboardData) {
        let offers: Vec<(Atom, Vec<u8>)> = clipfmt::offers(&d).into_iter().map(|(m, data)| (self.atom(&m), data)).collect();
        self.owned_time = self.timestamp();
        self.owned = Some(offers);
        let sel = self.atom("CLIPBOARD");
        let _ = self.conn.set_selection_owner(self.win, sel, self.owned_time);
        let _ = self.conn.flush();
    }

    fn serve(&mut self, r: &SelectionRequestEvent) {
        let property = if r.property == x11rb::NONE { r.target } else { r.property };
        let targets = self.atom("TARGETS");
        let timestamp = self.atom("TIMESTAMP");
        let incr = self.atom("INCR");
        let mut ok = false;
        if r.selection == self.atom("CLIPBOARD") {
            if let Some(owned) = self.owned.clone() {
                if r.target == targets {
                    let mut list = vec![targets, timestamp];
                    list.extend(owned.iter().map(|(a, _)| *a));
                    ok = self.conn.change_property32(PropMode::REPLACE, r.requestor, property, AtomEnum::ATOM, &list).is_ok();
                } else if r.target == timestamp {
                    ok = self.conn.change_property32(PropMode::REPLACE, r.requestor, property, AtomEnum::INTEGER, &[self.owned_time]).is_ok();
                } else if let Some((kind, data)) = owned.iter().find(|(a, _)| *a == r.target) {
                    if data.len() > self.chunk {
                        // INCR：先告诉对方总大小，对方每删一次属性我们就写下一段
                        let _ = self.conn.change_window_attributes(r.requestor, &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE));
                        ok = self.conn.change_property32(PropMode::REPLACE, r.requestor, property, incr, &[data.len() as u32]).is_ok();
                        self.incr.retain(|x| x.started.elapsed() < Duration::from_secs(30));
                        self.incr.push(IncrSend { requestor: r.requestor, property, kind: *kind, data: data.clone(), offset: 0, started: Instant::now() });
                    } else {
                        ok = self.conn.change_property8(PropMode::REPLACE, r.requestor, property, *kind, data).is_ok();
                    }
                }
            }
        }
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
            // 自己放的：直接按第一种类型解析
            let (a, d) = self.owned.as_ref()?.first()?.clone();
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
