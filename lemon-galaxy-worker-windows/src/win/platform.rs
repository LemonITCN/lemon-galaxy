//! 服务这一侧的画面能力：管理放在用户会话里的画面代理进程，把内核的请求转给它。
//!
//! 代理按需启动；控制台会话切换（用户注销、换人登录、远程桌面接管）时换到新会话重新启动，
//! 正在进行的画面自动续上。

use super::ipc::{read_msg, write_msg, FromAgent, ToAgent};
use galaxy_host::audio::AudioSink;
use galaxy_host::platform::{CaptureStarted, FrameSink, HostPlatform};
use galaxy_host::win as hw;
use galaxy_proto::peer::{ClipboardData, DisplayInfo, ImageData, InputEvent, VideoCodec};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;
use windows::Win32::System::RemoteDesktop::WTSGetActiveConsoleSessionId;

struct Agent {
    writer: mpsc::Sender<ToAgent>,
    child: Arc<hw::Child>,
    session: u32,
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
    /// (上次检查时间, 有没有声音输出设备)
    has_audio: Mutex<(Option<std::time::Instant>, bool)>,
    /// (上次检查时间, 虚拟屏驱动在不在)
    has_vd_driver: Mutex<(Option<std::time::Instant>, bool)>,
    /// 现在这个画面代理里的虚拟屏 id（代理退出时驱动随句柄删屏，这里跟着清空）
    virtual_ids: Mutex<std::collections::HashSet<u32>>,
}

pub struct WinPlatform {
    inner: Arc<Inner>,
}

impl WinPlatform {
    pub fn new() -> Arc<Self> {
        let me = Arc::new(Self {
            inner: Arc::new(Inner {
                agent: Mutex::new(None),
                next_id: AtomicU64::new(1),
                generation: AtomicU64::new(1),
                pending: Mutex::new(HashMap::new()),
                captures: Mutex::new(HashMap::new()),
                audio: Mutex::new(None),
                has_audio: Mutex::new((None, false)),
                has_vd_driver: Mutex::new((None, false)),
                virtual_ids: Mutex::new(Default::default()),
            }),
        });
        // 会话切换监视
        let w = Arc::downgrade(&me.inner);
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(1));
            let Some(inner) = w.upgrade() else { break };
            let active = unsafe { WTSGetActiveConsoleSessionId() };
            let stale = inner.agent.lock().as_ref().map(|a| a.session != active && hw::is_system()).unwrap_or(false);
            if stale && active != 0xFFFF_FFFF {
                tracing::info!("控制台会话切换到 {active}，重启画面代理");
                if let Some(a) = inner.agent.lock().take() {
                    a.child.kill();
                }
                if !inner.virtual_ids.lock().is_empty() {
                    tracing::warn!("画面代理重启，虚拟屏随之删除");
                    inner.virtual_ids.lock().clear();
                }
                let audio = inner.audio.lock().is_some();
                if !inner.captures.lock().is_empty() || audio {
                    let _ = ensure_agent(&inner);
                    restart_captures(&inner);
                }
            }
        });
        me
    }
}

