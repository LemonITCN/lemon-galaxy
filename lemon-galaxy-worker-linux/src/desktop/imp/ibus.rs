//! Wayland 下输入文字（主控端输入法提交的中文、日文，或粘贴的一段文字）：经 IBus 输入法引擎提交。
//!
//! Mutter / 门户的 NotifyKeyboardKeysym 只在当前键盘布局里找键码，找不到就静默丢掉（gnome-shell 日志
//! "No keycode found for keyval"），中日韩、é 之类全丢。做法同 Chrome 远程桌面的 Linux 主机：画面代理把自己注册成
//! 一个 IBus 引擎，有文字要输入时临时切成全局引擎、commit_text 提交，空闲约 300 毫秒后切回原来的引擎。
//!
//! - 库全部运行时加载（libibus-1.0.so.5 及它带的 GLib），程序不链接；单独一个线程跑 GLib 主循环处理 IBus。
//! - 切引擎必须异步：守护进程要回调我们的工厂 CreateEngine，同步调用会把自己锁住。我们的引擎不处理按键（原样放行）。
//! - 切回要确认：切完再读一次全局引擎核对，不对就重试，原引擎不行时依次用 GNOME 输入源设置对应的引擎、xkb:us::eng。
//! - 绝不能把对方的输入法弄没（我们的组件一消失、全局引擎还是我们的，IBus 就成了「没有引擎」）：
//!   代理退出、收到 SIGTERM、Mutter 会话关闭、出错时都先同步切回再退（最多约 1 秒）；
//!   另有一个看门进程（`__lg-ibus-guard`）：代理被 kill -9 或崩溃时由它切回。
//!   实测（IBus 1.5.27）：注册过组件的进程一退出，守护进程就把全局引擎清空（哪怕早已切回、甚至从没切过），
//!   所以看门进程在代理退出后都要看约 3 秒，引擎空了或还是我们的就设回最后一个真实引擎。
//! - 拿不到 IBus（没有库、没有守护进程、KDE 用 fcitx5 等）时调用方退回按 keysym，并记一条日志。

