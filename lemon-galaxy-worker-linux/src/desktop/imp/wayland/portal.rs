//! 其他 Wayland 桌面（KDE Plasma 等）：xdg-desktop-portal 的 RemoteDesktop + ScreenCast。
//!
//! 第一次必须有人在被控机上点「允许」（桌面门户的规定，没法绕过）；我们要 persist_mode=2，拿到的恢复令牌存在
//! ~/.config/lemon-galaxy/portal-restore-token，之后再连就不弹了（令牌失效时会再弹一次）。
//! 画面：OpenPipeWireRemote 给的连接 + 每块屏一路流；光标嵌在画面里。剪贴板暂不支持。
//!
//! 等用户点允许可能要很久：会话在后台线程里建，列屏幕时最多等 5 秒，没好就先返回空列表并在日志里说明。

use super::super::backend::{Backend, Capturer, Controller, CursorImage, CursorPos, Frame, Monitor};
use super::super::hidmap::{self, WheelAcc};
use super::pipewire::PwStream;
use galaxy_proto::peer::{ClipboardData, InputEvent, SpecialCombo, TouchPhase};
use parking_lot::{Condvar, Mutex};
use std::collections::HashMap;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::Arc;
use std::time::{Duration, Instant};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const BUS: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const RD: &str = "org.freedesktop.portal.RemoteDesktop";
const SC: &str = "org.freedesktop.portal.ScreenCast";

fn token_file() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"));
    base.join("lemon-galaxy").join("portal-restore-token")
}

struct Ready {
    session: OwnedObjectPath,
    /// (屏幕, 节点号, 逻辑尺寸)
    streams: Vec<(Monitor, u32, (i32, i32))>,
    remote: OwnedFd,
}

struct State {
    ready: Option<Arc<Ready>>,
    error: Option<String>,
}

struct Shared {
    conn: Connection,
    state: Mutex<State>,
    cv: Condvar,
    /// 各节点画面的实际像素尺寸（键鼠坐标要从画面像素换成逻辑坐标）
    frame_sizes: Mutex<HashMap<u32, (u32, u32)>>,
}

pub struct PortalBackend {
    shared: Arc<Shared>,
}

fn sender_path(conn: &Connection) -> String {
    conn.unique_name().map(|n| n.as_str().trim_start_matches(':').replace('.', "_")).unwrap_or_default()
}

fn new_token() -> String {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    format!("lg{}_{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}

/// 调一个返回 Request 的门户方法，等 Response。`call` 收到 handle_token 返回实际调用。
fn request(conn: &Connection, timeout: Duration, call: impl FnOnce(&str) -> zbus::Result<OwnedObjectPath>) -> Result<HashMap<String, OwnedValue>, String> {
    let token = new_token();
    let path = format!("/org/freedesktop/portal/desktop/request/{}/{token}", sender_path(conn));
    let rp = Proxy::new(conn, BUS, path, "org.freedesktop.portal.Request").map_err(|e| e.to_string())?;
    let it = rp.receive_signal("Response").map_err(|e| e.to_string())?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for msg in it {
            if let Ok(r) = msg.body().deserialize::<(u32, HashMap<String, OwnedValue>)>() {
                let _ = tx.send(r);
                break;
            }
        }
    });
    call(&token).map_err(|e| format!("桌面门户调用失败：{e}"))?;
    let (code, results) = rx.recv_timeout(timeout).map_err(|_| "等桌面门户回应超时（被控机上没人点「允许」？）".to_string())?;
    match code {
        0 => Ok(results),
        1 => Err("被控机上的人拒绝了屏幕共享".into()),
        _ => Err("桌面门户拒绝了请求".into()),
    }
}

fn opts<'a>(token: &'a str, extra: Vec<(&'a str, Value<'a>)>) -> HashMap<&'a str, Value<'a>> {
    let mut m: HashMap<&str, Value> = extra.into_iter().collect();
    m.insert("handle_token", Value::from(token));
    m
}

