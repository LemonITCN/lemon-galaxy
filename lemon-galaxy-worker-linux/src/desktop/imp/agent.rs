//! 画面代理（`__lg-session-agent`）：以图形会话的用户身份、带着会话的环境变量运行（服务负责放进来），
//! 从标准输入读服务的请求，把编码好的画面和应答写到标准输出；服务关掉管道时退出。
//! 日志写标准错误，服务转进自己的日志。

use super::backend::{draw_cursor, Backend, Controller, CursorImage, CursorPos, Frame, Monitor};
use super::encode::{jpeg, Request, VideoEncoder};
use super::ipc::{read_msg, write_msg, FromAgent, ToAgent};
use galaxy_host::pacing::Pacer;
use galaxy_proto::peer::{ClipOffer, ClipboardData, ImageData, InputEvent};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct CaptureCtl {
    stop: AtomicBool,
    keyframe: AtomicBool,
    bitrate: AtomicU32,
    reconfig: AtomicBool,
    cursor_in_video: AtomicBool,
    /// 要一张截图：采集线程下一轮把当前画面（含光标）交出来。正在采集的屏不能再另开一路流
    /// （Mutter / 门户的一路流同时只给一个接收方），也省得重复截屏。
    shot: parking_lot::Mutex<Option<mpsc::Sender<Frame>>>,
}

impl CaptureCtl {
    fn new(bitrate: u32) -> Self {
        Self { stop: AtomicBool::new(false), keyframe: AtomicBool::new(true), bitrate: AtomicU32::new(bitrate), reconfig: AtomicBool::new(false), cursor_in_video: AtomicBool::new(true), shot: parking_lot::Mutex::new(None) }
    }
}

enum UiJob {
    Inject(u32, InputEvent),
    ClipboardGet(u64),
    ClipboardSet(ClipboardData),
    ClipOffer(u64, ClipOffer),
}

/// 退出前要做的收尾：把输入法切回原来的（ibus.rs）、卸掉复制粘贴文件的虚拟目录。只做一次，多处同时调时后来的等着。
static CLIP: std::sync::OnceLock<Arc<super::clipoffer::Agent>> = std::sync::OnceLock::new();

pub fn cleanup() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        super::ibus::shutdown();
        if let Some(c) = CLIP.get() {
            c.shutdown();
        }
    });
}

/// 收尾后退出进程（Mutter 会话关闭等地方用它代替 std::process::exit）
pub fn exit(code: i32) -> ! {
    cleanup();
    std::process::exit(code)
}

/// SIGTERM / SIGINT / SIGHUP（服务停止时 systemd 会发给整个进程组）：收尾后退出。
/// 信号处理函数只往管道写一个字节，收尾在单独的线程里做。
fn exit_on_signals() {
    static FD: AtomicI32 = AtomicI32::new(-1);
    extern "C" fn on_signal(_: libc::c_int) {
        let fd = FD.load(Ordering::Relaxed);
        if fd >= 0 {
            unsafe { libc::write(fd, b"x".as_ptr() as *const libc::c_void, 1) };
        }
    }
    let mut fds = [0 as libc::c_int; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return;
    }
    FD.store(fds[1], Ordering::Relaxed);
    let r = fds[0];
    let _ = std::thread::Builder::new().name("signals".into()).spawn(move || {
        let mut b = [0u8; 1];
        if unsafe { libc::read(r, b.as_mut_ptr() as *mut libc::c_void, 1) } == 1 {
            eprintln!("收到退出信号，收尾后退出");
            exit(0);
        }
    });
    for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        unsafe { libc::signal(sig, on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t) };
    }
}

fn log(out: &SyncSender<FromAgent>, error: bool, text: String) {
    let _ = out.try_send(FromAgent::Log { error, text });
}

