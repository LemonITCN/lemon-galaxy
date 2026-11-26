//! GNOME Wayland：Mutter 自己的远程桌面接口（gnome-remote-desktop 也用它们），在用户的会话总线上，不弹授权，无人值守可用。
//!
//! - org.gnome.Mutter.RemoteDesktop.CreateSession → 远程桌面会话（键鼠、剪贴板）；
//! - org.gnome.Mutter.ScreenCast.CreateSession（带 remote-desktop-session-id）→ 绑定的录屏会话，每块屏 RecordMonitor 一路流；
//! - 会话 Start 后每路流发 PipeWireStreamAdded(节点号)，画面经 PipeWire 收（pipewire.rs）；
//! - 光标用元数据模式（cursor-mode=2）：位置每帧都有，画不画进画面由我们决定。
//! - 键鼠坐标是流里的像素（Mutter 自己按缩放换算）。
//! - 输入文字：经 IBus 输入法引擎提交（ibus.rs），没有 IBus 时按 keysym（布局里没有的字符会被 Mutter 丢掉）。
//!
//! 一个画面代理一个会话，开始时把所有屏都录上（RecordMonitor 只能在 Start 前调）；屏幕增删后整个会话重建（代理报 Lost，服务重启代理）。

use super::super::backend::{Backend, Capturer, Controller, CursorImage, CursorPos, Frame, Monitor};
use super::super::clipfmt::{self, LazyFiles};
use super::super::hidmap::{self, WheelAcc};
use super::pipewire::PwStream;
use galaxy_proto::peer::{ClipboardData, InputEvent, SpecialCombo, TouchPhase};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Duration;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const RD_BUS: &str = "org.gnome.Mutter.RemoteDesktop";
const RD_PATH: &str = "/org/gnome/Mutter/RemoteDesktop";
const RD_SESSION: &str = "org.gnome.Mutter.RemoteDesktop.Session";
const SC_BUS: &str = "org.gnome.Mutter.ScreenCast";
const SC_PATH: &str = "/org/gnome/Mutter/ScreenCast";
const SC_SESSION: &str = "org.gnome.Mutter.ScreenCast.Session";
const SC_STREAM: &str = "org.gnome.Mutter.ScreenCast.Stream";
const DC_BUS: &str = "org.gnome.Mutter.DisplayConfig";
const DC_PATH: &str = "/org/gnome/Mutter/DisplayConfig";

type MonitorSpec = (String, String, String, String);
type Mode = (String, i32, i32, f64, f64, Vec<f64>, HashMap<String, OwnedValue>);
type PhysMonitor = (MonitorSpec, Vec<Mode>, HashMap<String, OwnedValue>);
type LogicalMonitor = (i32, i32, f64, u32, bool, Vec<MonitorSpec>, HashMap<String, OwnedValue>);
type State = (u32, Vec<PhysMonitor>, Vec<LogicalMonitor>, HashMap<String, OwnedValue>);

fn e(x: impl std::fmt::Display) -> String {
    x.to_string()
}

/// 当前屏幕：(Monitor, 接口名)
fn current_monitors(conn: &Connection, is_virtual: bool) -> Result<Vec<(Monitor, String)>, String> {
    let p = Proxy::new(conn, DC_BUS, DC_PATH, DC_BUS).map_err(e)?;
    let (_, phys, logical, _): State = p.call("GetCurrentState", &()).map_err(|x| format!("读显示配置失败：{x}"))?;
    let mut v = vec![];
    for (x, y, scale, transform, primary, specs, _) in logical {
        let Some(spec) = specs.first() else { continue };
        let Some((_, modes, props)) = phys.iter().find(|(s, _, _)| s.0 == spec.0) else { continue };
        let cur = modes.iter().find(|m| m.6.get("is-current").and_then(|v| bool::try_from(v).ok()).unwrap_or(false)).or(modes.first());
        let Some(m) = cur else { continue };
        let (mut w, mut h) = (m.1.max(1) as u32, m.2.max(1) as u32);
        // 转 90 / 270 度时宽高对调
        if transform % 2 == 1 {
            std::mem::swap(&mut w, &mut h);
        }
        let name = props.get("display-name").and_then(|v| <&str>::try_from(v).ok().map(|s| s.to_string())).unwrap_or_else(|| spec.0.clone());
        v.push((Monitor { id: 0, name: if is_virtual { "虚拟桌面".into() } else { name }, x, y, width: w, height: h, primary, scale: scale as f32, is_virtual }, spec.0.clone()));
    }
    // 编号规则与 backend::assign_ids 相同：主屏 0，其余按位置
    v.sort_by_key(|(m, _)| (!m.primary, m.x, m.y));
    if !v.iter().any(|(m, _)| m.primary) {
        if let Some((m, _)) = v.first_mut() {
            m.primary = true;
        }
    }
    for (i, (m, _)) in v.iter_mut().enumerate() {
        m.id = i as u32;
    }
    Ok(v)
}