fn spawn_agent(inner: &Arc<Inner>) -> std::io::Result<Agent> {
    let exe = std::env::current_exe()?.to_string_lossy().to_string();
    let cmdline = hw::cmdline(&exe, &["__lg-session-agent"]);
    let (in_r, in_w) = hw::pipe(true)?;
    let (out_r, out_w) = hw::pipe(false)?;
    let (err_r, err_w) = hw::pipe(false)?;
    let (session, token) = if hw::is_system() {
        let s = unsafe { WTSGetActiveConsoleSessionId() };
        (s, Some(hw::system_token_in_session(s)?))
    } else {
        (0, None)
    };
    let t = token.as_ref().map(|t| t.raw());
    let child = hw::spawn(hw::Spawn {
        cmdline,
        cwd: None,
        env: hw::env_block(None, &[]),
        token: t,
        stdio: Some([in_r.raw(), out_w.raw(), err_w.raw()]),
        pseudo_console: None,
        desktop: if t.is_some() { Some("winsta0\\default") } else { None },
        no_window: true,
    })?;
    drop(in_r);
    drop(out_w);
    drop(err_w);
    // 代理的错误输出转进服务日志
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(err_r.into_file()).lines().map_while(Result::ok) {
            if !line.trim().is_empty() {
                tracing::warn!("画面代理：{line}");
            }
        }
    });
    let child = Arc::new(child);
    let generation = inner.generation.fetch_add(1, Ordering::Relaxed);
    // 写线程
    let (tx, rx) = mpsc::channel::<ToAgent>();
    let mut w = in_w.into_file();
    std::thread::spawn(move || {
        while let Ok(m) = rx.recv() {
            if write_msg(&mut w, &m).is_err() {
                break;
            }
        }
    });
    // 读线程
    let weak = Arc::downgrade(inner);
    let mut r = std::io::BufReader::with_capacity(256 * 1024, out_r.into_file());
    std::thread::spawn(move || {
        loop {
            let m = match read_msg::<_, FromAgent>(&mut r) {
                Ok(Some(m)) => m,
                _ => break,
            };
            let Some(inner) = weak.upgrade() else { break };
            route(&inner, m);
        }
        if let Some(inner) = weak.upgrade() {
            let mut g = inner.agent.lock();
            if g.as_ref().map(|a| a.generation == generation).unwrap_or(false) {
                *g = None;
                inner.virtual_ids.lock().clear();
                tracing::warn!("画面代理已退出");
            }
        }
    });
    tracing::info!("画面代理已启动（会话 {session}，进程 {}）", child.pid);
    Ok(Agent { writer: tx, child, session, generation })
}