use super::encode::dl::Lib;
use std::ffi::{c_char, c_int, c_uint, c_ulong, c_void, CStr, CString};
use std::io::{BufRead, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{mpsc, Arc, OnceLock};
use std::time::{Duration, Instant};

/// 我们的引擎名
pub const OURS: &str = "lemongalaxy-remote";
const COMPONENT: &str = "cn.lemonit.galaxy.remoteinput";
/// 最后一段文字之后这么久没有新的就切回
const IDLE: Duration = Duration::from_millis(300);
/// 原引擎、GNOME 设置都拿不到时用它（GNOME 对所有 xkb 输入源也是让 IBus 用这个）
const DEFAULT_ENGINE: &str = "xkb:us::eng";

// ======================== 切换逻辑（与 IBus 无关，可单测） ========================

/// IBus 的几个操作（真的在 IBus 线程里实现；单测用假的）
pub trait Engines {
    /// 当前全局引擎名；没有引擎时是空串；连不上 IBus 时 None
    fn current(&mut self) -> Option<String>;
    /// 切全局引擎，等守护进程确认
    fn set(&mut self, name: &str) -> bool;
    /// 用我们的引擎提交文字
    fn commit(&mut self, text: &str) -> bool;
    /// GNOME 输入源设置里当前源对应的引擎
    fn fallback(&mut self) -> Option<String>;
    /// 告诉看门进程：现在切到了我们的引擎、原来是 `prev`（None：已切回）
    fn guard(&mut self, prev: Option<&str>);
    /// 重试前等一下
    fn wait(&mut self, d: Duration);
}

fn real(name: &str) -> bool {
    !name.is_empty() && name != OURS
}

/// 何时切到我们的引擎、何时切回、切回失败怎么办
#[derive(Default)]
pub struct Switcher {
    /// 切过来之前的引擎（Some 表示现在是我们的引擎）
    prev: Option<String>,
    /// 最近一次见到的真实引擎（读到的「原引擎」已经是我们自己时用它）
    last_real: Option<String>,
    /// 到这个时候还没有新文字就切回
    deadline: Option<Instant>,
}

impl Switcher {
    pub fn active(&self) -> bool {
        self.prev.is_some()
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// 提交一段文字（必要时先切到我们的引擎）。失败时立刻切回，返回 false（调用方退回按键）。
    pub fn commit<E: Engines>(&mut self, e: &mut E, text: &str, now: Instant) -> bool {
        if self.prev.is_none() {
            let prev = match e.current() {
                Some(c) if real(&c) => {
                    self.last_real = Some(c.clone());
                    c
                }
                // 没有引擎、或者上次没切回还是我们的：用记下的、GNOME 设置里的、xkb:us::eng
                _ => self.last_real.clone().or_else(|| e.fallback().filter(|f| real(f))).unwrap_or_else(|| DEFAULT_ENGINE.into()),
            };
            e.guard(Some(&prev));
            self.prev = Some(prev);
            if !e.set(OURS) {
                self.restore(e, now + Duration::from_secs(3));
                return false;
            }
        }
        self.deadline = Some(now + IDLE);
        if e.commit(text) {
            true
        } else {
            self.restore(e, now + Duration::from_secs(3));
            false
        }
    }

    /// 空闲到点了就切回；切不回 1 秒后再试
    pub fn tick<E: Engines>(&mut self, e: &mut E, now: Instant) {
        if self.prev.is_some() && self.deadline.map(|d| now >= d).unwrap_or(true) && !self.restore(e, now + Duration::from_secs(3)) {
            self.deadline = Some(now + Duration::from_secs(1));
        }
    }

    /// 切回原引擎并核对；原引擎不行时依次试 GNOME 设置里的、xkb:us::eng。每个最多 3 次，超过 `until` 不再试。
    pub fn restore<E: Engines>(&mut self, e: &mut E, until: Instant) -> bool {
        let Some(prev) = self.prev.clone() else { return true };
        let mut targets = vec![prev];
        let mut i = 0;
        while i < targets.len() {
            let t = targets[i].clone();
            for attempt in 0..3u32 {
                if Instant::now() > until {
                    return false;
                }
                if attempt > 0 {
                    e.wait(Duration::from_millis(100 * attempt as u64));
                }
                if e.set(&t) && e.current().as_deref() == Some(t.as_str()) {
                    self.prev = None;
                    self.deadline = None;
                    self.last_real = Some(t);
                    e.guard(None);
                    return true;
                }
            }
            if i == 0 {
                if let Some(f) = e.fallback().filter(|f| real(f) && !targets.contains(f)) {
                    targets.push(f);
                }
                if !targets.iter().any(|x| x == DEFAULT_ENGINE) {
                    targets.push(DEFAULT_ENGINE.into());
                }
            }
            i += 1;
        }
        false
    }
}

/// `gsettings get org.gnome.desktop.input-sources mru-sources / sources` 的输出里第 `index` 个源对应的 IBus 引擎：
/// ibus 源就是引擎名；xkb 源 GNOME 让 IBus 用 xkb:us::eng（键盘布局由 Mutter 管）
pub fn engine_from_sources(s: &str, index: usize) -> Option<String> {
    let mut rest = s;
    let mut k = 0;
    loop {
        let start = rest.find("('")?;
        let item = &rest[start + 2..];
        let (kind, after) = item.split_once("', '")?;
        let (id, tail) = after.split_once("')")?;
        if k == index {
            return match kind {
                "ibus" if !id.is_empty() => Some(id.to_string()),
                "xkb" => Some(DEFAULT_ENGINE.into()),
                _ => None,
            };
        }
        k += 1;
        rest = tail;
    }
}

fn gsettings(key: &str) -> String {
    Command::new("gsettings").args(["get", "org.gnome.desktop.input-sources", key]).stdin(Stdio::null()).stderr(Stdio::null()).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default()
}

/// GNOME 当前输入源对应的引擎（最近用过的第一个；没有时按 current 下标取 sources）
fn gnome_engine() -> Option<String> {
    if let Some(e) = engine_from_sources(&gsettings("mru-sources"), 0) {
        return Some(e);
    }
    let cur = gsettings("current").split_whitespace().last().and_then(|n| n.parse::<usize>().ok()).unwrap_or(0);
    let sources = gsettings("sources");
    engine_from_sources(&sources, cur).or_else(|| engine_from_sources(&sources, 0))
}

// ======================== 运行时加载的库 ========================

type P = *mut c_void;

crate::dl_fns!(struct Glib {
    g_main_context_new: fn() -> P;
    g_main_context_push_thread_default: fn(P);
    g_main_context_iteration: fn(P, c_int) -> c_int;
    g_main_context_wakeup: fn(P);
    g_timeout_source_new: fn(c_uint) -> P;
    g_source_set_callback: fn(P, P, P, P);
    g_source_attach: fn(P, P) -> c_uint;
    g_source_unref: fn(P);
    g_error_free: fn(P);
});

crate::dl_fns!(struct Gobject {
    g_signal_connect_data: fn(P, *const c_char, P, P, P, c_int) -> c_ulong;
    g_object_ref: fn(P) -> P;
    g_object_unref: fn(P);
});

crate::dl_fns!(struct Gio {
    g_dbus_connection_flush_sync: fn(P, P, *mut P) -> c_int;
});

crate::dl_fns!(struct Ibusf {
    ibus_init: fn();
    ibus_bus_new: fn() -> P;
    ibus_bus_is_connected: fn(P) -> c_int;
    ibus_bus_get_connection: fn(P) -> P;
    ibus_bus_get_global_engine: fn(P) -> P;
    ibus_engine_desc_get_name: fn(P) -> *const c_char;
    ibus_bus_set_global_engine: fn(P, *const c_char) -> c_int;
    ibus_bus_set_global_engine_async: fn(P, *const c_char, c_int, P, P, P);
    ibus_bus_set_global_engine_async_finish: fn(P, P, *mut P) -> c_int;
    ibus_factory_new: fn(P) -> P;
    ibus_component_new: fn(*const c_char, *const c_char, *const c_char, *const c_char, *const c_char, *const c_char, *const c_char, *const c_char) -> P;
    ibus_engine_desc_new: fn(*const c_char, *const c_char, *const c_char, *const c_char, *const c_char, *const c_char, *const c_char, *const c_char) -> P;
    ibus_component_add_engine: fn(P, P);
    ibus_bus_register_component: fn(P, P) -> c_int;
    ibus_engine_new: fn(*const c_char, *const c_char, P) -> P;
    ibus_engine_commit_text: fn(P, P);
    ibus_text_new_from_string: fn(*const c_char) -> P;
});

struct Fns {
    g: Glib,
    o: Gobject,
    io: Gio,
    i: Ibusf,
}

static FNS: OnceLock<Result<Fns, String>> = OnceLock::new();

fn fns() -> Result<&'static Fns, String> {
    FNS.get_or_init(|| {
        let open = |names: &[&str]| Lib::open(names).map(|l| &*Box::leak(Box::new(l)));
        // 先加载 IBus：没装 IBus 的系统最常见，报这个原因
        let ibus = open(&["libibus-1.0.so.5", "libibus-1.0.so"])?;
        let g = Glib::load(open(&["libglib-2.0.so.0"])?)?;
        let o = Gobject::load(open(&["libgobject-2.0.so.0"])?)?;
        let io = Gio::load(open(&["libgio-2.0.so.0"])?)?;
        let i = Ibusf::load(ibus)?;
        unsafe { (i.ibus_init)() };
        Ok(Fns { g, o, io, i })
    })
    .as_ref()
    .map_err(|e| e.clone())
}

fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

/// 连上会话的 IBus 守护进程（地址按 IBUS_ADDRESS 或 WAYLAND_DISPLAY / DISPLAY 找）
fn connect(f: &Fns) -> Result<P, String> {
    let bus = unsafe { (f.i.ibus_bus_new)() };
    if bus.is_null() || unsafe { (f.i.ibus_bus_is_connected)(bus) } == 0 {
        return Err("IBus 守护进程没有运行".into());
    }
    Ok(bus)
}

fn global_engine(f: &Fns, bus: P) -> Option<String> {
    unsafe {
        if (f.i.ibus_bus_is_connected)(bus) == 0 {
            return None;
        }
        let desc = (f.i.ibus_bus_get_global_engine)(bus);
        if desc.is_null() {
            return Some(String::new());
        }
        let n = (f.i.ibus_engine_desc_get_name)(desc);
        let name = if n.is_null() { String::new() } else { CStr::from_ptr(n).to_string_lossy().into_owned() };
        (f.o.g_object_unref)(desc);
        Some(name)
    }
}

// ======================== IBus 线程（画面代理里） ========================

/// 工厂建出来的我们的引擎（只在 IBus 线程里用）
struct Slot {
    conn: P,
    engine: P,
    n: u32,
}

unsafe extern "C" fn on_create_engine(_factory: P, name: *const c_char, data: P) -> P {
    let Ok(f) = fns() else { return std::ptr::null_mut() };
    let slot = &mut *(data as *mut Slot);
    slot.n += 1;
    let path = cstr(&format!("/org/freedesktop/IBus/Engine/LemonGalaxy{}", slot.n));
    let e = (f.i.ibus_engine_new)(name, path.as_ptr(), slot.conn);
    if e.is_null() {
        return e;
    }
    // 自己留一个引用（工厂会接管浮动引用）；旧的放掉
    (f.o.g_object_ref)(e);
    if !slot.engine.is_null() {
        (f.o.g_object_unref)(slot.engine);
    }
    slot.engine = e;
    e
}

unsafe extern "C" fn on_set_done(bus: P, res: P, data: P) {
    let done = Box::from_raw(data as *mut Arc<AtomicI32>);
    let Ok(f) = fns() else { return done.store(0, Ordering::SeqCst) };
    let mut err: P = std::ptr::null_mut();
    let ok = (f.i.ibus_bus_set_global_engine_async_finish)(bus, res, &mut err);
    if !err.is_null() {
        (f.g.g_error_free)(err);
    }
    done.store((ok != 0) as i32, Ordering::SeqCst);
}

unsafe extern "C" fn wake_once(_: P) -> c_int {
    0
}

struct Live {
    f: &'static Fns,
    ctx: P,
    bus: P,
    slot: *mut Slot,
    guard: Option<std::fs::File>,
}

impl Live {
    /// `d` 之后保证主循环醒一次
    fn arm(&self, d: Duration) {
        unsafe {
            let src = (self.f.g.g_timeout_source_new)(d.as_millis().clamp(1, 60_000) as c_uint);
            (self.f.g.g_source_set_callback)(src, wake_once as P, std::ptr::null_mut(), std::ptr::null_mut());
            (self.f.g.g_source_attach)(src, self.ctx);
            (self.f.g.g_source_unref)(src);
        }
    }

    /// 跑主循环直到 `done()` 或超时（期间守护进程回调我们的工厂、引擎都能处理）
    fn pump_until(&self, timeout: Duration, done: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        self.arm(timeout);
        loop {
            if done() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            unsafe { (self.f.g.g_main_context_iteration)(self.ctx, 1) };
        }
    }

    fn drop_engine(&mut self) {
        let slot = unsafe { &mut *self.slot };
        if !slot.engine.is_null() {
            unsafe { (self.f.o.g_object_unref)(slot.engine) };
            slot.engine = std::ptr::null_mut();
        }
    }
}

impl Engines for Live {
    fn current(&mut self) -> Option<String> {
        global_engine(self.f, self.bus)
    }

    fn set(&mut self, name: &str) -> bool {
        if unsafe { (self.f.i.ibus_bus_is_connected)(self.bus) } == 0 {
            return false;
        }
        // 切到我们的：等工厂建出新引擎；切走：旧引擎守护进程会销毁
        self.drop_engine();
        let done = Arc::new(AtomicI32::new(-1));
        let data = Box::into_raw(Box::new(done.clone())) as P;
        let n = cstr(name);
        unsafe { (self.f.i.ibus_bus_set_global_engine_async)(self.bus, n.as_ptr(), 800, std::ptr::null_mut(), on_set_done as P, data) };
        // 异步调用自己 800 毫秒超时会回调；多等一点
        self.pump_until(Duration::from_millis(1200), || done.load(Ordering::SeqCst) >= 0);
        done.load(Ordering::SeqCst) == 1
    }

    fn commit(&mut self, text: &str) -> bool {
        // 切过来后守护进程要有焦点输入框才建引擎：稍等
        let slot = self.slot;
        self.pump_until(Duration::from_millis(300), || unsafe { !(*slot).engine.is_null() });
        let engine = unsafe { (*slot).engine };
        if engine.is_null() {
            return false;
        }
        let t = cstr(text);
        unsafe {
            let it = (self.f.i.ibus_text_new_from_string)(t.as_ptr());
            (self.f.i.ibus_engine_commit_text)(engine, it);
            // 发出去再返回：后面的按键（经 Mutter）不会跑到它前面
            (self.f.io.g_dbus_connection_flush_sync)((*slot).conn, std::ptr::null_mut(), std::ptr::null_mut());
        }
        true
    }

    fn fallback(&mut self) -> Option<String> {
        gnome_engine()
    }

    fn guard(&mut self, prev: Option<&str>) {
        if let Some(g) = self.guard.as_mut() {
            let line = match prev {
                Some(p) => format!("on {p}\n"),
                None => "off\n".into(),
            };
            let _ = g.write_all(line.as_bytes());
        }
    }

    fn wait(&mut self, d: Duration) {
        self.pump_until(d, || false);
    }
}

enum Cmd {
    Commit(String, mpsc::Sender<bool>),
    /// 切回原引擎（退出前），完成后回话
    Shutdown(mpsc::Sender<()>),
}

pub struct Ibus {
    tx: mpsc::Sender<Cmd>,
    ctx: usize,
}

/// 起看门进程：它从标准输入读我们的状态，我们（不管怎么）退出后它看到管道关闭，必要时把引擎切回
fn spawn_guard() -> Option<std::fs::File> {
    let mut fds = [0 as c_int; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return None;
    }
    use std::os::fd::FromRawFd;
    let (r, w) = unsafe { (std::os::fd::OwnedFd::from_raw_fd(fds[0]), std::fs::File::from_raw_fd(fds[1])) };
    // /proc/self/exe：程序所在目录本用户进不去时也能执行（单测里换成编好的被控端程序）
    #[cfg(test)]
    let exe = std::env::var("LG_IBUS_GUARD_EXE").unwrap_or_else(|_| "/proc/self/exe".into());
    #[cfg(not(test))]
    let exe = "/proc/self/exe";
    match Command::new(exe).arg("__lg-ibus-guard").stdin(Stdio::from(r)).stdout(Stdio::null()).spawn() {
        Ok(child) => {
            // 不等它：它在我们退出后才退出
            std::mem::forget(child);
            Some(w)
        }
        Err(e) => {
            eprintln!("IBus 看门进程起不来（代理异常退出时可能来不及切回输入法）：{e}");
            None
        }
    }
}

fn start() -> Result<Ibus, String> {
    let f = fns()?;
    let (tx, rx) = mpsc::channel::<Cmd>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<usize, String>>();
    std::thread::Builder::new()
        .name("ibus".into())
        .spawn(move || unsafe {
            let ctx = (f.g.g_main_context_new)();
            (f.g.g_main_context_push_thread_default)(ctx);
            let bus = match connect(f) {
                Ok(b) => b,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let conn = (f.i.ibus_bus_get_connection)(bus);
            let slot = Box::into_raw(Box::new(Slot { conn, engine: std::ptr::null_mut(), n: 0 }));
            let factory = (f.i.ibus_factory_new)(conn);
            let sig = cstr("create-engine");
            (f.o.g_signal_connect_data)(factory, sig.as_ptr(), on_create_engine as P, slot as P, std::ptr::null_mut(), 0);
            let s = |x: &str| cstr(x);
            let (cn, cd, cv, cl, ca, ch, ce, ct) = (s(COMPONENT), s("LemonGalaxy remote input"), s(env!("CARGO_PKG_VERSION")), s("MIT"), s("LemonIT"), s("https://lemonit.cn"), s(""), s("lemon-galaxy"));
            let comp = (f.i.ibus_component_new)(cn.as_ptr(), cd.as_ptr(), cv.as_ptr(), cl.as_ptr(), ca.as_ptr(), ch.as_ptr(), ce.as_ptr(), ct.as_ptr());
            let (en, el, ed, lang, icon, layout) = (s(OURS), s("LemonGalaxy"), s("Text typed from the remote controller"), s("other"), s(""), s("default"));
            let desc = (f.i.ibus_engine_desc_new)(en.as_ptr(), el.as_ptr(), ed.as_ptr(), lang.as_ptr(), cl.as_ptr(), ca.as_ptr(), icon.as_ptr(), layout.as_ptr());
            (f.i.ibus_component_add_engine)(comp, desc);
            if (f.i.ibus_bus_register_component)(bus, comp) == 0 {
                let _ = ready_tx.send(Err("向 IBus 注册引擎失败".into()));
                return;
            }
            let mut live = Live { f, ctx, bus, slot, guard: spawn_guard() };
            // 告诉看门进程现在的真实引擎：我们退出后守护进程会清空全局引擎，它要设回这个
            let seen = global_engine(f, bus).filter(|e| real(e)).or_else(gnome_engine).unwrap_or_else(|| DEFAULT_ENGINE.into());
            if let Some(g) = live.guard.as_mut() {
                let _ = g.write_all(format!("seen {seen}\n").as_bytes());
            }
            let _ = ready_tx.send(Ok(ctx as usize));
            run(rx, &mut live);
        })
        .map_err(|e| e.to_string())?;
    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(ctx)) => Ok(Ibus { tx, ctx }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("连 IBus 超时".into()),
    }
}

fn run(rx: mpsc::Receiver<Cmd>, live: &mut Live) {
    let mut sw = Switcher::default();
    let mut armed: Option<Instant> = None;
    loop {
        unsafe { (live.f.g.g_main_context_iteration)(live.ctx, 1) };
        loop {
            match rx.try_recv() {
                Ok(Cmd::Commit(text, reply)) => {
                    let ok = sw.commit(live, &text, Instant::now());
                    let _ = reply.send(ok);
                }
                Ok(Cmd::Shutdown(reply)) => {
                    if sw.active() && !sw.restore(live, Instant::now() + Duration::from_millis(900)) {
                        eprintln!("退出前没能切回原输入法（看门进程会再试）");
                    }
                    let _ = reply.send(());
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    sw.restore(live, Instant::now() + Duration::from_millis(900));
                    return;
                }
            }
        }
        sw.tick(live, Instant::now());
        // 下一个切回的时间点叫醒主循环
        if let Some(d) = sw.deadline() {
            if armed != Some(d) {
                armed = Some(d);
                live.arm(d.saturating_duration_since(Instant::now()));
            }
        }
    }
}

impl Ibus {
    fn send(&self, c: Cmd) -> bool {
        if self.tx.send(c).is_err() {
            return false;
        }
        if let Ok(f) = fns() {
            unsafe { (f.g.g_main_context_wakeup)(self.ctx as P) };
        }
        true
    }
}

static IBUS: OnceLock<Option<Ibus>> = OnceLock::new();

fn get() -> Option<&'static Ibus> {
    IBUS.get_or_init(|| match start() {
        Ok(i) => {
            eprintln!("输入文字：经 IBus 输入法引擎提交（布局外的字符、中日韩都能输入）");
            Some(i)
        }
        Err(e) => {
            eprintln!("输入文字：IBus 不可用（{e}），改为按键输入（键盘布局里没有的字符会丢失）");
            None
        }
    })
    .as_ref()
}