struct StreamInfo {
    monitor: Monitor,
    path: OwnedObjectPath,
    node: u32,
}

pub struct Session {
    conn: Connection,
    rd: OwnedObjectPath,
    streams: Vec<StreamInfo>,
    clip: Mutex<ClipState>,
}

/// 我们放进剪贴板的内容
#[derive(Clone)]
enum Owned {
    Data(Vec<(String, Vec<u8>)>),
    /// 有人粘贴时才准备的文件列表（没有 FUSE 时的兜底）
    Lazy(Arc<dyn LazyFiles>),
}

impl Owned {
    fn types(&self) -> Vec<String> {
        match self {
            Owned::Data(v) => v.iter().map(|(m, _)| m.clone()).collect(),
            Owned::Lazy(_) => clipfmt::LAZY_TYPES.iter().map(|m| m.to_string()).collect(),
        }
    }
}

#[derive(Default)]
struct ClipState {
    enabled: bool,
    /// 当前剪贴板里有的类型（别的程序放的）
    available: Vec<String>,
    /// 我们放的内容
    owned: Option<Owned>,
}

impl Session {
    fn rd_proxy(&self) -> Result<Proxy<'_>, String> {
        Proxy::new(&self.conn, RD_BUS, self.rd.as_str(), RD_SESSION).map_err(e)
    }

    fn start(is_virtual: bool) -> Result<Arc<Self>, String> {
        let conn = Connection::session().map_err(|x| format!("连不上会话总线：{x}"))?;
        let monitors = current_monitors(&conn, is_virtual)?;
        if monitors.is_empty() {
            return Err("没有屏幕".into());
        }
        let rd_root = Proxy::new(&conn, RD_BUS, RD_PATH, RD_BUS).map_err(e)?;
        let rd: OwnedObjectPath = rd_root.call("CreateSession", &()).map_err(|x| format!("Mutter 不允许远程桌面：{x}"))?;
        let rd_s = Proxy::new(&conn, RD_BUS, rd.as_str().to_string(), RD_SESSION).map_err(e)?;
        let sid: String = rd_s.get_property("SessionId").map_err(e)?;
        let sc_root = Proxy::new(&conn, SC_BUS, SC_PATH, SC_BUS).map_err(e)?;
        let mut opts: HashMap<&str, Value> = HashMap::new();
        opts.insert("remote-desktop-session-id", Value::from(sid.as_str()));
        let sc: OwnedObjectPath = sc_root.call("CreateSession", &(opts,)).map_err(|x| format!("Mutter 不允许录屏：{x}"))?;
        let sc_s = Proxy::new(&conn, SC_BUS, sc.as_str().to_string(), SC_SESSION).map_err(e)?;
        // 每块屏一路流；先订阅「流已加入 PipeWire」信号再 Start
        let (tx, rx) = std::sync::mpsc::channel::<(String, u32)>();
        let mut pending = vec![];
        for (m, connector) in monitors {
            let mut o: HashMap<&str, Value> = HashMap::new();
            o.insert("cursor-mode", Value::from(2u32));
            let path: OwnedObjectPath = sc_s.call("RecordMonitor", &(connector.as_str(), o)).map_err(|x| format!("录屏 {connector} 失败：{x}"))?;
            let sp = Proxy::new(&conn, SC_BUS, path.as_str().to_string(), SC_STREAM).map_err(e)?;
            let it = sp.receive_signal("PipeWireStreamAdded").map_err(e)?;
            let (tx, key) = (tx.clone(), path.as_str().to_string());
            std::thread::spawn(move || {
                for msg in it {
                    if let Ok((node,)) = msg.body().deserialize::<(u32,)>() {
                        let _ = tx.send((key.clone(), node));
                        break;
                    }
                }
            });
            pending.push((m, path));
        }
        rd_s.call::<_, _, ()>("Start", &()).map_err(|x| format!("启动远程桌面会话失败：{x}"))?;
        let mut nodes: HashMap<String, u32> = HashMap::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        while nodes.len() < pending.len() {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match rx.recv_timeout(left) {
                Ok((k, n)) => {
                    nodes.insert(k, n);
                }
                Err(_) => return Err("Mutter 没有给出画面流（PipeWire 没在运行？）".into()),
            }
        }
        let streams = pending.into_iter().filter_map(|(monitor, path)| nodes.get(path.as_str()).map(|n| StreamInfo { monitor, node: *n, path })).collect();
        let me = Arc::new(Self { conn, rd, streams, clip: Mutex::new(ClipState::default()) });
        // 会话被关（用户注销、Mutter 重启）：退出，服务会重找会话
        {
            let rd_s = Proxy::new(&me.conn, RD_BUS, me.rd.as_str().to_string(), RD_SESSION).map_err(e)?;
            if let Ok(it) = rd_s.receive_signal("Closed") {
                std::thread::spawn(move || {
                    if it.into_iter().next().is_some() {
                        eprintln!("Mutter 远程桌面会话被关闭");
                        super::super::agent::exit(3);
                    }
                });
            }
        }
        Ok(me)
    }

    fn stream_for(&self, m: &Monitor) -> Option<&StreamInfo> {
        self.streams.iter().find(|s| s.monitor.id == m.id).or_else(|| self.streams.first())
    }

    // ---------------- 剪贴板 ----------------

    fn enable_clipboard(self: &Arc<Self>) -> Result<(), String> {
        if self.clip.lock().enabled {
            return Ok(());
        }
        let p = self.rd_proxy()?;
        let o: HashMap<&str, Value> = HashMap::new();
        p.call::<_, _, ()>("EnableClipboard", &(o,)).map_err(|x| format!("Mutter 剪贴板不可用：{x}"))?;
        self.clip.lock().enabled = true;
        // 别的程序换了剪贴板：记下有哪些类型
        if let Ok(it) = p.receive_signal("SelectionOwnerChanged") {
            let me = Arc::downgrade(self);
            std::thread::spawn(move || {
                for msg in it {
                    let Some(me) = me.upgrade() else { break };
                    if let Ok((o,)) = msg.body().deserialize::<(HashMap<String, OwnedValue>,)>() {
                        let ours = o.get("session-is-owner").and_then(|v| bool::try_from(v).ok()).unwrap_or(false);
                        let types: Vec<String> = o.get("mime-types").and_then(|v| Vec::<String>::try_from(v.try_clone().ok()?).ok()).unwrap_or_default();
                        let mut g = me.clip.lock();
                        if !ours {
                            g.owned = None;
                            g.available = types;
                        }
                    }
                }
            });
        }
        // 别的程序要我们放的内容
        if let Ok(it) = p.receive_signal("SelectionTransfer") {
            let me = Arc::downgrade(self);
            std::thread::spawn(move || {
                for msg in it {
                    let Some(me) = me.upgrade() else { break };
                    let Ok((mime, serial)) = msg.body().deserialize::<(String, u32)>() else { continue };
                    let owned = me.clip.lock().owned.clone();
                    match owned {
                        Some(Owned::Data(v)) => {
                            let ok = match v.into_iter().find(|(m, _)| *m == mime) {
                                Some((_, d)) => me.selection_write(serial).map(|mut f| f.write_all(&d).is_ok()).unwrap_or(false),
                                None => false,
                            };
                            me.write_done(serial, ok);
                        }
                        Some(Owned::Lazy(files)) if clipfmt::LAZY_TYPES.contains(&mime.as_str()) => {
                            // 先要到写入端（Mutter 过一阵没人写就作废），再等文件准备好（可能要下载一阵），不卡后面的请求
                            std::thread::spawn(move || {
                                let ok = match me.selection_write(serial) {
                                    Some(mut f) => files.paths().and_then(|p| clipfmt::files_data(&mime, p)).map(|d| f.write_all(&d).is_ok()).unwrap_or(false),
                                    None => false,
                                };
                                me.write_done(serial, ok);
                            });
                        }
                        _ => me.write_done(serial, false),
                    }
                }
            });
        }
        Ok(())
    }

    fn selection_write(&self, serial: u32) -> Option<std::fs::File> {
        let fd: zbus::zvariant::OwnedFd = self.rd_proxy().ok()?.call("SelectionWrite", &(serial,)).ok()?;
        Some(std::fs::File::from(std::os::fd::OwnedFd::from(fd)))
    }

    fn write_done(&self, serial: u32, ok: bool) {
        if let Ok(p) = self.rd_proxy() {
            let _ = p.call::<_, _, ()>("SelectionWriteDone", &(serial, ok));
        }
    }

    fn clipboard_get(self: &Arc<Self>) -> Option<ClipboardData> {
        self.enable_clipboard().ok()?;
        let (owned, available) = {
            let g = self.clip.lock();
            (g.owned.clone(), g.available.clone())
        };
        match owned {
            // 粘贴时才准备的文件列表是主控端复制的，不能传回去
            Some(Owned::Lazy(_)) => return None,
            Some(Owned::Data(o)) => {
                let (m, d) = o.first()?.clone();
                return clipfmt::parse(&m, d);
            }
            None => {}
        }
        let want = clipfmt::pick(&available).unwrap_or_else(|| "text/plain;charset=utf-8".into());
        let p = self.rd_proxy().ok()?;
        let fd: zbus::zvariant::OwnedFd = p.call("SelectionRead", &(want.as_str(),)).ok()?;
        let fd = std::os::fd::OwnedFd::from(fd);
        let data = read_with_timeout(fd, Duration::from_secs(3))?;
        clipfmt::parse(&want, data)
    }

    /// 我们当剪贴板的主人，放上 `owned`
    fn set_owned(&self, owned: Owned) -> Result<(), String> {
        let types = owned.types();
        let types: Vec<&str> = types.iter().map(|m| m.as_str()).collect();
        let mut o: HashMap<&str, Value> = HashMap::new();
        o.insert("mime-types", Value::from(types));
        self.rd_proxy()?.call::<_, _, ()>("SetSelection", &(o,)).map_err(|x| format!("设置剪贴板失败：{x}"))?;
        self.clip.lock().owned = Some(owned);
        Ok(())
    }

    fn clipboard_set(self: &Arc<Self>, d: ClipboardData) {
        if self.enable_clipboard().is_err() {
            return;
        }
        // Mutter 只认 MIME 类型名（不认 X11 的 UTF8_STRING 之类）
        let offers: Vec<(String, Vec<u8>)> = clipfmt::offers(&d).into_iter().filter(|(m, _)| m.contains('/')).collect();
        let _ = self.set_owned(Owned::Data(offers));
    }

    fn clipboard_set_lazy(self: &Arc<Self>, files: Arc<dyn LazyFiles>) -> Result<(), String> {
        self.enable_clipboard()?;
        self.set_owned(Owned::Lazy(files))
    }
}