fn route(inner: &Inner, m: FromAgent) {
    if let Some(id) = m.reply_id() {
        if let Some(tx) = inner.pending.lock().remove(&id) {
            let _ = tx.send(m);
        }
        return;
    }
    match m {
        FromAgent::Frame { display, data, pts_us, keyframe } => {
            let sink = inner.captures.lock().get(&display).map(|c| c.sink.clone());
            if let Some(s) = sink {
                s.push(data, pts_us, keyframe);
            }
        }
        FromAgent::Resized { display, width, height } => {
            let sink = inner.captures.lock().get(&display).map(|c| c.sink.clone());
            if let Some(s) = sink {
                s.resized(width, height);
            }
        }
        FromAgent::Cursor { display, x, y, visible } => {
            let sink = inner.captures.lock().get(&display).map(|c| c.sink.clone());
            if let Some(s) = sink {
                s.cursor(x, y, visible);
            }
        }
        FromAgent::Audio { pcm, rate, channels } => {
            let sink = inner.audio.lock().clone();
            if let Some(s) = sink {
                s.push_pcm(&pcm, rate, channels);
            }
        }
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

fn ensure_agent(inner: &Arc<Inner>) -> std::io::Result<()> {
    let mut g = inner.agent.lock();
    let alive = g.as_ref().map(|a| a.child.try_wait().is_none()).unwrap_or(false);
    if !alive {
        *g = Some(spawn_agent(inner)?);
    }
    Ok(())
}

fn send(inner: &Arc<Inner>, m: ToAgent) -> bool {
    if ensure_agent(inner).is_err() {
        return false;
    }
    inner.agent.lock().as_ref().map(|a| a.writer.send(m).is_ok()).unwrap_or(false)
}

fn request(inner: &Arc<Inner>, make: impl FnOnce(u64) -> ToAgent, timeout: Duration) -> Option<FromAgent> {
    let id = inner.next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = mpsc::channel();
    inner.pending.lock().insert(id, tx);
    if !send(inner, make(id)) {
        inner.pending.lock().remove(&id);
        return None;
    }
    let r = rx.recv_timeout(timeout).ok();
    inner.pending.lock().remove(&id);
    r
}

fn restart_captures(inner: &Arc<Inner>) {
    if inner.audio.lock().is_some() {
        if let Some(FromAgent::AudioStarted { result: Err(e), .. }) = request(inner, |id| ToAgent::StartAudio { id }, Duration::from_secs(8)) {
            tracing::warn!("会话切换后重新采集声音失败：{e}");
        }
    }
    let caps: Vec<(u32, Capture)> = inner.captures.lock().iter().map(|(k, v)| (*k, v.clone())).collect();
    for (display, c) in caps {
        let r = request(inner, |id| ToAgent::StartCapture { id, display, max_fps: c.max_fps, bitrate_kbps: c.bitrate_kbps }, Duration::from_secs(8));
        if let Some(FromAgent::Started { result: Ok((w, h)), .. }) = r {
            c.sink.resized(w, h);
            if !c.cursor_in_video {
                send(inner, ToAgent::CursorInVideo { display, show: false });
            }
        }
    }
}

impl HostPlatform for WinPlatform {
    fn list_displays(&self) -> Vec<DisplayInfo> {
        match request(&self.inner, |id| ToAgent::ListDisplays { id }, Duration::from_secs(5)) {
            Some(FromAgent::Displays { list, .. }) => list,
            _ => vec![],
        }
    }

    fn start_capture(&self, display: u32, max_fps: u32, bitrate_kbps: u32, codecs: Vec<VideoCodec>, sink: Arc<dyn FrameSink>) -> Result<CaptureStarted, String> {
        if !codecs.is_empty() && !codecs.contains(&VideoCodec::H264) {
            return Err("被控端只支持 H.264".into());
        }
        self.inner.captures.lock().insert(display, Capture { sink, max_fps, bitrate_kbps, cursor_in_video: true });
        match request(&self.inner, |id| ToAgent::StartCapture { id, display, max_fps, bitrate_kbps }, Duration::from_secs(10)) {
            Some(FromAgent::Started { result: Ok((width, height)), .. }) => Ok(CaptureStarted { width, height, codec: VideoCodec::H264 }),
            Some(FromAgent::Started { result: Err(e), .. }) => {
                self.inner.captures.lock().remove(&display);
                Err(e)
            }
            _ => {
                self.inner.captures.lock().remove(&display);
                Err("画面代理没有响应".into())
            }
        }
    }

    fn stop_capture(&self, display: u32) {
        self.inner.captures.lock().remove(&display);
        let running = self.inner.agent.lock().is_some();
        if running {
            send(&self.inner, ToAgent::StopCapture { display });
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
            Some(FromAgent::Screenshot { image, .. }) => image,
            _ => None,
        }
    }

    fn clipboard_get(&self) -> Option<ClipboardData> {
        match request(&self.inner, |id| ToAgent::ClipboardGet { id }, Duration::from_secs(5)) {
            Some(FromAgent::Clipboard { data, .. }) => data,
            _ => None,
        }
    }

    fn clipboard_set(&self, d: ClipboardData) {
        send(&self.inner, ToAgent::ClipboardSet { data: d });
    }

    fn set_privacy(&self, on: bool) -> bool {
        if !on && self.inner.agent.lock().is_none() {
            return true;
        }
        match request(&self.inner, |id| ToAgent::Privacy { id, on }, Duration::from_secs(8)) {
            Some(FromAgent::PrivacyResult { result: Ok(()), .. }) => true,
            Some(FromAgent::PrivacyResult { result: Err(e), .. }) => {
                tracing::warn!("隐私屏：{e}");
                false
            }
            _ => false,
        }
    }

    fn start_audio(&self, sink: Arc<dyn AudioSink>) -> Result<(), String> {
        *self.inner.audio.lock() = Some(sink);
        match request(&self.inner, |id| ToAgent::StartAudio { id }, Duration::from_secs(8)) {
            Some(FromAgent::AudioStarted { result: Ok((rate, ch)), .. }) => {
                tracing::info!("开始采集声音（{rate} Hz × {ch}）");
                Ok(())
            }
            Some(FromAgent::AudioStarted { result: Err(e), .. }) => {
                *self.inner.audio.lock() = None;
                Err(e)
            }
            _ => {
                *self.inner.audio.lock() = None;
                Err("画面代理没有响应".into())
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
        console_user()
    }

    fn virtual_display_max(&self) -> u32 {
        // 驱动在不在：最多 10 秒查一次（握手时都会问）
        let mut g = self.inner.has_vd_driver.lock();
        if g.0.map(|t| t.elapsed() > Duration::from_secs(10)).unwrap_or(true) {
            *g = (Some(std::time::Instant::now()), super::vdisplay::driver_present());
        }
        if g.1 {
            super::vdisplay::CONTROLLER_MAX
        } else {
            0
        }
    }

    fn add_virtual_display(&self, width: u32, height: u32, refresh: u32) -> Result<DisplayInfo, String> {
        match request(&self.inner, |id| ToAgent::VirtualAdd { id, width, height, refresh }, Duration::from_secs(20)) {
            Some(FromAgent::VirtualAdded { result: Ok(d), .. }) => {
                self.inner.virtual_ids.lock().insert(d.id);
                Ok(d)
            }
            Some(FromAgent::VirtualAdded { result: Err(e), .. }) => Err(e),
            _ => Err("画面代理没有响应".into()),
        }
    }

    fn remove_virtual_display(&self, id: u32) -> Result<(), String> {
        if !self.inner.virtual_ids.lock().remove(&id) {
            return Ok(());
        }
        match request(&self.inner, |rid| ToAgent::VirtualRemove { id: rid, display: id }, Duration::from_secs(20)) {
            Some(FromAgent::VirtualRemoved { result, .. }) => result,
            _ => Err("画面代理没有响应".into()),
        }
    }

    fn virtual_display_ids(&self) -> Vec<u32> {
        self.inner.virtual_ids.lock().iter().copied().collect()
    }

    fn needs_headless_display(&self) -> bool {
        // 测试用：当作本机没有亮着的显示器（验证无头兜底屏的加删，不用真合上笔记本）
        if std::env::var("LEMON_GALAXY_TEST_HEADLESS").map(|v| v == "1").unwrap_or(false) {
            return true;
        }
        match request(&self.inner, |id| ToAgent::VirtualState { id }, Duration::from_secs(10)) {
            Some(FromAgent::VirtualState { ids, needs_headless, .. }) => {
                // 顺便对账：代理里实际有的虚拟屏
                let mut g = self.inner.virtual_ids.lock();
                g.retain(|x| ids.contains(x));
                needs_headless
            }
            _ => false,
        }
    }

    fn capabilities(&self) -> Vec<String> {
        let mut c: Vec<String> = vec!["display".into(), "screenshot".into(), "input".into(), "clipboard".into(), "privacy".into(), "client_cursor".into()];
        if self.virtual_display_max() > 0 {
            c.push("virtual_display".into());
        }
        // 有没有声音输出设备：最多 30 秒查一次（服务器、虚拟机常常没有）
        let has = {
            let mut g = self.inner.has_audio.lock();
            if g.0.map(|t| t.elapsed() > Duration::from_secs(30)).unwrap_or(true) {
                *g = (Some(std::time::Instant::now()), super::audio::has_render_device());
            }
            g.1
        };
        if has {
            c.push("audio".into());
        }
        c
    }
}

/// 控制台会话里登录的用户（域\用户名）；没人登录（登录界面）时为空。
fn console_user() -> Option<String> {
    use windows::core::PWSTR;
    use windows::Win32::System::RemoteDesktop::{WTSDomainName, WTSFreeMemory, WTSQuerySessionInformationW, WTSUserName, WTS_CURRENT_SERVER_HANDLE, WTS_INFO_CLASS};
    let session = unsafe { WTSGetActiveConsoleSessionId() };
    if session == 0xFFFF_FFFF {
        return None;
    }
    let query = |class: WTS_INFO_CLASS| -> Option<String> {
        unsafe {
            let mut buf = PWSTR::null();
            let mut len = 0u32;
            WTSQuerySessionInformationW(Some(WTS_CURRENT_SERVER_HANDLE), session, class, &mut buf, &mut len).ok()?;
            let s = buf.to_string().ok();
            WTSFreeMemory(buf.0 as _);
            s.filter(|s| !s.is_empty())
        }
    };
    let user = query(WTSUserName)?;
    Some(match query(WTSDomainName) {
        Some(d) => format!("{d}\\{user}"),
        None => user,
    })
}