/// 提交一段文字，阻塞到发出去（后面的按键自然排在它后面）。返回 false 表示没用 IBus，调用方改按键。
pub fn commit(text: &str) -> bool {
    if text.is_empty() {
        return true;
    }
    let Some(i) = get() else { return false };
    let (tx, rx) = mpsc::channel();
    if !i.send(Cmd::Commit(text.to_string(), tx)) {
        return false;
    }
    match rx.recv_timeout(Duration::from_millis(1500)) {
        Ok(ok) => ok,
        // 超时：IBus 可能已经提交了，不再按键（免得输入两遍）
        Err(_) => {
            eprintln!("IBus 提交文字超时");
            true
        }
    }
}

/// 退出前调用：还在我们的引擎上就同步切回（最多约 1 秒）。没用过 IBus 时什么也不做。
pub fn shutdown() {
    let Some(Some(i)) = IBUS.get() else { return };
    let (tx, rx) = mpsc::channel();
    if i.send(Cmd::Shutdown(tx)) {
        let _ = rx.recv_timeout(Duration::from_millis(1200));
    }
}

// ======================== 看门进程 ========================

/// 同步调用的 IBus（看门进程里用：这个进程没有工厂，同步切引擎不会自锁）
struct SyncEngines {
    f: &'static Fns,
    bus: P,
}

