//! 服务这一侧的平台层：找图形会话、把画面代理放进去、把内核的请求转给它（结构同 Windows 被控端）。
//!
//! 每 2 秒看一次「现在屏幕上是谁」，变了就把代理换到新会话里重启，正在进行的画面与声音自动续上。
//! 没有可看的会话时，主控打开屏幕列表会经 `prepare_desktop` 给账号的系统用户开虚拟桌面。

use super::ipc::{read_msg, write_msg, FromAgent, ToAgent};
use super::session::{self, GuiSession, Kind};
use super::vdesktop;
use galaxy_host::audio::AudioSink;
use galaxy_host::platform::{CaptureStarted, FrameSink, HostPlatform};
use galaxy_host::sysuser::SysUser;
use galaxy_proto::peer::{ClipboardData, DisplayInfo, ImageData, InputEvent, VideoCodec};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Agent {
    writer: mpsc::Sender<ToAgent>,
    child: Arc<Mutex<Child>>,
    session: GuiSession,
    generation: u64,
}

#[derive(Clone)]
struct Capture {
    sink: Arc<dyn FrameSink>,
    max_fps: u32,
    bitrate_kbps: u32,
    cursor_in_video: bool,
}

struct Inner {
    agent: Mutex<Option<Agent>>,
    next_id: AtomicU64,
    generation: AtomicU64,
    pending: Mutex<HashMap<u64, mpsc::Sender<FromAgent>>>,
    captures: Mutex<HashMap<u32, Capture>>,
    audio: Mutex<Option<Arc<dyn AudioSink>>>,
    /// 最近一次 prepare_desktop 的系统用户（有虚拟桌面时优先看它的）
    wanted_user: Mutex<Option<u32>>,
    /// 代理启动失败的会话与时间：同一会话 10 秒内不再反复重启
    failed: Mutex<Option<(String, Instant, String)>>,
    /// 当前目标会话（每 2 秒刷新）
    target: Mutex<Option<GuiSession>>,
    /// 我们为远程解锁的 logind 会话号：所有画面都停了以后重新锁上
    unlocked_by_us: Mutex<Option<String>>,
    /// 最近一次列到的屏幕（锁屏等原因列不出来时还给主控，让它开画面时看到原因）
    last_displays: Mutex<Vec<DisplayInfo>>,
}

/// logind 会话是否锁屏；不是 logind 会话或查不到时 None
fn session_locked(id: &str) -> Option<bool> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let out = Command::new("loginctl").args(["show-session", id, "-p", "LockedHint", "--value"]).output().ok()?;
    match String::from_utf8_lossy(&out.stdout).trim() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