fn establish(conn: &Connection) -> Result<Ready, String> {
    let rd = Proxy::new(conn, BUS, PATH, RD).map_err(|e| e.to_string())?;
    let sc = Proxy::new(conn, BUS, PATH, SC).map_err(|e| e.to_string())?;
    let st = new_token();
    let r = request(conn, Duration::from_secs(10), |t| rd.call("CreateSession", &(opts(t, vec![("session_handle_token", Value::from(st.as_str()))]),)))?;
    let session_s: String = r.get("session_handle").and_then(|v| <&str>::try_from(v).ok().map(|s| s.to_string())).ok_or("门户没有给会话")?;
    let session = OwnedObjectPath::try_from(session_s).map_err(|e| e.to_string())?;
    let restore = std::fs::read_to_string(token_file()).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    request(conn, Duration::from_secs(10), |t| {
        let mut extra = vec![("types", Value::from(3u32)), ("persist_mode", Value::from(2u32))];
        if let Some(r) = restore.as_deref() {
            extra.push(("restore_token", Value::from(r)));
        }
        rd.call("SelectDevices", &(&session, opts(t, extra)))
    })?;
    request(conn, Duration::from_secs(10), |t| sc.call("SelectSources", &(&session, opts(t, vec![("types", Value::from(1u32)), ("multiple", Value::from(true)), ("cursor_mode", Value::from(2u32))]))))?;
    let r = request(conn, Duration::from_secs(300), |t| rd.call("Start", &(&session, "", opts(t, vec![]))))?;
    if let Some(tok) = r.get("restore_token").and_then(|v| <&str>::try_from(v).ok()) {
        let f = token_file();
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&f, tok);
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600));
    }
    let raw: Vec<(u32, HashMap<String, OwnedValue>)> = r.get("streams").and_then(|v| v.try_clone().ok()).and_then(|v| v.try_into().ok()).ok_or("门户没有给画面流")?;
    let mut streams = vec![];
    for (i, (node, props)) in raw.into_iter().enumerate() {
        let pos: (i32, i32) = props.get("position").and_then(|v| v.try_clone().ok()).and_then(|v| v.try_into().ok()).unwrap_or((0, 0));
        let size: (i32, i32) = props.get("size").and_then(|v| v.try_clone().ok()).and_then(|v| v.try_into().ok()).unwrap_or((1920, 1080));
        let m = Monitor { id: i as u32, name: format!("屏幕 {}", i + 1), x: pos.0, y: pos.1, width: size.0.max(1) as u32, height: size.1.max(1) as u32, primary: i == 0, scale: 1.0, is_virtual: false };
        streams.push((m, node, size));
    }
    let fd: zbus::zvariant::OwnedFd = sc.call("OpenPipeWireRemote", &(&session, HashMap::<&str, Value>::new())).map_err(|e| format!("打开 PipeWire 连接失败：{e}"))?;
    Ok(Ready { session, streams, remote: OwnedFd::from(fd) })
}

impl PortalBackend {
    pub fn new() -> Result<Self, String> {
        let conn = Connection::session().map_err(|e| format!("连不上会话总线：{e}"))?;
        // 门户在不在
        Proxy::new(&conn, BUS, PATH, RD).and_then(|p| p.get_property::<u32>("AvailableDeviceTypes")).map_err(|e| format!("没有桌面门户的远程桌面接口（xdg-desktop-portal 太旧或没装）：{e}"))?;
        let shared = Arc::new(Shared { conn, state: Mutex::new(State { ready: None, error: None }), cv: Condvar::new(), frame_sizes: Mutex::new(HashMap::new()) });
        let s = shared.clone();
        std::thread::spawn(move || {
            let r = establish(&s.conn);
            let mut g = s.state.lock();
            match r {
                Ok(x) => g.ready = Some(Arc::new(x)),
                Err(e) => {
                    eprintln!("桌面门户：{e}");
                    g.error = Some(e);
                }
            }
            s.cv.notify_all();
        });
        Ok(Self { shared })
    }