/// 按环境变量 LG_BACKEND（服务按会话类型设）选后端。
fn open_backend() -> Result<Arc<dyn Backend>, String> {
    let want = std::env::var("LG_BACKEND").unwrap_or_default();
    let is_virtual = std::env::var("LG_VIRTUAL").is_ok_and(|v| v == "1");
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    let kind = match want.as_str() {
        "x11" | "mutter" | "portal" => want.clone(),
        _ if wayland && (desktop.contains("gnome") || desktop.contains("ubuntu") || desktop.contains("unity") || desktop.contains("pop")) => "mutter".into(),
        _ if wayland => "portal".into(),
        _ => "x11".into(),
    };
    match kind.as_str() {
        "mutter" => super::wayland::mutter::MutterBackend::new(is_virtual).map(|b| Arc::new(b) as _),
        "portal" => super::wayland::portal::PortalBackend::new().map(|b| Arc::new(b) as _),
        _ => super::x11::X11Backend::new(is_virtual).map(|b| Arc::new(b) as _),
    }
}

pub fn main() -> i32 {
    let (out_tx, out_rx) = mpsc::sync_channel::<FromAgent>(16);
    let writer = std::thread::spawn(move || {
        let stdout = std::io::stdout();
        let mut w = std::io::BufWriter::with_capacity(512 * 1024, stdout.lock());
        while let Ok(m) = out_rx.recv() {
            if write_msg(&mut w, &m).is_err() {
                break;
            }
        }
    });
    let backend = match open_backend() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("后端启动失败：{e}");
            let _ = out_tx.send(FromAgent::Hello { backend: "none".into(), detail: e.clone() });
            let _ = out_tx.send(FromAgent::Lost { reason: e });
            drop(out_tx);
            let _ = writer.join();
            return 1;
        }
    };
    let _ = out_tx.send(FromAgent::Hello { backend: backend.name().into(), detail: format!("DISPLAY={} WAYLAND_DISPLAY={} 桌面={}", std::env::var("DISPLAY").unwrap_or_default(), std::env::var("WAYLAND_DISPLAY").unwrap_or_default(), std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default()) });

    // 复制粘贴文件（粘贴时才传）：虚拟目录 / 兜底下载、经管道向服务读内容
    let clip = super::clipoffer::Agent::new(out_tx.clone());
    let _ = CLIP.set(clip.clone());
    exit_on_signals();

    // 键鼠与剪贴板一个线程（控制器在线程里建：X11 连接、D-Bus 会话都归它）
    let (ui_tx, ui_rx) = mpsc::channel::<UiJob>();
    {
        let out = out_tx.clone();
        let backend = backend.clone();
        let clip = clip.clone();
        std::thread::spawn(move || {
            let mut ctl: Option<Box<dyn Controller>> = None;
            let mut monitors: Vec<Monitor> = vec![];
            let mut checked = Instant::now() - Duration::from_secs(60);
            let mut warned = false;
            while let Ok(j) = ui_rx.recv() {
                if ctl.is_none() {
                    match backend.controller() {
                        Ok(c) => ctl = Some(c),
                        Err(e) => {
                            if !warned {
                                log(&out, true, format!("键鼠不可用：{e}"));
                                warned = true;
                            }
                        }
                    }
                }
                match j {
                    UiJob::Inject(d, ev) => {
                        if checked.elapsed() > Duration::from_secs(2) || !monitors.iter().any(|m| m.id == d) {
                            monitors = backend.monitors();
                            checked = Instant::now();
                        }
                        let m = monitors.iter().find(|m| m.id == d).or_else(|| monitors.first()).cloned();
                        if let (Some(c), Some(m)) = (ctl.as_mut(), m) {
                            c.inject(&m, ev);
                        }
                    }
                    UiJob::ClipboardGet(id) => {
                        // 指向我们自己虚拟目录 / 暂存目录的不算（是主控端复制过来的）
                        let data = clip.filter(ctl.as_mut().and_then(|c| c.clipboard_get()));
                        let _ = out.send(FromAgent::Clipboard { id, data });
                    }
                    UiJob::ClipOffer(id, offer) => {
                        let result = match ctl.as_mut() {
                            None => Err("剪贴板不可用（连不上桌面）".to_string()),
                            Some(c) if !c.clipboard_available() => Err("这个桌面暂不支持远程剪贴板".to_string()),
                            Some(c) => {
                                let n = offer.entries.len();
                                match clip.prepare(offer) {
                                    super::clipoffer::FileClip::Paths(p) => {
                                        log(&out, false, format!("复制了 {n} 项，放进剪贴板（{}…），粘贴时才传", p.first().cloned().unwrap_or_default()));
                                        c.clipboard_set(ClipboardData::Files(p));
                                        Ok(())
                                    }
                                    super::clipoffer::FileClip::Lazy(f) => c.clipboard_set_lazy(f),
                                }
                            }
                        };
                        let _ = out.send(FromAgent::ClipOfferSet { id, result });
                    }
                    UiJob::ClipboardSet(d) => {
                        if let Some(c) = ctl.as_mut() {
                            c.clipboard_set(d);
                        }
                    }
                }
            }
        });
    }

    let mut captures: HashMap<u32, Arc<CaptureCtl>> = HashMap::new();
    let mut audio_stop: Option<Arc<AtomicBool>> = None;
    // 有画面在看时不让会话闲置锁屏（虚拟桌面不用）
    let mut idle_inhibit: Option<super::inhibit::IdleInhibit> = None;
    let want_inhibit = std::env::var("LG_VIRTUAL").map(|v| v != "1").unwrap_or(true);
    let stdin = std::io::stdin();
    let mut r = std::io::BufReader::new(stdin.lock());
    loop {
        let m = match read_msg::<_, ToAgent>(&mut r) {
            Ok(Some(m)) => m,
            _ => break,
        };
        match m {
            ToAgent::Ping { id } => {
                let _ = out_tx.send(FromAgent::Pong { id });
            }
            ToAgent::ListDisplays { id } => {
                let list = backend.monitors().iter().map(|m| m.info()).collect();
                let _ = out_tx.send(FromAgent::Displays { id, list });
            }
            ToAgent::StartCapture { id, display, max_fps, bitrate_kbps } => {
                if let Some(old) = captures.remove(&display) {
                    old.stop.store(true, Ordering::Relaxed);
                }
                let ctl = Arc::new(CaptureCtl::new(bitrate_kbps));
                captures.insert(display, ctl.clone());
                if want_inhibit && idle_inhibit.is_none() {
                    match super::inhibit::IdleInhibit::start() {
                        Ok(i) => idle_inhibit = Some(i),
                        Err(e) => log(&out_tx, false, format!("远程期间没能阻止自动锁屏：{e}")),
                    }
                }
                let (out, backend) = (out_tx.clone(), backend.clone());
                std::thread::spawn(move || capture_thread(id, display, max_fps.clamp(1, 60), ctl, out, backend));
            }
            ToAgent::StopCapture { display } => {
                if let Some(c) = captures.remove(&display) {
                    c.stop.store(true, Ordering::Relaxed);
                }
                if captures.is_empty() {
                    idle_inhibit = None;
                }
            }
            ToAgent::RequestKeyframe { display } => {
                if let Some(c) = captures.get(&display) {
                    c.keyframe.store(true, Ordering::Relaxed);
                }
            }
            ToAgent::SetBitrate { display, kbps } => {
                if let Some(c) = captures.get(&display) {
                    c.bitrate.store(kbps, Ordering::Relaxed);
                    c.reconfig.store(true, Ordering::Relaxed);
                }
            }
            ToAgent::CursorInVideo { display, show } => {
                if let Some(c) = captures.get(&display) {
                    c.cursor_in_video.store(show, Ordering::Relaxed);
                }
            }
            ToAgent::Inject { display, ev } => {
                let _ = ui_tx.send(UiJob::Inject(display, ev));
            }
            ToAgent::ClipboardGet { id } => {
                let _ = ui_tx.send(UiJob::ClipboardGet(id));
            }
            ToAgent::ClipboardSet { data } => {
                let _ = ui_tx.send(UiJob::ClipboardSet(data));
            }
            ToAgent::ClipOffer { id, offer } => {
                let _ = ui_tx.send(UiJob::ClipOffer(id, offer));
            }
            m @ (ToAgent::ClipOpened { .. } | ToAgent::ClipData { .. }) => clip.deliver(m),
            ToAgent::Quit => break,
            ToAgent::Screenshot { id, display, max_w, max_h } => {
                let (out, backend) = (out_tx.clone(), backend.clone());
                let active = captures.get(&display).cloned();
                std::thread::spawn(move || {
                    // 这块屏正在采集：要采集线程的当前帧
                    let from_capture = active.and_then(|c| {
                        let (tx, rx) = mpsc::channel();
                        *c.shot.lock() = Some(tx);
                        rx.recv_timeout(Duration::from_secs(3)).ok()
                    });
                    let image = match from_capture {
                        Some(f) => jpeg(&f.data, f.width, f.height, max_w, max_h).ok().map(|(data, width, height)| ImageData { mime: "image/jpeg".into(), data, width, height }),
                        None => screenshot(&*backend, display, max_w, max_h),
                    };
                    let _ = out.send(FromAgent::Screenshot { id, image });
                });
            }
            ToAgent::StartAudio { id } => {
                if let Some(s) = audio_stop.take() {
                    s.store(true, Ordering::Relaxed);
                }
                let stop = Arc::new(AtomicBool::new(false));
                let out = out_tx.clone();
                let result = super::audio::start(stop.clone(), move |pcm, rate, channels| {
                    let _ = out.send(FromAgent::Audio { pcm, rate, channels });
                });
                if result.is_ok() {
                    audio_stop = Some(stop);
                }
                let _ = out_tx.send(FromAgent::AudioStarted { id, result });
            }
            ToAgent::StopAudio => {
                if let Some(s) = audio_stop.take() {
                    s.store(true, Ordering::Relaxed);
                }
            }
        }
    }
    // 服务关了管道（退出、换会话）：停掉采集与声音后直接退出进程。
    // 不等写线程：键鼠、采集、声音线程还拿着输出通道，写线程不会自己结束。
    for c in captures.values() {
        c.stop.store(true, Ordering::Relaxed);
    }
    if let Some(s) = audio_stop.take() {
        s.store(true, Ordering::Relaxed);
    }
    // 输入法切回原来的、卸掉虚拟目录（服务关管道、发 Quit 都走到这里）
    cleanup();
    drop(out_tx);
    drop(writer);
    std::thread::sleep(Duration::from_millis(200));
    std::process::exit(0)
}