/// 读管道直到对方关闭，最多等 `t`
fn read_with_timeout(fd: std::os::fd::OwnedFd, t: Duration) -> Option<Vec<u8>> {
    use std::os::fd::AsRawFd;
    let deadline = std::time::Instant::now() + t;
    let mut f = std::fs::File::from(fd);
    let mut out = vec![];
    let mut buf = [0u8; 65536];
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return if out.is_empty() { None } else { Some(out) };
        }
        let mut pfd = libc::pollfd { fd: f.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        if unsafe { libc::poll(&mut pfd, 1, left.as_millis() as i32) } <= 0 {
            continue;
        }
        match f.read(&mut buf) {
            Ok(0) => return Some(out),
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(_) => return None,
        }
        if out.len() > 64 * 1024 * 1024 {
            return None;
        }
    }
}

pub struct MutterBackend {
    session: Arc<Session>,
}

impl MutterBackend {
    pub fn new(is_virtual: bool) -> Result<Self, String> {
        Ok(Self { session: Session::start(is_virtual)? })
    }
}

impl Backend for MutterBackend {
    fn name(&self) -> &'static str {
        "mutter"
    }

    fn monitors(&self) -> Vec<Monitor> {
        self.session.streams.iter().map(|s| s.monitor.clone()).collect()
    }

    fn capturer(&self, m: &Monitor) -> Result<Box<dyn Capturer>, String> {
        let s = self.session.stream_for(m).ok_or("这块屏没有画面流")?;
        let pw = PwStream::connect(s.node, None, 60)?;
        Ok(Box::new(MutterCapturer { pw, mon: s.monitor.clone(), _session: self.session.clone() }))
    }

    fn controller(&self) -> Result<Box<dyn Controller>, String> {
        Ok(Box::new(MutterController { session: self.session.clone(), wheel: WheelAcc::default(), touch_down: false, last_pos: None }))
    }
}