fn loginctl(action: &str, id: &str) -> bool {
    Command::new("loginctl").args([action, id]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

/// 被控端设置里打开了「远程连入时自动解锁」没有（读 host.json，命令行与界面改了立即生效）
fn unlock_on_connect() -> bool {
    galaxy_host::accounts::HostState::load(&super::data_dir().join("host.json")).unlock_on_connect
}

const LOCKED_HINT: &str = "被控机屏幕已锁定：GNOME 等 Wayland 桌面锁屏时系统禁止远程桌面。请在被控机上解锁，或由机主在被控端设置里打开「远程连入时自动解锁」。";

pub struct LinuxPlatform {
    inner: Arc<Inner>,
}

/// 现在该看哪个会话：真屏幕（seat0）→ 指定用户的虚拟桌面 → 任一虚拟桌面 → 普通用户运行时自己的图形环境
fn pick(wanted: Option<u32>) -> Option<GuiSession> {
    let seat = session::find_seat();
    // 真屏幕上登录的用户如果开着虚拟桌面，关掉它（同一用户两个桌面会互相干扰）
    if let Some(s) = seat.as_ref().filter(|s| !s.greeter) {
        if vdesktop::users().contains(&s.uid) {
            vdesktop::stop(s.uid);
        }
    }
    // 真屏幕可用：X11 都行；Wayland 只有登录用户的会话（GDM 的 Wayland 登录界面看不到，这时用虚拟桌面）
    if let Some(s) = seat.as_ref() {
        let wayland_greeter = s.greeter && s.kind == Kind::Wayland;
        if !wayland_greeter {
            return seat;
        }
    }
    if let Some(u) = wanted {
        if let Some(v) = vdesktop::session_for(u) {
            return Some(v);
        }
    }
    if let Some(v) = vdesktop::any_session() {
        return Some(v);
    }
    if !nix::unistd::getuid().is_root() {
        if let Some(s) = session::from_own_env() {
            return Some(s);
        }
    }
    // 只有 Wayland 登录界面：还是给它（显示「登录界面」，代理会报看不到，主控那边能看到原因）
    seat
}

impl LinuxPlatform {
    pub fn new() -> Arc<Self> {
        let me = Arc::new(Self {
            inner: Arc::new(Inner {
                agent: Mutex::new(None),
                next_id: AtomicU64::new(1),
                generation: AtomicU64::new(1),
                pending: Mutex::new(HashMap::new()),
                captures: Mutex::new(HashMap::new()),
                audio: Mutex::new(None),
                wanted_user: Mutex::new(None),
                failed: Mutex::new(None),
                target: Mutex::new(None),
                unlocked_by_us: Mutex::new(None),
                last_displays: Mutex::new(vec![]),
            }),
        });
        let w = Arc::downgrade(&me.inner);
        std::thread::Builder::new()
            .name("session-watch".into())
            .spawn(move || loop {
                let Some(inner) = w.upgrade() else { break };
                let wanted = *inner.wanted_user.lock();
                let t = pick(wanted);
                *inner.target.lock() = t.clone();
                let current = inner.agent.lock().as_ref().map(|a| a.session.clone());
                if let Some(cur) = current {
                    if Some(&cur) != t.as_ref() {
                        tracing::info!("屏幕上的会话变了：{} → {}，重启画面代理", cur.describe(), t.as_ref().map(|s| s.describe()).unwrap_or_else(|| "没有".into()));
                        if let Some(a) = inner.agent.lock().take() {
                            let _ = a.child.lock().kill();
                        }
                        let busy = !inner.captures.lock().is_empty() || inner.audio.lock().is_some();
                        if busy && t.is_some() {
                            let _ = ensure_agent(&inner);
                            restart_captures(&inner);
                        }
                    }
                }
                drop(inner);
                std::thread::sleep(Duration::from_secs(2));
            })
            .expect("thread");
        me
    }
}

/// 本程序的一个打开着的文件描述符，子进程经 /proc/self/fd/N 执行它。
/// 以别的用户身份启动代理时用：程序可能放在那个用户进不去的目录里（比如 0750 的家目录），
/// 经描述符执行不用再走一遍目录，只要程序文件本身对其他用户可执行（0755）。
fn exe_via_fd() -> Option<std::path::PathBuf> {
    use std::os::fd::AsRawFd;
    static F: std::sync::OnceLock<Option<std::fs::File>> = std::sync::OnceLock::new();
    F.get_or_init(|| std::env::current_exe().ok().and_then(|p| std::fs::File::open(p).ok())).as_ref().map(|f| std::path::PathBuf::from(format!("/proc/self/fd/{}", f.as_raw_fd())))
}

/// 以会话用户身份、带会话环境变量运行本程序的 `arg` 子命令（画面代理、自检）。返回命令与后端名。
pub fn session_command(s: &GuiSession, arg: &str) -> Result<(Command, &'static str), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let root = nix::unistd::getuid().is_root();
    // 以会话用户身份运行（X11 登录界面以 root 运行：它的 Xauthority 往往只有 root 能读）
    let run_as = if root && !(s.greeter && s.kind == Kind::X11) { SysUser::lookup(&s.user).filter(|u| u.uid == s.uid) } else { None };
    let program = if run_as.is_some() { exe_via_fd().unwrap_or(exe) } else { exe };
    let mut cmd = Command::new(&program);
    cmd.arg(arg);
    let base = run_as.clone().unwrap_or_else(SysUser::current);
    let mut env = galaxy_host::sysuser::child_env(&base);
    env.retain(|(k, _)| k != "TERM" && k != "COLORTERM");
    env.extend(s.env.iter().cloned());
    let backend = match (s.kind, s.is_gnome() || s.greeter) {
        (Kind::X11, _) => "x11",
        (Kind::Wayland, true) => "mutter",
        (Kind::Wayland, false) => "portal",
    };
    env.push(("LG_BACKEND".into(), backend.into()));
    env.push(("LG_SESSION_ID".into(), s.id.clone()));
    if s.virtual_desktop {
        env.push(("LG_VIRTUAL".into(), "1".into()));
    }
    // 排障用的环境变量传下去
    for k in ["LEMON_GALAXY_VIDEO_ENCODER", "LEMON_GALAXY_VAAPI_DEVICE", "LEMON_GALAXY_CUDA_DEVICE", "LEMON_GALAXY_NVENC_PRESET", "LEMON_GALAXY_X11_DAMAGE", "LEMON_GALAXY_X11_SHM", "LEMON_GALAXY_LANG", "RUST_LOG"] {
        if let Ok(v) = std::env::var(k) {
            env.push((k.into(), v));
        }
    }
    cmd.env_clear().envs(env).current_dir(if base.home.is_empty() { "/".into() } else { base.home.clone() });
    if let Some(u) = run_as.as_ref() {
        galaxy_host::sysuser::prepare_command(&mut cmd, u, false);
    }
    Ok((cmd, backend))
}

fn spawn_agent(inner: &Arc<Inner>, s: &GuiSession) -> Result<Agent, String> {
    let (mut cmd, backend) = session_command(s, "__lg-session-agent")?;
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("启动画面代理失败：{e}"))?;
    let stdin = child.stdin.take().ok_or("没有标准输入")?;
    let stdout = child.stdout.take().ok_or("没有标准输出")?;
    let stderr = child.stderr.take().ok_or("没有标准错误")?;
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
            if !line.trim().is_empty() {
                tracing::warn!("画面代理：{line}");
            }
        }
    });
    let generation = inner.generation.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = mpsc::channel::<ToAgent>();
    std::thread::spawn(move || {
        let mut w = stdin;
        while let Ok(m) = rx.recv() {
            if write_msg(&mut w, &m).is_err() {
                break;
            }
        }
    });
    let weak = Arc::downgrade(inner);
    let session_id = s.id.clone();
    std::thread::spawn(move || {
        let mut r = std::io::BufReader::with_capacity(256 * 1024, stdout);
        loop {
            let m = match read_msg::<_, FromAgent>(&mut r) {
                Ok(Some(m)) => m,
                _ => break,
            };
            let Some(inner) = weak.upgrade() else { break };
            if let FromAgent::Lost { reason } = &m {
                tracing::warn!("画面代理报告会话不可用：{reason}");
                let why = if reason.contains("inhibited") && session_locked(&session_id) == Some(true) { LOCKED_HINT.to_string() } else { reason.clone() };
                *inner.failed.lock() = Some((session_id.clone(), Instant::now(), why));
            }
            route(&inner, m);
        }
        if let Some(inner) = weak.upgrade() {
            let mut g = inner.agent.lock();
            if g.as_ref().map(|a| a.generation == generation).unwrap_or(false) {
                if let Some(a) = g.take() {
                    let _ = a.child.lock().wait();
                }
                tracing::warn!("画面代理已退出");
            }
        }
    });
    tracing::info!("画面代理已启动：{}（{backend}，进程 {}）", s.describe(), child.id());
    Ok(Agent { writer: tx, child: Arc::new(Mutex::new(child)), session: s.clone(), generation })
}