fn find_monitor(backend: &dyn Backend, display: u32) -> Option<Monitor> {
    let list = backend.monitors();
    list.iter().find(|m| m.id == display).or_else(|| list.iter().find(|m| m.primary)).cloned()
}

fn screenshot(backend: &dyn Backend, display: u32, max_w: u32, max_h: u32) -> Option<ImageData> {
    // 刚停掉的采集可能还没放开 Mutter / 门户的流：失败时稍等再试一次
    shot_once(backend, display, max_w, max_h).or_else(|| {
        std::thread::sleep(Duration::from_millis(600));
        shot_once(backend, display, max_w, max_h)
    })
}

fn shot_once(backend: &dyn Backend, display: u32, max_w: u32, max_h: u32) -> Option<ImageData> {
    let m = find_monitor(backend, display)?;
    let mut cap = backend.capturer(&m).ok()?;
    let mut f = Frame::default();
    let t0 = Instant::now();
    while f.width == 0 && t0.elapsed() < Duration::from_secs(3) {
        cap.next(&mut f, Duration::from_millis(200)).ok()?;
    }
    if f.width == 0 {
        return None;
    }
    if !cap.cursor_embedded() {
        if let Some((pos, Some(img))) = cap.cursor() {
            draw_cursor(&mut f, &img, pos);
        }
    }
    let (data, width, height) = jpeg(&f.data, f.width, f.height, max_w, max_h).ok()?;
    Some(ImageData { mime: "image/jpeg".into(), data, width, height })
}