    fn ready(&self, wait: Duration) -> Result<Arc<Ready>, String> {
        let mut g = self.shared.state.lock();
        let deadline = Instant::now() + wait;
        while g.ready.is_none() && g.error.is_none() {
            if self.shared.cv.wait_until(&mut g, deadline).timed_out() {
                break;
            }
        }
        if let Some(r) = &g.ready {
            return Ok(r.clone());
        }
        Err(g.error.clone().unwrap_or_else(|| "等被控机上的人在弹窗里点「允许」共享屏幕".into()))
    }
}

impl Backend for PortalBackend {
    fn name(&self) -> &'static str {
        "portal"
    }

    fn monitors(&self) -> Vec<Monitor> {
        match self.ready(Duration::from_secs(5)) {
            Ok(r) => r.streams.iter().map(|(m, _, _)| m.clone()).collect(),
            Err(e) => {
                eprintln!("{e}");
                vec![]
            }
        }
    }

    fn capturer(&self, m: &Monitor) -> Result<Box<dyn Capturer>, String> {
        let r = self.ready(Duration::from_secs(10))?;
        let (mon, node, _) = r.streams.iter().find(|(x, _, _)| x.id == m.id).or(r.streams.first()).cloned().ok_or("没有画面流")?;
        let dup = unsafe { libc::dup(r.remote.as_raw_fd()) };
        if dup < 0 {
            return Err("复制 PipeWire 连接失败".into());
        }
        let pw = PwStream::connect(node, Some(unsafe { OwnedFd::from_raw_fd(dup) }), 60)?;
        Ok(Box::new(PortalCapturer { pw, mon, node, shared: self.shared.clone() }))
    }

    fn controller(&self) -> Result<Box<dyn Controller>, String> {
        let r = self.ready(Duration::from_secs(10))?;
        Ok(Box::new(PortalController { ready: r, shared: self.shared.clone(), wheel: WheelAcc::default(), touch_down: false, last_pos: None }))
    }
}

pub struct PortalCapturer {
    pw: PwStream,
    mon: Monitor,
    node: u32,
    shared: Arc<Shared>,
}

impl Capturer for PortalCapturer {
    fn next(&mut self, frame: &mut Frame, timeout: Duration) -> Result<bool, String> {
        let r = self.pw.next(frame, timeout)?;
        if r {
            self.shared.frame_sizes.lock().insert(self.node, (frame.width, frame.height));
        }
        Ok(r)
    }
    fn size(&self) -> (u32, u32) {
        match self.pw.size() {
            (0, _) | (_, 0) => (self.mon.width, self.mon.height),
            s => s,
        }
    }
    fn cursor(&mut self) -> Option<(CursorPos, Option<CursorImage>)> {
        None
    }
    fn cursor_embedded(&self) -> bool {
        true
    }
    fn kind(&self) -> &'static str {
        "portal-pipewire"
    }
}

pub struct PortalController {
    ready: Arc<Ready>,
    shared: Arc<Shared>,
    wheel: WheelAcc,
    touch_down: bool,
    /// 上次发出的指针位置（流节点、逻辑坐标）
    last_pos: Option<(u32, f64, f64)>,
}

impl PortalController {
    fn call<B: serde::Serialize + zbus::zvariant::DynamicType>(&self, method: &str, body: &B) {
        if let Ok(p) = Proxy::new(&self.shared.conn, BUS, PATH, RD) {
            let _ = p.call_noreply(method, body);
        }
    }

    fn empty() -> HashMap<&'static str, Value<'static>> {
        HashMap::new()
    }