fn route(inner: &Inner, m: FromAgent) {
    if let Some(id) = m.reply_id() {
        if let Some(tx) = inner.pending.lock().remove(&id) {
            let _ = tx.send(m);
        }
        return;
    }
    let sink_for = |d: u32| inner.captures.lock().get(&d).map(|c| c.sink.clone());
    match m {
        FromAgent::Frame { display, data, pts_us, keyframe } => {
            if let Some(s) = sink_for(display) {
                s.push(data, pts_us, keyframe);
            }
        }
        FromAgent::Resized { display, width, height } => {
            if let Some(s) = sink_for(display) {
                s.resized(width, height);
            }
        }
        FromAgent::Cursor { display, x, y, visible } => {
            if let Some(s) = sink_for(display) {
                s.cursor(x, y, visible);
            }
        }
        FromAgent::Audio { pcm, rate, channels } => {
            let sink = inner.audio.lock().clone();
            if let Some(s) = sink {
                s.push_pcm(&pcm, rate, channels);
            }
        }
        FromAgent::Hello { backend, detail } => tracing::info!("画面代理后端：{backend}（{detail}）"),
        FromAgent::Log { error, text } => {
            if error {
                tracing::warn!("画面代理：{text}");
            } else {
                tracing::info!("画面代理：{text}");
            }
        }
        _ => {}
    }
}