fn capture_thread(id: u64, display: u32, fps: u32, ctl: Arc<CaptureCtl>, out: SyncSender<FromAgent>, backend: Arc<dyn Backend>) {
    let Some(mon) = find_monitor(&*backend, display) else {
        let _ = out.send(FromAgent::Started { id, result: Err("找不到这块屏幕".into()) });
        return;
    };
    let mut cap = match backend.capturer(&mon) {
        Ok(c) => c,
        Err(e) => {
            let _ = out.send(FromAgent::Started { id, result: Err(e) });
            return;
        }
    };
    log(&out, false, format!("屏幕 {display} 开始采集（{}，{}x{}，{fps} 帧）", cap.kind(), mon.width, mon.height));
    let request = Request::from_env();
    let interval = Duration::from_millis(1000 / fps as u64);
    let t0 = Instant::now();
    let mut frame = Frame::default();
    let mut work = Frame::default();
    let mut enc: Option<VideoEncoder> = None;
    let mut started = false;
    let mut have = false;
    // 编码节奏（画面停下后补编、保活）与交给编码器的码率（按实际帧率放大），见 galaxy_host::pacing
    let mut pacer = Pacer::new(fps, ctl.bitrate.load(Ordering::Relaxed));
    let mut cursor_img: Option<CursorImage> = None;
    let mut last_cursor: Option<CursorPos> = None;
    let mut last_report: Option<CursorPos> = None;
    let mut last_in_video = true;
    let mut failures = 0u32;
    let mut hw_allowed = true;
    let mut enc_errors = 0u32;
    let mut applied_in_video = true;
    while !ctl.stop.load(Ordering::Relaxed) {
        let tick = Instant::now();
        let in_video = ctl.cursor_in_video.load(Ordering::Relaxed);
        if in_video != applied_in_video {
            cap.set_cursor_in_video(in_video);
            applied_in_video = in_video;
        }
        let res = cap.next(&mut frame, interval);
        let changed = match res {
            Ok(c) => {
                failures = 0;
                if c {
                    have = true;
                }
                c
            }
            Err(e) => {
                failures += 1;
                log(&out, true, format!("截屏失败（{}）：{e}", cap.kind()));
                if failures >= 4 {
                    let reason = format!("屏幕 {display} 连续截屏失败：{e}");
                    if !started {
                        let _ = out.send(FromAgent::Started { id, result: Err(reason.clone()) });
                    }
                    let _ = out.send(FromAgent::Lost { reason });
                    return;
                }
                std::thread::sleep(Duration::from_millis(300));
                let m = find_monitor(&*backend, display).unwrap_or_else(|| mon.clone());
                if let Ok(c) = backend.capturer(&m) {
                    cap = c;
                    applied_in_video = true;
                }
                ctl.keyframe.store(true, Ordering::Relaxed);
                continue;
            }
        };
        if have {
            let want = ctl.shot.lock().take();
            if let Some(tx) = want {
                let mut f = frame.clone();
                if !cap.cursor_embedded() {
                    if let Some((p, img)) = cap.cursor() {
                        if let Some(i) = img {
                            cursor_img = Some(i);
                        }
                        if let Some(i) = cursor_img.as_ref() {
                            draw_cursor(&mut f, i, p);
                        }
                    }
                }
                let _ = tx.send(f);
            }
        }
        if !have {
            if !started && t0.elapsed() > Duration::from_secs(8) {
                let _ = out.send(FromAgent::Started { id, result: Err("拿不到屏幕画面".into()) });
                return;
            }
            continue;
        }
        // 编码器：没有或截屏尺寸变了就重建；改码率时硬件编码器直接改，软件编码器重建
        let size = (frame.width, frame.height);
        if ctl.reconfig.swap(false, Ordering::Relaxed) {
            pacer.set_target(ctl.bitrate.load(Ordering::Relaxed));
        }
        let mut rebuild = enc.as_ref().map(|e| e.source_size() != size).unwrap_or(true);
        if !rebuild {
            if let (Some(e), Some(k)) = (enc.as_mut(), pacer.retune(Instant::now())) {
                rebuild = !e.set_bitrate(k);
            }
        }
        if rebuild {
            let kbps = pacer.target();
            let req = if hw_allowed { request.clone() } else { Request::software() };
            match VideoEncoder::new(size.0, size.1, fps, kbps, &req) {
                Ok((e, notes)) => {
                    let old = enc.take();
                    if old.as_ref().map(|o| o.name() != e.name() || (o.width, o.height) != (e.width, e.height)).unwrap_or(true) {
                        let why = if notes.is_empty() { String::new() } else { format!("；没用上的：{}", notes.join("；")) };
                        log(&out, false, format!("屏幕 {display} 编码器：{}（{}x{}，{kbps} kbps{why}）", e.name(), e.width, e.height));
                    }
                    if started && old.as_ref().map(|o| (o.width, o.height) != (e.width, e.height)).unwrap_or(false) {
                        let _ = out.send(FromAgent::Resized { display, width: e.width, height: e.height });
                    }
                    pacer.encoder_ready(kbps, e.scales_bitrate());
                    enc = Some(e);
                    enc_errors = 0;
                    ctl.keyframe.store(true, Ordering::Relaxed);
                }
                Err(e) => {
                    if started {
                        log(&out, true, format!("屏幕 {display} 编码器启动失败：{e}"));
                    } else {
                        let _ = out.send(FromAgent::Started { id, result: Err(format!("编码器启动失败：{e}")) });
                    }
                    return;
                }
            }
        }
        let Some(e) = enc.as_mut() else { continue };
        if !started {
            let _ = out.send(FromAgent::Started { id, result: Ok((e.width, e.height)) });
            started = true;
        }
        let need_key = ctl.keyframe.swap(false, Ordering::Relaxed);
        let embedded = cap.cursor_embedded();
        let cursor = cap.cursor().map(|(p, img)| {
            if let Some(i) = img {
                cursor_img = Some(i);
            }
            p
        });
        if !in_video {
            if let Some(p) = cursor {
                if Some(p) != last_report {
                    last_report = Some(p);
                    let _ = out.try_send(FromAgent::Cursor { display, x: p.x, y: p.y, visible: p.visible });
                }
            }
        } else {
            last_report = None;
        }
        let toggled = in_video != last_in_video;
        last_in_video = in_video;
        let cursor_moved = in_video && !embedded && cursor != last_cursor;
        // 画面变了、画面里的光标动了、要关键帧时立即编码；停下后按节奏补编几次，之后每秒保活一次
        let now = Instant::now();
        if let Some(why) = pacer.decide(changed || need_key || toggled || cursor_moved, now) {
            last_cursor = cursor;
            // 画面里要画光标：画在副本上（frame 要留着和下一帧比较）
            let src: &[u8] = match (in_video && !embedded, cursor, cursor_img.as_ref()) {
                (true, Some(p), Some(img)) if p.visible => {
                    work.width = frame.width;
                    work.height = frame.height;
                    work.data.clear();
                    work.data.extend_from_slice(&frame.data);
                    draw_cursor(&mut work, img, p);
                    &work.data
                }
                _ => &frame.data,
            };
            match e.encode(src, need_key, t0.elapsed().as_micros() as u64) {
                Ok(frames) => {
                    enc_errors = 0;
                    pacer.encoded(why, now, frames.iter().map(|f| f.data.len()).sum());
                    for f in frames {
                        if out.send(FromAgent::Frame { display, data: f.data, pts_us: t0.elapsed().as_micros() as u64, keyframe: f.keyframe }).is_err() {
                            return;
                        }
                    }
                }
                Err(err) => {
                    pacer.encoded(why, now, 0);
                    log(&out, true, format!("编码失败（{}）：{err}", e.name()));
                    if need_key {
                        ctl.keyframe.store(true, Ordering::Relaxed);
                    }
                    enc_errors += 1;
                    if e.is_hardware() && enc_errors >= 3 {
                        log(&out, true, format!("屏幕 {display} 硬件编码连续出错，改用软件编码"));
                        hw_allowed = false;
                        enc = None;
                    }
                }
            }
        }
        let spent = tick.elapsed();
        if changed && spent < interval {
            std::thread::sleep(interval - spent);
        }
    }
    log(&out, false, format!("屏幕 {display} 停止采集"));
}

