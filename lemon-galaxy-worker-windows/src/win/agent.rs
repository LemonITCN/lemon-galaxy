//! 画面代理：运行在用户会话里（服务以 SYSTEM 身份放进来，锁屏、UAC 时也能截屏和操作），
//! 从标准输入读服务的请求，把编码好的画面和应答写到标准输出。服务关掉管道时退出。

use super::capture::{attach_input_desktop, draw_cursor, find_monitor, grab, input_desktop_changed, monitors, open_capturer, Capturer, Frame, GdiCapturer, Monitor};
use super::encode::{com_init, hardware_requests, has_parameter_sets, jpeg, nal_types, Request, VideoEncoder};
use super::input::Injector;
use super::ipc::{read_msg, write_msg, FromAgent, ToAgent};
use galaxy_proto::peer::{ClipboardData, ImageData, InputEvent};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{FILETIME, POINT};
use windows::Win32::Graphics::Direct3D11::ID3D11Device;
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorInfo, GetCursorPos, CURSORINFO, CURSOR_SHOWING};

struct CaptureCtl {
    stop: AtomicBool,
    keyframe: AtomicBool,
    bitrate: AtomicU32,
    reconfig: AtomicBool,
    /// 画面里画光标；关掉时只上报光标位置
    cursor_in_video: AtomicBool,
}

enum VdJob {
    Add { id: u64, width: u32, height: u32, refresh: u32 },
    Remove { id: u64, display: u32 },
    State { id: u64 },
}

enum UiJob {
    Inject(u32, InputEvent),
    ClipboardGet(u64),
    ClipboardSet(ClipboardData),
}

fn log(out: &SyncSender<FromAgent>, error: bool, text: String) {
    let _ = out.try_send(FromAgent::Log { error, text });
}

fn set_dpi_aware() {
    super::compat::set_dpi_aware();
}