fn ensure_agent(inner: &Arc<Inner>) -> Result<(), String> {
    let mut g = inner.agent.lock();
    let alive = g.as_ref().map(|a| matches!(a.child.lock().try_wait(), Ok(None))).unwrap_or(false);
    if alive {
        return Ok(());
    }
    let target = inner.target.lock().clone().or_else(|| pick(*inner.wanted_user.lock()));
    let Some(s) = target else {
        // 开虚拟桌面失败的原因（比如没装 Xvfb）比「没有会话」更有用
        if let Some((_, t, why)) = inner.failed.lock().clone() {
            if t.elapsed() < Duration::from_secs(30) {
                return Err(why);
            }
        }
        return Err("没有可看的图形会话（没有人登录桌面，也没有虚拟桌面）".into());
    };
    if let Some((id, t, why)) = inner.failed.lock().clone() {
        if id == s.id && t.elapsed() < Duration::from_secs(10) {
            return Err(why);
        }
    }
    *g = Some(spawn_agent(inner, &s)?);
    Ok(())
}

fn send(inner: &Arc<Inner>, m: ToAgent) -> bool {
    if ensure_agent(inner).is_err() {
        return false;
    }
    inner.agent.lock().as_ref().map(|a| a.writer.send(m).is_ok()).unwrap_or(false)
}

fn request(inner: &Arc<Inner>, make: impl FnOnce(u64) -> ToAgent, timeout: Duration) -> Result<FromAgent, String> {
    ensure_agent(inner)?;
    let id = inner.next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = mpsc::channel();
    inner.pending.lock().insert(id, tx);
    if !send(inner, make(id)) {
        inner.pending.lock().remove(&id);
        return Err("画面代理不可用".into());
    }
    let r = rx.recv_timeout(timeout);
    inner.pending.lock().remove(&id);
    r.map_err(|_| {
        // 代理没回：可能是刚报了会话不可用
        inner.failed.lock().as_ref().filter(|(_, t, _)| t.elapsed() < Duration::from_secs(10)).map(|(_, _, w)| w.clone()).unwrap_or_else(|| "画面代理没有响应".into())
    })
}

fn restart_captures(inner: &Arc<Inner>) {
    if inner.audio.lock().is_some() {
        if let Ok(FromAgent::AudioStarted { result: Err(e), .. }) = request(inner, |id| ToAgent::StartAudio { id }, Duration::from_secs(8)) {
            tracing::warn!("会话切换后重新采集声音失败：{e}");
        }
    }
    let caps: Vec<(u32, Capture)> = inner.captures.lock().iter().map(|(k, v)| (*k, v.clone())).collect();
    for (display, c) in caps {
        let r = request(inner, |id| ToAgent::StartCapture { id, display, max_fps: c.max_fps, bitrate_kbps: c.bitrate_kbps }, Duration::from_secs(10));
        match r {
            Ok(FromAgent::Started { result: Ok((w, h)), .. }) => {
                c.sink.resized(w, h);
                if !c.cursor_in_video {
                    send(inner, ToAgent::CursorInVideo { display, show: false });
                }
            }
            Ok(FromAgent::Started { result: Err(e), .. }) | Err(e) => {
                let msg = format!("会话切换后重新采集屏幕 {} 失败：{}", display, e);
                tracing::warn!("{msg}")
            },
            _ => {}
        }
    }
}

impl HostPlatform for LinuxPlatform {
    fn list_displays(&self) -> Vec<DisplayInfo> {
        match request(&self.inner, |id| ToAgent::ListDisplays { id }, Duration::from_secs(8)) {
            Ok(FromAgent::Displays { list, .. }) => {
                if !list.is_empty() {
                    *self.inner.last_displays.lock() = list.clone();
                }
                list
            }
            Ok(_) => vec![],
            Err(e) => {
                tracing::warn!("列屏幕失败：{e}");
                // 锁屏之类的原因：把上次的屏幕给主控，它开画面时会看到原因
                if e == LOCKED_HINT {
                    let last = self.inner.last_displays.lock().clone();
                    if !last.is_empty() {
                        return last;
                    }
                    return vec![DisplayInfo { id: 0, name: "屏幕（已锁定）".into(), width: 1920, height: 1080, scale: 1.0, primary: true, is_virtual: false }];
                }
                vec![]
            }
        }
    }