    /// 指针移动：先发一次相对移动（和上次位置的差），再发绝对位置定准。
    /// 平时相对移动已经到位，绝对位置不再挪；但有程序锁定 / 限定指针时（VMware 点进虚拟机后抓住鼠标、
    /// 浏览器游戏的指针锁定），Mutter 会把虚拟设备的绝对坐标夹在窗口里、且不产生相对移动事件，
    /// 这些程序就收不到任何移动——VMware 里的指针不动、走不到边缘、放不了手，只能 Ctrl+Alt。
    /// 发相对移动后它们和用真鼠标一样收到移动。换屏后第一次不发（不同屏的坐标不能相减）。
    fn motion(&mut self, m: &Monitor, x: i32, y: i32) {
        let Some((_, node, logical)) = self.ready.streams.iter().find(|(s, _, _)| s.id == m.id).or(self.ready.streams.first()).cloned() else { return };
        // 画面像素 → 流的逻辑坐标（缩放显示时两者不同）
        let (fw, fh) = self.shared.frame_sizes.lock().get(&node).copied().unwrap_or((logical.0.max(1) as u32, logical.1.max(1) as u32));
        let lx = x.max(0) as f64 * logical.0 as f64 / fw.max(1) as f64;
        let ly = y.max(0) as f64 * logical.1 as f64 / fh.max(1) as f64;
        if let Some((n, px, py)) = self.last_pos {
            if n == node && (lx, ly) != (px, py) {
                self.call("NotifyPointerMotion", &(&self.ready.session, Self::empty(), lx - px, ly - py));
            }
        }
        self.call("NotifyPointerMotionAbsolute", &(&self.ready.session, Self::empty(), node, lx, ly));
        self.last_pos = Some((node, lx, ly));
    }

    fn button(&self, code: u32, down: bool) {
        self.call("NotifyPointerButton", &(&self.ready.session, Self::empty(), code as i32, down as u32));
    }

    fn key(&self, evdev: u32, down: bool) {
        self.call("NotifyKeyboardKeycode", &(&self.ready.session, Self::empty(), evdev as i32, down as u32));
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

impl Controller for PortalController {
    fn inject(&mut self, m: &Monitor, ev: InputEvent) {
        match ev {
            InputEvent::MouseMove { x, y } => self.motion(m, x, y),
            InputEvent::MouseButton { button, down, x, y } => {
                self.motion(m, x, y);
                self.button(hidmap::button(button), down);
            }
            InputEvent::Wheel { dx, dy } => {
                let (sx, sy) = self.wheel.add(dx, dy);
                if sy != 0 {
                    self.call("NotifyPointerAxisDiscrete", &(&self.ready.session, Self::empty(), 0u32, -sy));
                }
                if sx != 0 {
                    self.call("NotifyPointerAxisDiscrete", &(&self.ready.session, Self::empty(), 1u32, -sx));
                }
            }
            InputEvent::Key { hid_usage, down } => {
                if let Some(k) = hidmap::evdev(hid_usage) {
                    self.key(k, down);
                }
            }
            InputEvent::Text { text } => {
                // 键盘上有的字符按按键（虚拟机等只收按键的程序也能收到）；中日韩等经 IBus 提交，
                // 没有 IBus（KDE 多用 fcitx5）时也按 keysym（布局里没有的字符会被丢掉）。见 ibus::text_runs
                for (keys, run) in super::super::ibus::text_runs(&text) {
                    if keys || !super::super::ibus::commit(&run) {
                        for c in run.chars() {
                            match c {
                                '\n' | '\r' => self.chord(&[hidmap::KEY_ENTER]),
                                '\t' => self.chord(&[hidmap::KEY_TAB]),
                                _ => {
                                    let sym = super::super::x11::keysym_for(c) as i32;
                                    self.call("NotifyKeyboardKeysym", &(&self.ready.session, Self::empty(), sym, 1u32));
                                    self.call("NotifyKeyboardKeysym", &(&self.ready.session, Self::empty(), sym, 0u32));
                                }
                            }
                        }
                    }
                }
            }
            InputEvent::Touch { phase, x, y, .. } => match phase {
                TouchPhase::Began => {
                    self.motion(m, x, y);
                    self.button(hidmap::BTN_LEFT, true);
                    self.touch_down = true;
                }
                TouchPhase::Moved => self.motion(m, x, y),
                TouchPhase::Ended | TouchPhase::Cancelled => {
                    if std::mem::take(&mut self.touch_down) {
                        self.motion(m, x, y);
                        self.button(hidmap::BTN_LEFT, false);
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
        None
    }

    fn clipboard_set(&mut self, _d: ClipboardData) {}

    fn clipboard_available(&self) -> bool {
        false
    }
}