pub struct MutterCapturer {
    pw: PwStream,
    mon: Monitor,
    _session: Arc<Session>,
}

impl Capturer for MutterCapturer {
    fn next(&mut self, frame: &mut Frame, timeout: Duration) -> Result<bool, String> {
        self.pw.next(frame, timeout)
    }
    fn size(&self) -> (u32, u32) {
        match self.pw.size() {
            (0, _) | (_, 0) => (self.mon.width, self.mon.height),
            s => s,
        }
    }
    fn cursor(&mut self) -> Option<(CursorPos, Option<CursorImage>)> {
        let (w, h) = self.size();
        self.pw.cursor().map(|(mut p, img)| {
            p.visible = p.visible && p.x >= 0 && p.y >= 0 && p.x < w as i32 && p.y < h as i32;
            (p, img)
        })
    }
    fn kind(&self) -> &'static str {
        "mutter-pipewire"
    }
}

pub struct MutterController {
    session: Arc<Session>,
    wheel: WheelAcc,
    touch_down: bool,
    /// 上次发出的指针位置（屏幕编号、画面像素坐标）
    last_pos: Option<(u32, i32, i32)>,
}

impl MutterController {
    fn call<B: serde::Serialize + zbus::zvariant::DynamicType>(&self, method: &str, body: &B) {
        if let Ok(p) = self.session.rd_proxy() {
            let _ = p.call_noreply(method, body);
        }
    }