    fn start_capture(&self, display: u32, max_fps: u32, bitrate_kbps: u32, codecs: Vec<VideoCodec>, sink: Arc<dyn FrameSink>) -> Result<CaptureStarted, String> {
        if !codecs.is_empty() && !codecs.contains(&VideoCodec::H264) {
            return Err("被控端只支持 H.264".into());
        }
        self.inner.captures.lock().insert(display, Capture { sink, max_fps, bitrate_kbps, cursor_in_video: true });
        match request(&self.inner, |id| ToAgent::StartCapture { id, display, max_fps, bitrate_kbps }, Duration::from_secs(12)) {
            Ok(FromAgent::Started { result: Ok((width, height)), .. }) => Ok(CaptureStarted { width, height, codec: VideoCodec::H264 }),
            Ok(FromAgent::Started { result: Err(e), .. }) | Err(e) => {
                self.inner.captures.lock().remove(&display);
                Err(e)
            }
            Ok(_) => {
                self.inner.captures.lock().remove(&display);
                Err("画面代理应答不对".into())
            }
        }
    }

    fn stop_capture(&self, display: u32) {
        self.inner.captures.lock().remove(&display);
        if self.inner.agent.lock().is_some() {
            send(&self.inner, ToAgent::StopCapture { display });
        }
        if self.inner.captures.lock().is_empty() && self.inner.unlocked_by_us.lock().is_some() {
            let w = Arc::downgrade(&self.inner);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(10));
                let Some(inner) = w.upgrade() else { return };
                if !inner.captures.lock().is_empty() {
                    return;
                }
                let id = inner.unlocked_by_us.lock().take();
                if let Some(id) = id {
                    let ok = loginctl("lock-session", &id);
                    tracing::info!("远程都结束了，重新锁上会话 {id}（{}）", if ok { "成功" } else { "失败" });
                }
            });
        }
    }

    fn request_keyframe(&self, display: u32) {
        send(&self.inner, ToAgent::RequestKeyframe { display });
    }

    fn set_bitrate(&self, display: u32, kbps: u32) {
        if let Some(c) = self.inner.captures.lock().get_mut(&display) {
            c.bitrate_kbps = kbps;
        }
        send(&self.inner, ToAgent::SetBitrate { display, kbps });
    }

    fn set_cursor_in_video(&self, display: u32, show: bool) {
        if let Some(c) = self.inner.captures.lock().get_mut(&display) {
            c.cursor_in_video = show;
        }
        send(&self.inner, ToAgent::CursorInVideo { display, show });
    }

    fn inject(&self, display: u32, ev: InputEvent) {
        send(&self.inner, ToAgent::Inject { display, ev });
    }

    fn screenshot(&self, display: u32, max_width: u32, max_height: u32) -> Option<ImageData> {
        match request(&self.inner, |id| ToAgent::Screenshot { id, display, max_w: max_width, max_h: max_height }, Duration::from_secs(10)) {
            Ok(FromAgent::Screenshot { image, .. }) => image,
            _ => None,
        }
    }

    fn clipboard_get(&self) -> Option<ClipboardData> {
        match request(&self.inner, |id| ToAgent::ClipboardGet { id }, Duration::from_secs(6)) {
            Ok(FromAgent::Clipboard { data, .. }) => data,
            _ => None,
        }
    }

    fn clipboard_set(&self, d: ClipboardData) {
        send(&self.inner, ToAgent::ClipboardSet { data: d });
    }

    fn start_audio(&self, sink: Arc<dyn AudioSink>) -> Result<(), String> {
        *self.inner.audio.lock() = Some(sink);
        let r = request(&self.inner, |id| ToAgent::StartAudio { id }, Duration::from_secs(8));
        match r {
            Ok(FromAgent::AudioStarted { result: Ok((rate, ch)), .. }) => {
                tracing::info!("开始采集声音（{rate} Hz × {ch}）");
                Ok(())
            }
            Ok(FromAgent::AudioStarted { result: Err(e), .. }) | Err(e) => {
                *self.inner.audio.lock() = None;
                Err(e)
            }
            Ok(_) => {
                *self.inner.audio.lock() = None;
                Err("画面代理应答不对".into())
            }
        }
    }

    fn stop_audio(&self) {
        *self.inner.audio.lock() = None;
        if self.inner.agent.lock().is_some() {
            send(&self.inner, ToAgent::StopAudio);
        }
    }

    fn interactive_user(&self) -> Option<String> {
        session::find_seat().filter(|s| !s.greeter).map(|s| s.user)
    }

    fn capabilities(&self) -> Vec<String> {
        // 有图形会话，或者能开虚拟桌面，才有画面能力
        let target = self.inner.target.lock().clone();
        let can = target.is_some() || vdesktop::availability().is_ok();
        if !can {
            return vec![];
        }
        // 画面代理空闲时每秒至少编一帧（agent.rs），主控端据此判断画面卡死
        let mut c: Vec<String> = ["display", "screenshot", "input", "clipboard", "client_cursor", galaxy_proto::peer::CAP_VIDEO_KEEPALIVE].map(String::from).to_vec();
        if super::audio::library_present() {
            c.push("audio".into());
        }
        c
    }

    fn prepare_desktop(&self, system_user: &str, mode: Option<(u32, u32, u32)>, can_control: bool) {
        let u = SysUser::lookup(system_user);
        *self.inner.wanted_user.lock() = u.as_ref().map(|u| u.uid);
        let wanted = u.as_ref().map(|u| u.uid);
        let current = pick(wanted);
        let usable = current.as_ref().map(|s| !(s.greeter && s.kind == Kind::Wayland)).unwrap_or(false);
        if usable {
            // Wayland 桌面锁着屏：系统禁止远程桌面（X11 能直接看到锁屏，不用管）
            if let Some(s) = current.as_ref().filter(|s| s.kind == Kind::Wayland && !s.virtual_desktop) {
                if session_locked(&s.id) == Some(true) {
                    if can_control && unlock_on_connect() {
                        let ok = loginctl("unlock-session", &s.id);
                        tracing::info!("远程连入，按设置解锁会话 {}（{}）", s.id, if ok { "成功" } else { "失败" });
                        if ok {
                            *self.inner.unlocked_by_us.lock() = Some(s.id.clone());
                            let t = Instant::now();
                            while session_locked(&s.id) == Some(true) && t.elapsed() < Duration::from_secs(3) {
                                std::thread::sleep(Duration::from_millis(100));
                            }
                            // 等桌面撤掉对远程的限制
                            std::thread::sleep(Duration::from_millis(800));
                            *self.inner.failed.lock() = None;
                            if let Some(a) = self.inner.agent.lock().take() {
                                let _ = a.child.lock().kill();
                            }
                        }
                    } else {
                        *self.inner.failed.lock() = Some((s.id.clone(), Instant::now(), LOCKED_HINT.into()));
                        if let Some(a) = self.inner.agent.lock().take() {
                            let _ = a.child.lock().kill();
                        }
                    }
                } else if self.inner.failed.lock().as_ref().is_some_and(|(_, _, w)| w == LOCKED_HINT) {
                    *self.inner.failed.lock() = None;
                }
            }
            *self.inner.target.lock() = current;
            return;
        }
        let Some((w, h, _)) = mode else {
            tracing::info!("没有可看的图形会话，虚拟桌面已关闭（headless-display off）");
            return;
        };
        let Some(u) = u else { return };
        match vdesktop::ensure(&u.name, w, h) {
            Ok(s) => {
                tracing::info!("没有可看的图形会话，使用虚拟桌面：{}", s.describe());
                *self.inner.target.lock() = Some(s.clone());
                *self.inner.failed.lock() = None;
                // 代理还在旧会话（比如 Wayland 登录界面）上：换过来
                let stale = self.inner.agent.lock().as_ref().map(|a| a.session != s).unwrap_or(false);
                if stale {
                    if let Some(a) = self.inner.agent.lock().take() {
                        let _ = a.child.lock().kill();
                    }
                }
                // 桌面刚起来：给窗口管理器一点时间画出来
                std::thread::sleep(Duration::from_millis(1500));
            }
            Err(e) => {
                tracing::warn!("开虚拟桌面失败：{e}");
                *self.inner.failed.lock() = Some((current.map(|s| s.id).unwrap_or_default(), Instant::now(), e));
            }
        }
    }
}
