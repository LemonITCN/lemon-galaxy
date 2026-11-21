//! 画面代理：运行在用户会话里（服务以 SYSTEM 身份放进来，锁屏、UAC 时也能截屏和操作），
//! 从标准输入读服务的请求，把编码好的画面和应答写到标准输出。服务关掉管道时退出。

use super::capture::{attach_input_desktop, draw_cursor, find_monitor, grab, input_desktop_changed, monitors, open_capturer, Capturer, Frame, GdiCapturer};
use super::encode::{jpeg, H264};
use super::input::Injector;
use super::ipc::{read_msg, write_msg, FromAgent, ToAgent};
use galaxy_proto::peer::{ClipboardData, ImageData, InputEvent};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

struct CaptureCtl {
    stop: AtomicBool,
    keyframe: AtomicBool,
    bitrate: AtomicU32,
    reconfig: AtomicBool,
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
    use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
    unsafe {
        if SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).is_err() {
            let _ = windows::Win32::UI::WindowsAndMessaging::SetProcessDPIAware();
        }
    }
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
    let mut captures: HashMap<u32, Arc<CaptureCtl>> = HashMap::new();
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
                let list = monitors().iter().map(|m| m.info()).collect();
                let _ = out_tx.send(FromAgent::Displays { id, list });
            }
            ToAgent::StartCapture { id, display, max_fps, bitrate_kbps } => {
                if let Some(old) = captures.remove(&display) {
                    old.stop.store(true, Ordering::Relaxed);
                }
                let ctl = Arc::new(CaptureCtl { stop: AtomicBool::new(false), keyframe: AtomicBool::new(true), bitrate: AtomicU32::new(bitrate_kbps), reconfig: AtomicBool::new(false) });
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
            ToAgent::Inject { display, ev } => {
                let _ = ui_tx.send(UiJob::Inject(display, ev));
            }
            ToAgent::ClipboardGet { id } => {
                let _ = ui_tx.send(UiJob::ClipboardGet(id));
            }
            ToAgent::ClipboardSet { data } => {
                let _ = ui_tx.send(UiJob::ClipboardSet(data));
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

fn capture_thread(id: u64, display: u32, fps: u32, ctl: Arc<CaptureCtl>, out: SyncSender<FromAgent>) {
    attach_input_desktop();
    let Some(mut mon) = find_monitor(display) else {
        let _ = out.send(FromAgent::Started { id, result: Err("找不到这块屏幕".into()) });
        return;
    };
    let mut cap: Box<dyn Capturer> = open_capturer(&mon, fps);
    log(&out, false, format!("屏幕 {display} 开始采集（{}，{}x{}，{fps} 帧）", cap.kind(), mon.width(), mon.height()));
    let interval = Duration::from_millis(1000 / fps as u64);
    let t0 = Instant::now();
    let mut frame = Frame { width: 0, height: 0, data: vec![] };
    let mut work: Vec<u8> = vec![];
    let mut enc: Option<H264> = None;
    let mut started = false;
    let mut have = false;
    let mut last_sent = Instant::now();
    let mut last_cursor = (i32::MIN, i32::MIN);
    let mut failures = 0u32;
    while !ctl.stop.load(Ordering::Relaxed) {
        let tick = Instant::now();
        if input_desktop_changed() {
            attach_input_desktop();
            cap = open_capturer(&mon, fps);
            ctl.keyframe.store(true, Ordering::Relaxed);
        }
        let changed = match cap.next(&mut frame, interval.as_millis() as u32) {
            Ok(c) => {
                failures = 0;
                if c {
                    have = true;
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
                cap = if failures > 3 { Box::new(GdiCapturer::new(&mon, fps)) } else { open_capturer(&mon, fps) };
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
        // 分辨率变了或改了码率：重建编码器
        let size_changed = enc.as_ref().map(|e| e.width != frame.width & !1 || e.height != frame.height & !1).unwrap_or(true);
        if size_changed || ctl.reconfig.swap(false, Ordering::Relaxed) {
            match H264::new(frame.width, frame.height, fps, ctl.bitrate.load(Ordering::Relaxed)) {
                Ok(e) => {
                    if started && size_changed {
                        let _ = out.send(FromAgent::Resized { display, width: e.width, height: e.height });
                    }
                    enc = Some(e);
                    ctl.keyframe.store(true, Ordering::Relaxed);
                }
                Err(e) => {
                    let _ = out.send(FromAgent::Started { id, result: Err(format!("编码器启动失败：{e}")) });
                    return;
                }
            }
        }
        let e = enc.as_mut().unwrap();
        if !started {
            let _ = out.send(FromAgent::Started { id, result: Ok((e.width, e.height)) });
            started = true;
        }
        let cursor = cursor_pos();
        let need_key = ctl.keyframe.swap(false, Ordering::Relaxed);
        // 画面变了、光标动了、要关键帧、或空闲超过 1 秒（保活）时才编码
        if changed || need_key || cursor != last_cursor || last_sent.elapsed() > Duration::from_secs(1) {
            last_cursor = cursor;
            work.clear();
            work.extend_from_slice(&frame.data);
            let mut wf = Frame { width: frame.width, height: frame.height, data: std::mem::take(&mut work) };
            draw_cursor(&mut wf, &mon);
            if need_key {
                e.force_keyframe();
            }
            match e.encode(&wf.data, wf.width, wf.height) {
                Ok(Some((data, keyframe))) => {
                    last_sent = Instant::now();
                    if out.send(FromAgent::Frame { display, data, pts_us: t0.elapsed().as_micros() as u64, keyframe }).is_err() {
                        return;
                    }
                }
                Ok(None) => {}
                Err(err) => log(&out, true, format!("编码失败：{err}")),
            }
            work = wf.data;
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

/// 命令行自检：每块屏幕抓几帧并编码，打印耗时（CI 与排障用）。
pub fn selftest() -> i32 {
    set_dpi_aware();
    attach_input_desktop();
    let ms = monitors();
    println!("屏幕 {} 块", ms.len());
    if ms.is_empty() {
        println!("没有可用的屏幕（服务会话或没有显示器）");
        return 0;
    }
    let mut ok = true;
    for m in &ms {
        println!("  {} {} {}x{} 主屏={} 缩放={}", m.id, m.device, m.width(), m.height(), m.primary, m.scale);
        let mut cap = open_capturer(m, 30);
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
        println!("    {} 截屏 {} ms", cap.kind(), t.elapsed().as_millis());
        draw_cursor(&mut f, m);
        match H264::new(f.width, f.height, 30, 4000) {
            Ok(mut e) => {
                let t = Instant::now();
                let mut bytes = 0;
                for i in 0..10 {
                    if i == 0 {
                        e.force_keyframe();
                    }
                    if let Ok(Some((d, _))) = e.encode(&f.data, f.width, f.height) {
                        bytes += d.len();
                    }
                }
                println!("    H.264 编码 10 帧 {} ms，共 {} 字节", t.elapsed().as_millis(), bytes);
            }
            Err(e) => {
                println!("    编码器失败：{e}");
                ok = false;
            }
        }
        match jpeg(&f.data, f.width, f.height, 1568, 1568) {
            Ok((d, w, h)) => println!("    截图 JPEG {w}x{h} {} 字节", d.len()),
            Err(e) => println!("    JPEG 失败：{e}"),
        }
    }
    if ok {
        0
    } else {
        1
    }
}
