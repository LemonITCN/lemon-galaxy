//! 服务这一侧的画面能力：管理放在用户会话里的画面代理进程，把内核的请求转给它。
//!
//! 代理按需启动；控制台会话切换（用户注销、换人登录、远程桌面接管）时换到新会话重新启动，
//! 正在进行的画面自动续上。

use super::ipc::{read_msg, write_msg, FromAgent, ToAgent};
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
}

struct Inner {
    agent: Mutex<Option<Agent>>,
    next_id: AtomicU64,
    generation: AtomicU64,
    pending: Mutex<HashMap<u64, mpsc::Sender<FromAgent>>>,
    captures: Mutex<HashMap<u32, Capture>>,
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
                if !inner.captures.lock().is_empty() {
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
    let caps: Vec<(u32, Capture)> = inner.captures.lock().iter().map(|(k, v)| (*k, v.clone())).collect();
    for (display, c) in caps {
        let r = request(inner, |id| ToAgent::StartCapture { id, display, max_fps: c.max_fps, bitrate_kbps: c.bitrate_kbps }, Duration::from_secs(8));
        if let Some(FromAgent::Started { result: Ok((w, h)), .. }) = r {
            c.sink.resized(w, h);
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
        self.inner.captures.lock().insert(display, Capture { sink, max_fps, bitrate_kbps });
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

    fn capabilities(&self) -> Vec<String> {
        vec!["display".into(), "screenshot".into(), "input".into(), "clipboard".into(), "privacy".into()]
    }
}