/// 自检：截一帧、各编码器试编，打印结果（selftest-capture 用，在会话环境里运行）。
pub fn selftest(secs: u64) -> i32 {
    let backend = match open_backend() {
        Ok(b) => b,
        Err(e) => {
            println!("后端：失败 {e}");
            return 1;
        }
    };
    println!("后端：{}", backend.name());
    let mons = backend.monitors();
    for m in &mons {
        println!("  屏幕 {}：{} {}x{} @({},{}){}{}", m.id, m.name, m.width, m.height, m.x, m.y, if m.primary { " 主屏" } else { "" }, if m.is_virtual { " 虚拟" } else { "" });
    }
    let Some(m) = mons.iter().find(|m| m.primary).or(mons.first()).cloned() else {
        println!("没有屏幕");
        return 1;
    };
    let mut cap = match backend.capturer(&m) {
        Ok(c) => c,
        Err(e) => {
            println!("采集器：失败 {e}");
            return 1;
        }
    };
    let mut f = Frame::default();
    let t = Instant::now();
    while f.width == 0 && t.elapsed() < Duration::from_secs(5) {
        if let Err(e) = cap.next(&mut f, Duration::from_millis(200)) {
            println!("截屏：失败 {e}");
            return 1;
        }
    }
    println!("截屏：{}，{}x{}，首帧 {:.1} ms", cap.kind(), f.width, f.height, t.elapsed().as_secs_f64() * 1000.0);
    let nonblack = f.data.chunks(4).filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8).count();
    println!("  非黑像素 {:.1}%", nonblack as f64 * 100.0 / (f.width as f64 * f.height as f64).max(1.0));
    let mut n = 0u32;
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(secs.min(10)) {
        if cap.next(&mut f, Duration::from_millis(33)).unwrap_or(false) {
            n += 1;
        }
    }
    println!("  {secs} 秒内画面变化 {n} 次");
    if let Some((p, img)) = cap.cursor() {
        println!("光标：({}, {}) {}，图像 {}", p.x, p.y, if p.visible { "可见" } else { "不在这块屏" }, img.map(|i| format!("{}x{}", i.width, i.height)).unwrap_or_else(|| "无".into()));
    }
    let mut reqs = vec![("自动".to_string(), Request::from_env())];
    reqs.extend(super::encode::hardware_requests());
    reqs.push(("软件".into(), Request::software()));
    for (label, req) in reqs {
        let t = Instant::now();
        match VideoEncoder::new(f.width, f.height, 30, 8000, &req) {
            Ok((mut e, notes)) => {
                let made = t.elapsed();
                let mut times = vec![];
                let mut bytes = 0usize;
                for i in 0..30 {
                    let t = Instant::now();
                    let r = e.encode(&f.data, i == 0, i * 33_333);
                    times.push(t.elapsed().as_secs_f64() * 1000.0);
                    if let Ok(v) = r {
                        bytes += v.iter().map(|x| x.data.len()).sum::<usize>();
                    }
                }
                times.sort_by(|a, b| a.partial_cmp(b).unwrap());
                println!("编码 [{label}]：{}，建立 {:.0} ms，每帧中位 {:.1} ms、最慢 {:.1} ms，30 帧 {} KB{}", e.name(), made.as_secs_f64() * 1000.0, times[15], times[29], bytes / 1024, if notes.is_empty() { String::new() } else { format!("（{}）", notes.join("；")) });
            }
            Err(e) => println!("编码 [{label}]：不可用 {e}"),
        }
    }
    match super::audio::probe() {
        Ok(s) => println!("声音：{s}"),
        Err(e) => println!("声音：不可用 {e}"),
    }
    0
}