impl Engines for SyncEngines {
    fn current(&mut self) -> Option<String> {
        global_engine(self.f, self.bus)
    }
    fn set(&mut self, name: &str) -> bool {
        let n = cstr(name);
        unsafe { (self.f.i.ibus_bus_set_global_engine)(self.bus, n.as_ptr()) != 0 }
    }
    fn commit(&mut self, _: &str) -> bool {
        false
    }
    fn fallback(&mut self) -> Option<String> {
        gnome_engine()
    }
    fn guard(&mut self, _: Option<&str>) {}
    fn wait(&mut self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// 看门进程读到的状态
#[derive(Default, Debug, PartialEq)]
pub struct GuardState {
    /// 代理退出时还在我们的引擎上：原引擎
    pub active: Option<String>,
    /// 最后一个真实引擎
    pub last_real: Option<String>,
}

impl GuardState {
    pub fn feed(&mut self, line: &str) {
        match line.trim().split_once(' ') {
            Some(("seen", e)) if real(e) => self.last_real = Some(e.to_string()),
            Some(("on", e)) => {
                self.active = Some(e.to_string());
                if real(e) {
                    self.last_real = Some(e.to_string());
                }
            }
            _ if line.trim() == "off" => self.active = None,
            _ => {}
        }
    }

    /// 要设回的引擎
    pub fn target(&self) -> Option<String> {
        self.active.clone().or_else(|| self.last_real.clone())
    }
}

/// `__lg-ibus-guard`：标准输入是画面代理写的状态（"seen <引擎>" / "on <原引擎>" / "off"）。管道关闭（代理退出，
/// 包括被 kill -9）后看约 3 秒：全局引擎是我们的或者空了（守护进程在注册过组件的进程退出后会清空它），就设回去。
pub fn guard_main() -> i32 {
    // 服务停止时 systemd 会给整个进程组发 SIGTERM：忽略它，把切回做完（代理一退我们 3 秒内就结束）
    unsafe {
        libc::signal(libc::SIGTERM, libc::SIG_IGN);
        libc::signal(libc::SIGINT, libc::SIG_IGN);
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
    }
    let mut st = GuardState::default();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        st.feed(&line);
    }
    let Some(target) = st.target() else { return 0 };
    let Ok(f) = fns() else { return 1 };
    let Ok(bus) = connect(f) else { return 1 };
    let mut e = SyncEngines { f, bus };
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(3) {
        match e.current() {
            // 守护进程没了：没什么可做
            None => return 0,
            Some(c) if real(&c) => std::thread::sleep(Duration::from_millis(100)),
            Some(_) => {
                let mut sw = Switcher { prev: Some(target.clone()), last_real: None, deadline: None };
                return if sw.restore(&mut e, Instant::now() + Duration::from_secs(5)) {
                    eprintln!("画面代理退出后 IBus 没有可用的引擎，已设回 {target}");
                    0
                } else {
                    eprintln!("画面代理退出后 IBus 没有可用的引擎，没能设回 {target}");
                    1
                };
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 假的 IBus：记下调用；`fail_set` 里的引擎名前几次切换失败；`wrong_after_set` 次切换后读回的不对
    #[derive(Default)]
    struct Fake {
        cur: String,
        log: Vec<String>,
        fail_set: Vec<(String, u32)>,
        never: Vec<String>,
        mismatch: u32,
        commit_ok: bool,
        gnome: Option<String>,
        guard: Option<String>,
    }

    impl Engines for Fake {
        fn current(&mut self) -> Option<String> {
            Some(self.cur.clone())
        }
        fn set(&mut self, name: &str) -> bool {
            self.log.push(format!("set {name}"));
            if self.never.iter().any(|n| n == name) {
                return false;
            }
            if let Some(f) = self.fail_set.iter_mut().find(|(n, k)| n.as_str() == name && *k > 0) {
                f.1 -= 1;
                return false;
            }
            if self.mismatch > 0 && name != OURS {
                self.mismatch -= 1;
                return true;
            }
            self.cur = name.to_string();
            true
        }
        fn commit(&mut self, text: &str) -> bool {
            self.log.push(format!("commit {text}"));
            self.commit_ok
        }
        fn fallback(&mut self) -> Option<String> {
            self.gnome.clone()
        }
        fn guard(&mut self, prev: Option<&str>) {
            self.guard = prev.map(|s| s.to_string());
        }
        fn wait(&mut self, _: Duration) {}
    }

    fn fake(cur: &str) -> Fake {
        Fake { cur: cur.into(), commit_ok: true, ..Default::default() }
    }

    #[test]
    fn batches_and_switches_back_after_idle() {
        let mut e = fake("ibus-libpinyin");
        let mut sw = Switcher::default();
        let t0 = Instant::now();
        assert!(sw.commit(&mut e, "你好", t0));
        assert_eq!(e.guard.as_deref(), Some("ibus-libpinyin"));
        // 300 毫秒内又来一段：不再切换
        assert!(sw.commit(&mut e, "世界", t0 + Duration::from_millis(200)));
        sw.tick(&mut e, t0 + Duration::from_millis(400));
        assert!(sw.active());
        sw.tick(&mut e, t0 + Duration::from_millis(501));
        assert!(!sw.active());
        assert_eq!(e.cur, "ibus-libpinyin");
        assert_eq!(e.guard, None);
        assert_eq!(e.log, vec!["set lemongalaxy-remote", "commit 你好", "commit 世界", "set ibus-libpinyin"]);
    }

    #[test]
    fn remembers_real_engine_when_left_on_ours() {
        // 上次没切回、现在还是我们的引擎：用记下的真实引擎
        let mut e = fake("xkb:de::ger");
        let mut sw = Switcher::default();
        let t0 = Instant::now();
        assert!(sw.commit(&mut e, "é", t0));
        e.never = vec!["xkb:de::ger".into()];
        e.gnome = None;
        // 切不回（原引擎没了）：依次试 GNOME 设置（没有）、xkb:us::eng
        assert!(sw.restore(&mut e, t0 + Duration::from_secs(3)));
        assert_eq!(e.cur, DEFAULT_ENGINE);
        // 再来：当前是 xkb:us::eng，就以它为原引擎
        e.cur = OURS.into();
        let mut sw2 = Switcher { last_real: Some("ibus-anthy".into()), ..Default::default() };
        e.never.clear();
        assert!(sw2.commit(&mut e, "テスト", t0));
        assert_eq!(e.guard.as_deref(), Some("ibus-anthy"));
        sw2.tick(&mut e, t0 + Duration::from_secs(1));
        assert_eq!(e.cur, "ibus-anthy");
        // 记录都没有、也没有引擎：GNOME 设置里的
        let mut e = fake("");
        e.gnome = Some("ibus-hangul".into());
        let mut sw3 = Switcher::default();
        assert!(sw3.commit(&mut e, "한", t0));
        sw3.tick(&mut e, t0 + Duration::from_secs(1));
        assert_eq!(e.cur, "ibus-hangul");
    }

    #[test]
    fn restore_retries_and_verifies() {
        let mut e = fake("ibus-rime");
        let mut sw = Switcher::default();
        let t0 = Instant::now();
        assert!(sw.commit(&mut e, "x", t0));
        // 前两次切换报失败、再一次报成功但读回来不对：都重试，最后核对通过
        e.fail_set = vec![("ibus-rime".into(), 1)];
        e.mismatch = 1;
        assert!(sw.restore(&mut e, t0 + Duration::from_secs(3)));
        assert_eq!(e.cur, "ibus-rime");
        assert_eq!(e.log.iter().filter(|l| *l == "set ibus-rime").count(), 3);
        assert!(!sw.active());
    }

    #[test]
    fn failed_restore_stays_active_and_retries_later() {
        let mut e = fake("ibus-rime");
        let mut sw = Switcher::default();
        let t0 = Instant::now();
        assert!(sw.commit(&mut e, "x", t0));
        e.never = vec!["ibus-rime".into(), DEFAULT_ENGINE.into()];
        sw.tick(&mut e, t0 + Duration::from_secs(1));
        // 切不回：还记着，看门进程也还是 on，1 秒后再试
        assert!(sw.active());
        assert_eq!(e.guard.as_deref(), Some("ibus-rime"));
        assert!(sw.deadline().unwrap() > t0 + Duration::from_secs(1));
        e.never.clear();
        sw.tick(&mut e, t0 + Duration::from_secs(3));
        assert!(!sw.active());
        assert_eq!(e.cur, "ibus-rime");
    }

    #[test]
    fn commit_failure_switches_back_at_once() {
        // 没有焦点输入框（引擎没建出来）：提交失败，立刻切回，调用方改按键
        let mut e = fake("xkb:us::eng");
        e.commit_ok = false;
        let mut sw = Switcher::default();
        assert!(!sw.commit(&mut e, "a", Instant::now()));
        assert!(!sw.active());
        assert_eq!(e.cur, "xkb:us::eng");
        // 切到我们的引擎就失败：也确认原引擎还在
        let mut e = fake("xkb:us::eng");
        e.never = vec![OURS.into()];
        assert!(!sw.commit(&mut e, "a", Instant::now()));
        assert!(!sw.active());
        assert_eq!(e.guard, None);
    }

    #[test]
    fn shutdown_restores_when_active() {
        let mut e = fake("ibus-mozc-jp");
        let mut sw = Switcher::default();
        let t0 = Instant::now();
        // 没切过：什么也不做
        assert!(sw.restore(&mut e, t0 + Duration::from_secs(1)));
        assert!(e.log.is_empty());
        assert!(sw.commit(&mut e, "日本", t0));
        // 还没到空闲时间就退出：同步切回
        assert!(sw.restore(&mut e, Instant::now() + Duration::from_secs(1)));
        assert_eq!(e.cur, "ibus-mozc-jp");
        // 超时不再试
        let mut sw = Switcher::default();
        assert!(sw.commit(&mut e, "x", t0));
        e.never = vec!["ibus-mozc-jp".into()];
        assert!(!sw.restore(&mut e, Instant::now() - Duration::from_millis(1)));
    }

    /// 真 IBus（容器里起 ibus-daemon，另有一个客户端建好输入上下文并聚焦、原引擎 xkb:us::eng）：LG_TEST_IBUS=1 时跑
    #[test]
    fn real_ibus() {
        if std::env::var("LG_TEST_IBUS").ok().as_deref() != Some("1") {
            return;
        }
        let f = fns().expect("加载 libibus");
        let bus = connect(f).expect("连 IBus");
        assert_eq!(global_engine(f, bus).as_deref(), Some("xkb:us::eng"));
        assert!(commit("你好世界 テスト é\n"));
        assert_eq!(global_engine(f, bus).as_deref(), Some(OURS));
        // 300 毫秒内的下一段：还在我们的引擎上
        std::thread::sleep(Duration::from_millis(100));
        assert!(commit("第二段"));
        // 空闲后切回并核对
        std::thread::sleep(Duration::from_millis(800));
        assert_eq!(global_engine(f, bus).as_deref(), Some("xkb:us::eng"));
        // 提交后马上退出：同步切回
        assert!(commit("最后"));
        assert_eq!(global_engine(f, bus).as_deref(), Some(OURS));
        shutdown();
        assert_eq!(global_engine(f, bus).as_deref(), Some("xkb:us::eng"));
    }

    /// 提交后一直不退（LG_TEST_IBUS_HOLD=1）：外面 kill -9 后看看门进程有没有把引擎切回
    #[test]
    fn real_ibus_hold() {
        if std::env::var("LG_TEST_IBUS_HOLD").ok().as_deref() != Some("1") {
            return;
        }
        assert!(commit("hold"));
        // 别让它空闲切回：一直提交
        for _ in 0..600 {
            std::thread::sleep(Duration::from_millis(100));
            commit(".");
        }
    }

    #[test]
    fn guard_state() {
        let mut g = GuardState::default();
        assert_eq!(g.target(), None);
        g.feed("seen xkb:us::eng");
        assert_eq!(g.target().as_deref(), Some("xkb:us::eng"));
        // 切到我们的时原引擎是 libpinyin；退出时还在我们的引擎上：设回 libpinyin
        g.feed("on ibus-libpinyin");
        assert_eq!(g.target().as_deref(), Some("ibus-libpinyin"));
        g.feed("off");
        // 已切回：也还记着最后一个真实引擎（守护进程会在我们退出后清空全局引擎）
        assert_eq!(g, GuardState { active: None, last_real: Some("ibus-libpinyin".into()) });
        // 我们自己、空的不算真实引擎
        g.feed(&format!("seen {OURS}"));
        g.feed("seen ");
        assert_eq!(g.target().as_deref(), Some("ibus-libpinyin"));
    }

    #[test]
    fn parses_gnome_sources() {
        let s = "[('xkb', 'us'), ('ibus', 'libpinyin'), ('ibus', 'anthy')]\n";
        assert_eq!(engine_from_sources(s, 0).as_deref(), Some(DEFAULT_ENGINE));
        assert_eq!(engine_from_sources(s, 1).as_deref(), Some("libpinyin"));
        assert_eq!(engine_from_sources(s, 2).as_deref(), Some("anthy"));
        assert_eq!(engine_from_sources(s, 3), None);
        assert_eq!(engine_from_sources("@a(ss) []\n", 0), None);
        assert_eq!(engine_from_sources("", 0), None);
    }
}