    /// 指针移动：先发一次相对移动（和上次位置的差），再发绝对位置定准。
    /// 平时相对移动已经到位，绝对位置不再挪；但有程序锁定 / 限定指针时（VMware 点进虚拟机后抓住鼠标、
    /// 浏览器游戏的指针锁定），Mutter 会把虚拟设备的绝对坐标夹在窗口里、且不产生相对移动事件，
    /// 这些程序就收不到任何移动——VMware 里的指针不动、走不到边缘、放不了手，只能 Ctrl+Alt。
    /// 发相对移动后它们和用真鼠标一样收到移动。换屏后第一次不发（不同屏的坐标不能相减）。
    fn motion(&mut self, m: &Monitor, x: i32, y: i32) {
        let (x, y) = (x.max(0), y.max(0));
        if let Some(s) = self.session.stream_for(m) {
            if let Some((id, lx, ly)) = self.last_pos {
                if id == m.id && (x, y) != (lx, ly) {
                    self.call("NotifyPointerMotionRelative", &((x - lx) as f64, (y - ly) as f64));
                }
            }
            self.call("NotifyPointerMotionAbsolute", &(s.path.as_str(), x as f64, y as f64));
            self.last_pos = Some((m.id, x, y));
        }
    }

    fn key(&self, evdev: u32, down: bool) {
        super::super::ibus::before_key();
        self.call("NotifyKeyboardKeycode", &(evdev, down));
        super::super::ibus::after_key();
    }