pub fn main() -> i32 {
    set_dpi_aware();
    attach_input_desktop();
    let (out_tx, out_rx) = mpsc::sync_channel::<FromAgent>(16);
    // 写线程：所有输出经这里串行写到标准输出
    let writer = std::thread::spawn(move || {
        let stdout = std::io::stdout();
        let mut w = std::io::BufWriter::with_capacity(512 * 1024, stdout.lock());
        while let Ok(m) = out_rx.recv() {
            if write_msg(&mut w, &m).is_err() {
                break;
            }
        }
    });
    // 键鼠与剪贴板放在一个线程里，跟着输入桌面走
    let (ui_tx, ui_rx) = mpsc::channel::<UiJob>();
    {
        let out = out_tx.clone();
        std::thread::spawn(move || {
            attach_input_desktop();
            let mut inj = Injector::new();
            while let Ok(j) = ui_rx.recv() {
                if input_desktop_changed() {
                    attach_input_desktop();
                }
                match j {
                    UiJob::Inject(d, ev) => inj.inject(d, ev),
                    UiJob::ClipboardGet(id) => {
                        let _ = out.send(FromAgent::Clipboard { id, data: super::clipboard::get() });
                    }
                    UiJob::ClipboardSet(d) => super::clipboard::set(d),
                }
            }
        });
    }
    // 虚拟屏放在自己的线程里（加删一次要一秒左右，不挡住键鼠和其它请求）；驱动句柄归这个线程，代理退出时删屏
    let vd_names: Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let (vd_tx, vd_rx) = mpsc::channel::<VdJob>();
    let vd_thread = {
        let out = out_tx.clone();
        let names = vd_names.clone();
        std::thread::spawn(move || {
            attach_input_desktop();
            let logf = |text: String| {
                let _ = out.send(FromAgent::Log { error: false, text });
            };
            let mut vd = super::vdisplay::VirtualDisplays::default();
            while let Ok(j) = vd_rx.recv() {
                if input_desktop_changed() {
                    attach_input_desktop();
                }
                match j {
                    VdJob::Add { id, width, height, refresh } => {
                        let result = vd.add(width, height, refresh, &logf);
                        *names.lock().unwrap() = vd.names();
                        let _ = out.send(FromAgent::VirtualAdded { id, result });
                    }
                    VdJob::Remove { id, display } => {
                        let result = vd.remove(display, &logf);
                        *names.lock().unwrap() = vd.names();
                        let _ = out.send(FromAgent::VirtualRemoved { id, result });
                    }
                    VdJob::State { id } => {
                        let _ = out.send(FromAgent::VirtualState { id, ids: vd.ids(), needs_headless: vd.needs_headless() });
                    }
                }
            }
            vd.clear(&logf);
        })
    };
    let mut captures: HashMap<u32, Arc<CaptureCtl>> = HashMap::new();
    let mut privacy: Option<super::privacy::Privacy> = None;
    let mut audio_stop: Option<Arc<AtomicBool>> = None;
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
            ToAgent::Privacy { id, on } => {
                let result = if on {
                    if privacy.is_some() {
                        Ok(())
                    } else {
                        super::privacy::Privacy::start().map(|p| privacy = Some(p))
                    }
                } else {
                    privacy = None;
                    Ok(())
                };
                let _ = out_tx.send(FromAgent::PrivacyResult { id, result });
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
            ToAgent::ListDisplays { id } => {
                let virt = vd_names.lock().unwrap().clone();
                let list = monitors()
                    .iter()
                    .map(|m| {
                        let mut i = m.info();
                        i.is_virtual = virt.iter().any(|n| n.eq_ignore_ascii_case(&m.device));
                        i
                    })
                    .collect();
                let _ = out_tx.send(FromAgent::Displays { id, list });
            }
            ToAgent::VirtualAdd { id, width, height, refresh } => {
                let _ = vd_tx.send(VdJob::Add { id, width, height, refresh });
            }
            ToAgent::VirtualRemove { id, display } => {
                let _ = vd_tx.send(VdJob::Remove { id, display });
            }
            ToAgent::VirtualState { id } => {
                let _ = vd_tx.send(VdJob::State { id });
            }
            ToAgent::StartCapture { id, display, max_fps, bitrate_kbps } => {
                if let Some(old) = captures.remove(&display) {
                    old.stop.store(true, Ordering::Relaxed);
                }
                let ctl = Arc::new(CaptureCtl { stop: AtomicBool::new(false), keyframe: AtomicBool::new(true), bitrate: AtomicU32::new(bitrate_kbps), reconfig: AtomicBool::new(false), cursor_in_video: AtomicBool::new(true) });
                captures.insert(display, ctl.clone());
                let out = out_tx.clone();
                std::thread::spawn(move || capture_thread(id, display, max_fps.clamp(1, 60), ctl, out));
            }
            ToAgent::StopCapture { display } => {
                if let Some(c) = captures.remove(&display) {
                    c.stop.store(true, Ordering::Relaxed);
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
            ToAgent::ComputerUse { id, request } => {
                let out = out_tx.clone();
                std::thread::spawn(move || {
                    let (text, image) = galaxy_host::win_cu::handle(&request).unwrap_or_else(|| ("错误：这台电脑不支持应用操作".into(), None));
                    let _ = out.send(FromAgent::ComputerUse { id, text, image });
                });
            }
            ToAgent::Screenshot { id, display, max_w, max_h } => {
                let out = out_tx.clone();
                std::thread::spawn(move || {
                    attach_input_desktop();
                    let image = find_monitor(display).and_then(|m| grab(&m).ok()).and_then(|f| jpeg(&f.data, f.width, f.height, max_w, max_h).ok()).map(|(data, width, height)| ImageData { mime: "image/jpeg".into(), data, width, height });
                    let _ = out.send(FromAgent::Screenshot { id, image });
                });
            }
        }
    }
    for c in captures.values() {
        c.stop.store(true, Ordering::Relaxed);
    }
    // 删掉虚拟屏、恢复显示配置后再退出（句柄关闭时驱动也会删屏，这里多做一步恢复配置）
    drop(vd_tx);
    let _ = vd_thread.join();
    drop(out_tx);
    let _ = writer.join();
    0
}

fn cursor_pos() -> (i32, i32) {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

fn cursor_showing() -> bool {
    let mut ci = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
    unsafe { GetCursorInfo(&mut ci).is_ok() && ci.flags.0 & CURSOR_SHOWING.0 != 0 }
}

/// 重开截屏：先放掉旧的桌面复制再建新的。`prefer` 是编码器的设备（同一块显卡就共用，零拷贝）。
fn reopen(cap: &mut Box<dyn Capturer>, mon: &Monitor, fps: u32, prefer: Option<&ID3D11Device>) {
    *cap = Box::new(GdiCapturer::new(mon, fps));
    *cap = open_capturer(mon, fps, prefer);
}

fn capture_thread(id: u64, display: u32, fps: u32, ctl: Arc<CaptureCtl>, out: SyncSender<FromAgent>) {
    attach_input_desktop();
    com_init();
    let Some(mut mon) = find_monitor(display) else {
        let _ = out.send(FromAgent::Started { id, result: Err("找不到这块屏幕".into()) });
        return;
    };
    let request = Request::from_env();
    let mut cap: Box<dyn Capturer> = open_capturer(&mon, fps, None);
    log(&out, false, format!("屏幕 {display} 开始采集（{}，{}x{}，{fps} 帧）", cap.kind(), mon.width(), mon.height()));
    let interval = Duration::from_millis(1000 / fps as u64);
    let t0 = Instant::now();
    let mut frame = Frame { width: 0, height: 0, data: vec![] };
    // `frame` 里是不是当前画面（零拷贝时画面不经内存，它会过时）
    let mut cpu_valid = false;
    let mut enc: Option<VideoEncoder> = None;
    let mut started = false;
    // 编码器的输入里已经有画面
    let mut have = false;
    let mut last_sent = Instant::now();
    let mut last_cursor = (i32::MIN, i32::MIN);
    let mut last_report: Option<(i32, i32, bool)> = None;
    let mut last_in_video = true;
    let mut failures = 0u32;
    // 硬件编码连续出错后，这次采集改用软件编码
    let mut hw_allowed = true;
    let mut enc_errors = 0u32;
    // 桌面复制连续失败退到 GDI 的时刻：过一会儿再试桌面复制（虚拟屏增删、切分辨率后通常就好了）
    let mut gdi_since: Option<Instant> = None;
    while !ctl.stop.load(Ordering::Relaxed) {
        let tick = Instant::now();
        if input_desktop_changed() {
            attach_input_desktop();
            reopen(&mut cap, &mon, fps, enc.as_ref().and_then(|e| e.device()));
            ctl.keyframe.store(true, Ordering::Relaxed);
            gdi_since = None;
        }
        if gdi_since.map(|t| t.elapsed() > Duration::from_secs(5)).unwrap_or(false) {
            reopen(&mut cap, &mon, fps, enc.as_ref().and_then(|e| e.device()));
            gdi_since = (cap.kind() == "gdi").then(Instant::now);
            if gdi_since.is_none() {
                log(&out, false, format!("屏幕 {display} 恢复桌面复制截屏"));
            }
        }
        // 编码器：没有或截屏尺寸变了就重建；改码率时硬件编码器直接改，软件编码器重建
        let size = cap.size();
        let mut rebuild = enc.as_ref().map(|e| e.source_size() != size).unwrap_or(true);
        if ctl.reconfig.swap(false, Ordering::Relaxed) && !rebuild {
            if let Some(e) = enc.as_mut() {
                rebuild = !e.set_bitrate(ctl.bitrate.load(Ordering::Relaxed));
            }
        }
        if rebuild {
            let kbps = ctl.bitrate.load(Ordering::Relaxed);
            let req = if hw_allowed { request.clone() } else { Request::software() };
            match VideoEncoder::new(size.0, size.1, fps, kbps, cap.device(), &req) {
                Ok((e, notes)) => {
                    let old = enc.take();
                    // 只在第一次、换了编码器或尺寸时记日志（软件编码改码率会频繁重建）
                    if old.as_ref().map(|o| o.name() != e.name() || (o.width, o.height) != (e.width, e.height)).unwrap_or(true) {
                        let why = if notes.is_empty() { String::new() } else { format!("；没用上的：{}", notes.join("；")) };
                        log(&out, false, format!("屏幕 {display} 编码器：{}（{}x{}，{kbps} kbps{why}）", e.name(), e.width, e.height));
                    }
                    if started && old.as_ref().map(|o| (o.width, o.height) != (e.width, e.height)).unwrap_or(false) {
                        let _ = out.send(FromAgent::Resized { display, width: e.width, height: e.height });
                    }
                    drop(old);
                    // 新编码器的输入是空的：内存里有当前画面就传上去，否则让截屏重新出一整帧
                    let mut e = e;
                    if e.gpu_target(cap.device()).is_none() && cpu_valid && (frame.width, frame.height) == size {
                        e.upload(&frame);
                    } else {
                        reopen(&mut cap, &mon, fps, e.device());
                        have = false;
                    }
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
            continue;
        }
        let target = enc.as_ref().and_then(|e| e.gpu_target(cap.device())).cloned();
        let res = match &target {
            Some(t) => cap.next_gpu(t, interval.as_millis() as u32),
            None => cap.next(&mut frame, interval.as_millis() as u32),
        };
        let changed = match res {
            Ok(c) => {
                failures = 0;
                if c {
                    have = true;
                    cpu_valid = target.is_none();
                    if cpu_valid {
                        if let Some(e) = enc.as_mut() {
                            e.upload(&frame);
                        }
                    }
                }
                c
            }
            Err(e) => {
                failures += 1;
                let lost = e.to_string() == "access_lost";
                if !lost {
                    log(&out, true, format!("截屏失败（{}）：{e}", cap.kind()));
                }
                std::thread::sleep(Duration::from_millis(if lost { 50 } else { 300 }));
                attach_input_desktop();
                if let Some(m) = find_monitor(display) {
                    mon = m;
                }
                if failures > 3 {
                    cap = Box::new(GdiCapturer::new(&mon, fps));
                    gdi_since.get_or_insert_with(Instant::now);
                } else {
                    reopen(&mut cap, &mon, fps, enc.as_ref().and_then(|e| e.device()));
                }
                ctl.keyframe.store(true, Ordering::Relaxed);
                continue;
            }
        };
        if !have {
            if !started && t0.elapsed() > Duration::from_secs(5) {
                let _ = out.send(FromAgent::Started { id, result: Err("拿不到屏幕画面".into()) });
                return;
            }
            continue;
        }
        let Some(e) = enc.as_mut() else { continue };
        if !started {
            let _ = out.send(FromAgent::Started { id, result: Ok((e.width, e.height)) });
            started = true;
        }
        let cursor = cursor_pos();
        let need_key = ctl.keyframe.swap(false, Ordering::Relaxed);
        let in_video = ctl.cursor_in_video.load(Ordering::Relaxed);
        // 主控端自己画光标：光标变了就报位置（换算到画面像素），画面里不画
        if !in_video {
            let (w, h) = (mon.width().max(1) as i64, mon.height().max(1) as i64);
            let (rx, ry) = ((cursor.0 - mon.rect.left) as i64, (cursor.1 - mon.rect.top) as i64);
            let visible = cursor_showing() && rx >= 0 && ry >= 0 && rx < w && ry < h;
            let report = ((rx * e.width as i64 / w) as i32, (ry * e.height as i64 / h) as i32, visible);
            if Some(report) != last_report {
                last_report = Some(report);
                let _ = out.try_send(FromAgent::Cursor { display, x: report.0, y: report.1, visible: report.2 });
            }
        } else {
            last_report = None;
        }
        let toggled = in_video != last_in_video;
        last_in_video = in_video;
        // 画面变了、画面里的光标动了、要关键帧、或空闲超过 1 秒（保活）时才编码
        if changed || need_key || toggled || (in_video && cursor != last_cursor) || last_sent.elapsed() > Duration::from_secs(1) {
            last_cursor = cursor;
            match e.encode(&frame, &mon, in_video, need_key, t0.elapsed().as_micros() as u64) {
                Ok(frames) => {
                    enc_errors = 0;
                    for f in frames {
                        last_sent = Instant::now();
                        if out.send(FromAgent::Frame { display, data: f.data, pts_us: t0.elapsed().as_micros() as u64, keyframe: f.keyframe }).is_err() {
                            return;
                        }
                    }
                }
                Err(err) => {
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
        if spent < interval && !changed {
            // DXGI 没有新帧时已经等过了；GDI 自己按节奏睡
        } else if spent < interval {
            std::thread::sleep(interval - spent);
        }
    }
    log(&out, false, format!("屏幕 {display} 停止采集"));
}

fn cpu_time() -> Duration {
    let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe {
        let _ = GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u);
    }
    let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    Duration::from_nanos((t(k) + t(u)) * 100)
}

fn stats(v: &[f64]) -> String {
    if v.is_empty() {
        return "无".into();
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let avg = s.iter().sum::<f64>() / s.len() as f64;
    format!("平均 {avg:.2} ms，中位 {:.2}，最大 {:.2}", s[s.len() / 2], s[s.len() - 1])
}

fn avg_size(v: &[(usize, bool)]) -> usize {
    let p: Vec<usize> = v.iter().filter(|x| !x.1).map(|x| x.0).collect();
    if p.is_empty() {
        0
    } else {
        p.iter().sum::<usize>() / p.len()
    }
}

/// 自检里的单个编码器测试：真实截屏 2 秒（量编码耗时与 CPU 占用）、合成滚动画面（码率运行时修改、
/// 按需关键帧）、换分辨率。`dump` 给出时把整段码流存成文件（可以拿 ffmpeg 解一遍核对）。
fn bench(label: &str, req: &Request, mon: &Monitor, base: &Frame, cap: &mut Box<dyn Capturer>, dump: Option<&std::path::Path>) -> bool {
    let (w, h) = (base.width, base.height);
    let (mut e, notes) = match VideoEncoder::new(w, h, 30, 4000, cap.device(), req) {
        Ok(x) => x,
        Err(err) => {
            println!("    [{label}] 不可用：{err}");
            return true;
        }
    };
    println!("    [{label}] {}", e.name());
    for n in notes {
        println!("      （跳过：{n}）");
    }
    let mut stream: Vec<u8> = Vec::new();
    let mut ok = true;
    let mut check = |frames: &[super::encode::EncodedFrame], stream: &mut Vec<u8>, sizes: &mut Vec<(usize, bool)>| {
        for f in frames {
            let t = nal_types(&f.data);
            if t.contains(&9) {
                println!("      错误：输出里有 AUD");
                ok = false;
            }
            if f.keyframe && !has_parameter_sets(&f.data) {
                println!("      错误：关键帧没有 SPS/PPS（{t:?}）");
                ok = false;
            }
            sizes.push((f.data.len(), f.keyframe));
            stream.extend_from_slice(&f.data);
        }
    };

    // 1. 真实截屏 60 帧，每帧都编（30 帧/秒节奏）
    e.upload(base);
    let mut cur = Frame { width: w, height: h, data: base.data.clone() };
    let target = e.gpu_target(cap.device()).cloned();
    let mut times = Vec::new();
    let mut sizes = Vec::new();
    let (c0, w0) = (cpu_time(), Instant::now());
    for i in 0..60u32 {
        let tick = Instant::now();
        match &target {
            Some(t) => {
                let _ = cap.next_gpu(t, 1);
            }
            None => {
                if let Ok(true) = cap.next(&mut cur, 1) {
                    e.upload(&cur);
                }
            }
        }
        let t = Instant::now();
        match e.encode(&cur, mon, true, i == 0, w0.elapsed().as_micros() as u64) {
            Ok(fs) => {
                times.push(t.elapsed().as_secs_f64() * 1000.0);
                check(&fs, &mut stream, &mut sizes);
            }
            Err(err) => {
                println!("      编码失败：{err}");
                return false;
            }
        }
        if let Some(r) = Duration::from_millis(33).checked_sub(tick.elapsed()) {
            std::thread::sleep(r);
        }
    }
    let (cpu, wall) = (cpu_time() - c0, w0.elapsed());
    let key = sizes.iter().find(|x| x.1).map(|x| x.0).unwrap_or(0);
    println!(
        "      真实画面 {w}x{h} 60 帧（{}）：编码 {}；进程 CPU {:.0}%（单核为 100%）；关键帧 {} 字节，其余平均 {} 字节，出帧 {}",
        if target.is_some() { "显卡直通" } else { "经内存" },
        stats(&times),
        cpu.as_secs_f64() / wall.as_secs_f64() * 100.0,
        key,
        avg_size(&sizes),
        sizes.len()
    );

    // 2. 合成滚动画面 90 帧：4000 kbps → 第 30 帧改 1000 → 第 60 帧改 8000；第 75 帧要关键帧
    let mut syn = Frame { width: w, height: h, data: vec![0; base.data.len()] };
    let row = w as usize * 4;
    let mut seg: [Vec<(usize, bool)>; 3] = Default::default();
    let mut times = Vec::new();
    let mut keys = Vec::new();
    let mut runtime = Vec::new();
    for i in 0..90u32 {
        if i == 30 || i == 60 {
            let kbps = if i == 30 { 1000 } else { 8000 };
            let t = Instant::now();
            if e.set_bitrate(kbps) {
                runtime.push(format!("第 {i} 帧改 {kbps} kbps：运行时生效（{:.2} ms）", t.elapsed().as_secs_f64() * 1000.0));
            } else {
                runtime.push(format!("第 {i} 帧改 {kbps} kbps：要重建编码器"));
                match VideoEncoder::new(w, h, 30, kbps, cap.device(), req) {
                    Ok((n, _)) => e = n,
                    Err(err) => {
                        println!("      重建失败：{err}");
                        return false;
                    }
                }
            }
        }
        let off = (i as usize * 8) % h as usize;
        for y in 0..h as usize {
            let sy = (y + off) % h as usize;
            syn.data[y * row..(y + 1) * row].copy_from_slice(&base.data[sy * row..(sy + 1) * row]);
        }
        // 再加一块每帧都变的噪点（压不动，码率高低能从帧大小上看出来）
        let (bw, bh) = ((w as usize / 4).min(480), (h as usize / 4).min(320));
        let mut seed = 0x9e37_79b9u32 ^ i.wrapping_mul(0x85eb_ca6b);
        for y in 0..bh {
            for x in 0..bw {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let p = (y + 64) * row + (x + 64) * 4;
                syn.data[p..p + 4].copy_from_slice(&[seed as u8, (seed >> 8) as u8, (seed >> 16) as u8, 255]);
            }
        }
        let t = Instant::now();
        e.upload(&syn);
        match e.encode(&syn, mon, true, i == 0 || i == 75, (i as u64 + 1000) * 33_333) {
            Ok(fs) => {
                times.push(t.elapsed().as_secs_f64() * 1000.0);
                let s = &mut seg[(i / 30) as usize];
                for f in &fs {
                    if f.keyframe {
                        keys.push(i);
                    }
                }
                check(&fs, &mut stream, s);
            }
            Err(err) => {
                println!("      编码失败：{err}");
                return false;
            }
        }
    }
    println!("      滚动画面 90 帧：上传+转换+编码 {}", stats(&times));
    println!("      {}", runtime.join("；"));
    println!("      非关键帧平均大小：4000 kbps {} 字节，1000 kbps {} 字节，8000 kbps {} 字节；关键帧出现在第 {keys:?} 帧（应为 [0, 75]）", avg_size(&seg[0]), avg_size(&seg[1]), avg_size(&seg[2]));

    // 3. 换分辨率（减半）：重建编码器，码流里接着出新的 SPS
    let (hw, hh) = ((w / 2).max(16), (h / 2).max(16));
    let mut half = Frame { width: hw, height: hh, data: vec![0; (hw * hh * 4) as usize] };
    for y in 0..hh as usize {
        half.data[y * hw as usize * 4..(y + 1) * hw as usize * 4].copy_from_slice(&base.data[y * row..y * row + hw as usize * 4]);
    }
    match VideoEncoder::new(hw, hh, 30, 4000, None, req) {
        Ok((mut e2, _)) => {
            let mut sz = Vec::new();
            for i in 0..10u32 {
                e2.upload(&half);
                match e2.encode(&half, mon, true, i == 0, i as u64 * 33_333) {
                    Ok(fs) => check(&fs, &mut stream, &mut sz),
                    Err(err) => {
                        println!("      {hw}x{hh} 编码失败：{err}");
                        ok = false;
                        break;
                    }
                }
            }
            println!("      换分辨率 {}x{}：{}，10 帧共 {} 字节，首帧关键帧 {}", e2.width, e2.height, e2.name(), sz.iter().map(|x| x.0).sum::<usize>(), sz.first().map(|x| x.1).unwrap_or(false));
        }
        Err(err) => {
            println!("      换分辨率 {hw}x{hh} 失败：{err}");
            ok = false;
        }
    }
    if let Some(dir) = dump {
        let name: String = label.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
        let p = dir.join(format!("{}.h264", name.trim_matches('-')));
        match std::fs::write(&p, &stream) {
            Ok(()) => println!("      码流已存 {}（{} 字节）", p.display(), stream.len()),
            Err(err) => println!("      码流存不下来：{err}"),
        }
    }
    ok
}

/// 自检：按画面代理的真实流程在主屏上采集 `secs` 秒（中途降码率、要一次关键帧），统计输出。
fn agent_run(secs: u64, dump: Option<&std::path::Path>) {
    let Some(mon) = find_monitor(0) else { return };
    let (tx, rx) = mpsc::sync_channel::<FromAgent>(64);
    let ctl = Arc::new(CaptureCtl { stop: AtomicBool::new(false), keyframe: AtomicBool::new(true), bitrate: AtomicU32::new(6000), reconfig: AtomicBool::new(false), cursor_in_video: AtomicBool::new(true) });
    let c2 = ctl.clone();
    let th = std::thread::spawn(move || capture_thread(1, mon.id, 30, c2, tx));
    let t0 = Instant::now();
    let (c0, mut stream, mut frames, mut keys, mut bytes) = (cpu_time(), Vec::new(), 0usize, Vec::new(), [0usize; 2]);
    let (mut changed_rate, mut asked_key) = (false, false);
    while t0.elapsed() < Duration::from_secs(secs) {
        let el = t0.elapsed();
        if !changed_rate && el > Duration::from_secs(secs / 2) {
            ctl.bitrate.store(1500, Ordering::Relaxed);
            ctl.reconfig.store(true, Ordering::Relaxed);
            changed_rate = true;
            println!("    {:.1}s 改码率 1500 kbps", el.as_secs_f64());
        }
        if !asked_key && el > Duration::from_secs(secs * 3 / 4) {
            ctl.keyframe.store(true, Ordering::Relaxed);
            asked_key = true;
            println!("    {:.1}s 请求关键帧", el.as_secs_f64());
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(FromAgent::Frame { data, keyframe, .. }) => {
                frames += 1;
                bytes[changed_rate as usize] += data.len();
                if keyframe {
                    keys.push(format!("{:.2}s", el.as_secs_f64()));
                }
                stream.extend_from_slice(&data);
            }
            Ok(FromAgent::Log { error, text }) => println!("    {}{text}", if error { "错误：" } else { "" }),
            Ok(FromAgent::Started { result, .. }) => println!("    开始：{result:?}"),
            Ok(FromAgent::Resized { width, height, .. }) => println!("    分辨率变为 {width}x{height}"),
            Ok(_) => {}
            Err(_) => {}
        }
    }
    ctl.stop.store(true, Ordering::Relaxed);
    while rx.recv_timeout(Duration::from_millis(300)).is_ok() {}
    let _ = th.join();
    let cpu = (cpu_time() - c0).as_secs_f64() / t0.elapsed().as_secs_f64() * 100.0;
    let half = secs as f64 / 2.0;
    println!(
        "    {secs} 秒出 {frames} 帧，关键帧在 {keys:?}；前半 6000 kbps 时 {:.0} kbps，后半 1500 kbps 时 {:.0} kbps；进程 CPU {cpu:.0}%（单核为 100%）",
        bytes[0] as f64 * 8.0 / 1000.0 / half,
        bytes[1] as f64 * 8.0 / 1000.0 / half
    );
    if let Some(dir) = dump {
        let p = dir.join("agent.h264");
        if std::fs::write(&p, &stream).is_ok() {
            println!("    码流已存 {}", p.display());
        }
    }
}

/// 命令行自检：每块屏幕抓几帧并编码，打印用的编码器与耗时（CI 与排障用）。
/// 参数 `--bench`：在主屏上把本机能用的每种编码器都测一遍；`--dump 目录`：同时把码流存下来；
/// `--agent 秒数`：按画面代理的真实流程采集编码一段时间。
pub fn selftest(args: &[String]) -> i32 {
    set_dpi_aware();
    attach_input_desktop();
    com_init();
    let dump = args.iter().position(|a| a == "--dump").and_then(|i| args.get(i + 1)).map(std::path::PathBuf::from);
    let bench_all = dump.is_some() || args.iter().any(|a| a == "--bench");
    if let Some(d) = &dump {
        let _ = std::fs::create_dir_all(d);
    }
    if let Some(secs) = args.iter().position(|a| a == "--agent").and_then(|i| args.get(i + 1)).and_then(|s| s.parse::<u64>().ok()) {
        println!("画面代理流程（主屏，{secs} 秒）：");
        agent_run(secs.clamp(2, 120), dump.as_deref());
        return 0;
    }
    let ms = monitors();
    println!("屏幕 {} 块", ms.len());
    if ms.is_empty() {
        println!("没有可用的屏幕（服务会话或没有显示器）");
        return 0;
    }
    let gpus = super::encode::gpu::adapters();
    println!("显卡 {} 块：{}", gpus.len(), gpus.iter().map(|a| format!("{}（{:04X}）", a.name, a.vendor)).collect::<Vec<_>>().join("，"));
    let mut ok = true;
    for m in &ms {
        println!("  {} {} {}x{} 主屏={} 缩放={}", m.id, m.device, m.width(), m.height(), m.primary, m.scale);
        let mut cap = open_capturer(m, 30, None);
        let mut f = Frame { width: 0, height: 0, data: vec![] };
        let t = Instant::now();
        let mut got = false;
        for _ in 0..20 {
            match cap.next(&mut f, 200) {
                Ok(true) => {
                    got = true;
                    break;
                }
                Ok(false) => {}
                Err(e) => {
                    println!("    {} 截屏失败：{e}", cap.kind());
                    cap = Box::new(GdiCapturer::new(m, 30));
                }
            }
        }
        if !got {
            println!("    没有拿到画面");
            ok = false;
            continue;
        }
        let lit = f.data.chunks(4).filter(|px| px[0] > 16 || px[1] > 16 || px[2] > 16).count();
        println!("    画面非黑像素 {:.1}%", lit as f64 * 400.0 / f.data.len().max(1) as f64);
        let gpu_name = cap.device().and_then(super::encode::gpu::device_adapter).map(|a| a.name).unwrap_or_default();
        println!("    {} 截屏 {} ms{}", cap.kind(), t.elapsed().as_millis(), if gpu_name.is_empty() { String::new() } else { format!("（{gpu_name}）") });
        // 实际会用的编码器（与画面代理同样的选择）
        match VideoEncoder::new(f.width, f.height, 30, 4000, cap.device(), &Request::from_env()) {
            Ok((mut e, notes)) => {
                println!("    编码器：{}", e.name());
                for n in notes {
                    println!("      （跳过：{n}）");
                }
                e.upload(&f);
                let t = Instant::now();
                let mut bytes = 0;
                let mut fail = None;
                for i in 0..10 {
                    match e.encode(&f, m, true, i == 0, i as u64 * 33_333) {
                        Ok(fs) => bytes += fs.iter().map(|x| x.data.len()).sum::<usize>(),
                        Err(err) => fail = Some(err),
                    }
                }
                match fail {
                    Some(err) => {
                        println!("    编码失败：{err}");
                        ok = false;
                    }
                    None => println!("    H.264 编码 10 帧 {} ms，共 {} 字节", t.elapsed().as_millis(), bytes),
                }
            }
            Err(e) => {
                println!("    编码器失败：{e}");
                ok = false;
            }
        }
        // 同一块屏幕一个进程只能有一个桌面复制：测各编码器前先放掉这个
        drop(cap);
        if bench_all && (m.primary || ms.len() == 1) {
            let mut list = hardware_requests();
            list.push(("openh264".into(), Request::software()));
            for (label, req) in list {
                // 每种编码器都用新开的截屏，互不影响
                let mut c = open_capturer(m, 30, None);
                if !bench(&label, &req, m, &f, &mut c, dump.as_deref()) {
                    ok = false;
                }
            }
        }
        draw_cursor(&mut f, m);
        match jpeg(&f.data, f.width, f.height, 1568, 1568) {
            Ok((d, w, h)) => println!("    截图 JPEG {w}x{h} {} 字节", d.len()),
            Err(e) => println!("    JPEG 失败：{e}"),
        }
    }
    // 隐私屏：开着时截到的画面不应是全黑（黑窗被排除在截屏之外）
    if let Some(m) = ms.iter().find(|m| m.primary).or(ms.first()) {
        match super::privacy::Privacy::start() {
            Ok(p) => {
                std::thread::sleep(std::time::Duration::from_millis(500));
                let mut cap = open_capturer(m, 30, None);
                let mut f = Frame { width: 0, height: 0, data: vec![] };
                let mut got = false;
                for _ in 0..20 {
                    if let Ok(true) = cap.next(&mut f, 200) {
                        got = true;
                        break;
                    }
                }
                let lit = f.data.chunks(4).filter(|px| px[0] > 16 || px[1] > 16 || px[2] > 16).count();
                let ratio = if f.data.is_empty() { 0.0 } else { lit as f64 / (f.data.len() / 4) as f64 };
                println!("  隐私屏开启后截屏（{}）：拿到画面 {got}，非黑像素占比 {:.1}%", cap.kind(), ratio * 100.0);
                drop(p);
            }
            Err(e) => println!("  隐私屏：{e}"),
        }
    }
    // 声音：只报告，不算失败（服务器、虚拟机常常没有声音设备）
    let stop = Arc::new(AtomicBool::new(false));
    let got = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let g2 = got.clone();
    match super::audio::start(stop.clone(), move |pcm, _, _| {
        g2.fetch_add(pcm.len() as u64, Ordering::Relaxed);
    }) {
        Ok((rate, ch)) => {
            std::thread::sleep(Duration::from_millis(500));
            stop.store(true, Ordering::Relaxed);
            println!("  声音采集：{rate} Hz × {ch} 声道，0.5 秒内收到 {} 个样本（没有在播放时为 0）", got.load(Ordering::Relaxed));
        }
        Err(e) => println!("  声音采集：{e}"),
    }
    if ok {
        0
    } else {
        1
    }
}