    fn chord(&self, keys: &[u32]) {
        for k in keys {
            self.key(*k, true);
        }
        for k in keys.iter().rev() {
            self.key(*k, false);
        }
    }
}

impl Controller for MutterController {
    fn inject(&mut self, m: &Monitor, ev: InputEvent) {
        match ev {
            InputEvent::MouseMove { x, y } => self.motion(m, x, y),
            InputEvent::MouseButton { button, down, x, y } => {
                self.motion(m, x, y);
                self.call("NotifyPointerButton", &(hidmap::button(button) as i32, down));
            }
            InputEvent::Wheel { dx, dy } => {
                let (sx, sy) = self.wheel.add(dx, dy);
                // Mutter：步数正为向下 / 向右；协议正 dy 向上、正 dx 向左
                if sy != 0 {
                    self.call("NotifyPointerAxisDiscrete", &(0u32, -sy));
                }
                if sx != 0 {
                    self.call("NotifyPointerAxisDiscrete", &(1u32, -sx));
                }
            }
            InputEvent::Key { hid_usage, down } => {
                if let Some(k) = hidmap::evdev(hid_usage) {
                    self.key(k, down);
                }
            }
            InputEvent::Text { text } => {
                // 键盘上有的字符按按键（虚拟机等只收按键的程序也能收到）；中日韩等经 IBus 提交，
                // 没有 IBus 时也按 keysym（布局里没有的字符会被丢掉）。见 ibus::text_runs
                for (keys, run) in super::super::ibus::text_runs(&text) {
                    if keys || !super::super::ibus::commit(&run) {
                        for c in run.chars() {
                            match c {
                                '\n' | '\r' => self.chord(&[hidmap::KEY_ENTER]),
                                '\t' => self.chord(&[hidmap::KEY_TAB]),
                                _ => {
                                    let sym = super::super::x11::keysym_for(c);
                                    super::super::ibus::before_key();
                                    self.call("NotifyKeyboardKeysym", &(sym, true));
                                    self.call("NotifyKeyboardKeysym", &(sym, false));
                                    super::super::ibus::after_key();
                                }
                            }
                        }
                    }
                }
            }
            InputEvent::Touch { phase, x, y, .. } => match phase {
                TouchPhase::Began => {
                    self.motion(m, x, y);
                    self.call("NotifyPointerButton", &(hidmap::BTN_LEFT as i32, true));
                    self.touch_down = true;
                }
                TouchPhase::Moved => self.motion(m, x, y),
                TouchPhase::Ended | TouchPhase::Cancelled => {
                    if std::mem::take(&mut self.touch_down) {
                        self.motion(m, x, y);
                        self.call("NotifyPointerButton", &(hidmap::BTN_LEFT as i32, false));
                    }
                }
            },
            InputEvent::SpecialCombo(c) => {
                use hidmap::*;
                match c {
                    SpecialCombo::CtrlAltDel => self.chord(&[KEY_LEFTCTRL, KEY_LEFTALT, KEY_DELETE]),
                    SpecialCombo::LockScreen => super::super::actions::lock_screen(),
                    SpecialCombo::AltTab => self.chord(&[KEY_LEFTALT, KEY_TAB]),
                    SpecialCombo::WinKey => self.chord(&[KEY_LEFTMETA]),
                    SpecialCombo::TaskManager => {
                        if !super::super::actions::task_manager() {
                            self.chord(&[KEY_LEFTCTRL, KEY_LEFTALT, KEY_DELETE]);
                        }
                    }
                    SpecialCombo::ShowDesktop => self.chord(&[KEY_LEFTMETA, KEY_D]),
                }
            }
        }
    }

    fn clipboard_get(&mut self) -> Option<ClipboardData> {
        self.session.clipboard_get()
    }

    fn clipboard_set(&mut self, d: ClipboardData) {
        self.session.clipboard_set(d)
    }

    fn clipboard_set_lazy(&mut self, files: Arc<dyn LazyFiles>) -> Result<(), String> {
        self.session.clipboard_set_lazy(files)
    }
}
