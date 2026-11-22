//! AI 专属光标：透明、不接收鼠标、不抢焦点的置顶分层窗口，画一个与系统光标明显不同的箭头
//! （品牌蓝描边 + 白色内填充 + 「AI」小标签）。动作前沿平滑曲线移到动作位置，点击时有一圈扩散；
//! 跟着目标窗口走，窗口关闭 / 最小化时隐藏；`end` 或 30 秒没有动作后淡出。
//!
//! 自己一个线程跑消息循环；别的线程把命令放进队列再给窗口发 WM_APP 唤醒。

use super::{dpi_thread_per_monitor, frame_rect, wide, window_scale};
use std::cell::RefCell;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

/// 品牌强调色（默认「银河」主题的 #2F6BEE）
const ACCENT: (f32, f32, f32) = (0x2F as f32 / 255.0, 0x6B as f32 / 255.0, 0xEE as f32 / 255.0);
const MOVE_DUR: Duration = Duration::from_millis(300);
const PULSE_DUR: Duration = Duration::from_millis(420);
const FADE_DUR: Duration = Duration::from_millis(300);
const IDLE: Duration = Duration::from_secs(30);
/// 画布边长与箭头尖在画布里的位置（1 倍缩放下的单位）
const CANVAS: f32 = 52.0;
const TIP: f32 = 20.0;
const TIMER_ID: usize = 1;

enum Cmd {
    Move { target: isize, x: i32, y: i32 },
    Pulse,
    Touch,
    Hide,
}

static QUEUE: Mutex<Vec<Cmd>> = Mutex::new(Vec::new());
static WND: AtomicIsize = AtomicIsize::new(0);
static START: Once = Once::new();

fn push(c: Cmd) {
    QUEUE.lock().unwrap_or_else(|e| e.into_inner()).push(c);
    START.call_once(|| {
        let _ = std::thread::Builder::new().name("win-cu-cursor".into()).spawn(thread_main);
    });
    let h = WND.load(Ordering::Acquire);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as *mut _)), WM_APP, WPARAM(0), LPARAM(0));
        }
    }
}

/// 移到目标窗口上的屏幕坐标（物理像素）
pub(super) fn move_to(target: HWND, x: i32, y: i32) {
    push(Cmd::Move { target: target.0 as isize, x, y });
}

/// 点击动画
pub(super) fn pulse() {
    push(Cmd::Pulse);
}

/// 有动作发生（重置 30 秒空闲计时）
pub(super) fn touch() {
    if WND.load(Ordering::Acquire) != 0 {
        push(Cmd::Touch);
    }
}

/// 淡出收起
pub(super) fn hide() {
    if WND.load(Ordering::Acquire) != 0 {
        push(Cmd::Hide);
    }
}

struct Ov {
    hwnd: HWND,
    target: HWND,
    visible: bool,
    /// 位置相对目标窗口左上角（窗口移动时跟着走）
    from: (f64, f64),
    to: (f64, f64),
    anim: Instant,
    pulse: Option<Instant>,
    fade: Option<Instant>,
    last: Instant,
    /// 上次画的状态，没变就不重画
    drawn: Option<(i32, i32, i32, i32, u8)>,
    surf: Option<Surface>,
    topmost_at: Instant,
}

struct Surface {
    size: i32,
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bmp.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

thread_local! {
    static OV: RefCell<Option<Ov>> = const { RefCell::new(None) };
}

fn thread_main() {
    dpi_thread_per_monitor();
    unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let cls = wide("LemonGalaxyAiCursor");
        let wc = WNDCLASSEXW { cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32, lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: PCWSTR(cls.as_ptr()), ..Default::default() };
        RegisterClassExW(&wc);
        let title = wide("LemonGalaxy AI");
        let ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST;
        let Ok(h) = CreateWindowExW(ex, PCWSTR(cls.as_ptr()), PCWSTR(title.as_ptr()), WS_POPUP, 0, 0, 1, 1, None, None, Some(inst.into()), None) else {
            return;
        };
        let now = Instant::now();
        OV.with(|o| {
            *o.borrow_mut() = Some(Ov { hwnd: h, target: HWND::default(), visible: false, from: (0.0, 0.0), to: (0.0, 0.0), anim: now, pulse: None, fade: None, last: now, drawn: None, surf: None, topmost_at: now });
        });
        WND.store(h.0 as isize, Ordering::Release);
        // 窗口建好之前排进来的命令
        let _ = PostMessageW(Some(h), WM_APP, WPARAM(0), LPARAM(0));
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn wndproc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_APP => {
            let cmds: Vec<Cmd> = std::mem::take(&mut *QUEUE.lock().unwrap_or_else(|e| e.into_inner()));
            OV.with(|o| {
                if let Some(ov) = o.borrow_mut().as_mut() {
                    for c in cmds {
                        ov.apply(c);
                    }
                    ov.tick();
                }
            });
            LRESULT(0)
        }
        WM_TIMER => {
            OV.with(|o| {
                if let Some(ov) = o.borrow_mut().as_mut() {
                    ov.tick();
                }
            });
            LRESULT(0)
        }
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => unsafe { DefWindowProcW(h, msg, w, l) },
    }
}

fn ease(t: f64) -> f64 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

impl Ov {
    fn origin(&self) -> (f64, f64) {
        let r = frame_rect(self.target);
        (r.left as f64, r.top as f64)
    }

    /// 当前（动画中）的相对位置：沿一条略带弧度的曲线
    fn current(&self) -> (f64, f64) {
        let t = (self.anim.elapsed().as_secs_f64() / MOVE_DUR.as_secs_f64()).min(1.0);
        let e = ease(t);
        let (dx, dy) = (self.to.0 - self.from.0, self.to.1 - self.from.1);
        let bow = (std::f64::consts::PI * e).sin() * 0.12;
        (self.from.0 + dx * e - dy * bow, self.from.1 + dy * e + dx * bow)
    }

    fn apply(&mut self, c: Cmd) {
        let now = Instant::now();
        match c {
            Cmd::Move { target, x, y } => {
                let target = HWND(target as *mut _);
                let was = self.visible && self.fade.is_none();
                let cur_screen = if was {
                    let (ox, oy) = self.origin();
                    let (cx, cy) = self.current();
                    Some((ox + cx, oy + cy))
                } else {
                    None
                };
                self.target = target;
                let (ox, oy) = self.origin();
                self.from = match cur_screen {
                    Some((sx, sy)) => (sx - ox, sy - oy),
                    None => {
                        // 刚出现：从窗口中间出发
                        let r = frame_rect(target);
                        (((r.right - r.left) / 2) as f64, ((r.bottom - r.top) / 2) as f64)
                    }
                };
                self.to = (x as f64 - ox, y as f64 - oy);
                self.anim = now;
                self.last = now;
                self.fade = None;
                if !self.visible {
                    self.visible = true;
                    self.drawn = None;
                    unsafe {
                        let _ = SetTimer(Some(self.hwnd), TIMER_ID, 16, None);
                    }
                }
            }
            Cmd::Pulse => {
                if self.visible {
                    self.pulse = Some(now);
                    self.last = now;
                }
            }
            Cmd::Touch => self.last = now,
            Cmd::Hide => {
                if self.visible && self.fade.is_none() {
                    self.fade = Some(now);
                }
            }
        }
    }

    fn hide_now(&mut self) {
        self.visible = false;
        self.fade = None;
        self.pulse = None;
        self.drawn = None;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_ID);
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    fn tick(&mut self) {
        if !self.visible {
            return;
        }
        unsafe {
            if !IsWindow(Some(self.target)).as_bool() || IsIconic(self.target).as_bool() || !IsWindowVisible(self.target).as_bool() {
                self.hide_now();
                return;
            }
        }
        if self.fade.is_none() && self.last.elapsed() > IDLE {
            self.fade = Some(Instant::now());
        }
        let alpha = match self.fade {
            Some(f) => {
                let a = 1.0 - f.elapsed().as_secs_f32() / FADE_DUR.as_secs_f32();
                if a <= 0.0 {
                    self.hide_now();
                    return;
                }
                a
            }
            None => 1.0,
        };
        let pulse_t = self.pulse.map(|p| p.elapsed().as_secs_f32() / PULSE_DUR.as_secs_f32()).filter(|t| *t < 1.0);
        if pulse_t.is_none() {
            self.pulse = None;
        }
        let scale = window_scale(self.target).clamp(0.75, 4.0);
        let (ox, oy) = self.origin();
        let (cx, cy) = self.current();
        let (tx, ty) = ((ox + cx).round() as i32, (oy + cy).round() as i32);
        let key = (tx, ty, (scale * 100.0) as i32, pulse_t.map(|t| (t * 1000.0) as i32).unwrap_or(-1), (alpha * 255.0) as u8);
        let first = self.drawn.is_none();
        if self.drawn == Some(key) {
            return;
        }
        self.drawn = Some(key);
        let size = (CANVAS * scale).ceil() as i32;
        if self.surf.as_ref().map(|s| s.size != size).unwrap_or(true) {
            self.surf = Surface::new(size);
        }
        let Some(surf) = &self.surf else { return };
        let px = unsafe { std::slice::from_raw_parts_mut(surf.bits, (size * size) as usize) };
        render(px, size, scale, pulse_t);
        let tip = (TIP * scale).round() as i32;
        let dst = POINT { x: tx - tip, y: ty - tip };
        let sz = SIZE { cx: size, cy: size };
        let src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: (alpha * 255.0) as u8, AlphaFormat: AC_SRC_ALPHA as u8 };
        unsafe {
            let screen = GetDC(None);
            let _ = UpdateLayeredWindow(self.hwnd, Some(screen), Some(&dst), Some(&sz), Some(surf.dc), Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA);
            ReleaseDC(None, screen);
            if first {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
            // 隔一会儿重新声明置顶（别的置顶窗口可能盖上来）
            if first || self.topmost_at.elapsed() > Duration::from_secs(1) {
                self.topmost_at = Instant::now();
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
            }
        }
    }
}

impl Surface {
    fn new(size: i32) -> Option<Self> {
        unsafe {
            let screen = GetDC(None);
            let dc = CreateCompatibleDC(Some(screen));
            ReleaseDC(None, screen);
            let bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: size, biHeight: -size, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
                ..Default::default()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            match CreateDIBSection(Some(dc), &bi, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(bmp) if !bits.is_null() => {
                    let old = SelectObject(dc, bmp.into());
                    Some(Surface { size, dc, bmp, old, bits: bits as *mut u32 })
                }
                _ => {
                    let _ = DeleteDC(dc);
                    None
                }
            }
        }
    }
}

// ---------------------------------------------------------------- 绘制（软件光栅化，预乘 BGRA）

/// 箭头轮廓（相对箭头尖，1 倍单位）
const ARROW: [(f32, f32); 7] = [(0.0, 0.0), (0.0, 19.0), (4.6, 14.8), (7.8, 21.6), (10.6, 20.4), (7.5, 13.8), (13.4, 13.6)];
/// 「AI」两个字的 3×5 点阵
const GLYPH_A: [u8; 5] = [0b010, 0b101, 0b111, 0b101, 0b101];
const GLYPH_I: [u8; 5] = [0b111, 0b010, 0b010, 0b010, 0b111];

fn seg_dist(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let (wx, wy) = (p.0 - a.0, p.1 - a.1);
    let t = ((wx * vx + wy * vy) / (vx * vx + vy * vy)).clamp(0.0, 1.0);
    let (dx, dy) = (wx - vx * t, wy - vy * t);
    (dx * dx + dy * dy).sqrt()
}

/// 到箭头边界的距离与是否在里面
fn arrow_sd(p: (f32, f32)) -> (f32, bool) {
    let mut d = f32::MAX;
    let mut inside = false;
    let n = ARROW.len();
    for i in 0..n {
        let a = ARROW[i];
        let b = ARROW[(i + 1) % n];
        d = d.min(seg_dist(p, a, b));
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) + a.0 {
            inside = !inside;
        }
    }
    (d, inside)
}

/// 圆角矩形的有符号距离（里面为负）
fn rrect_sd(p: (f32, f32), x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let qx = (p.0 - cx).abs() - (w / 2.0 - r);
    let qy = (p.1 - cy).abs() - (h / 2.0 - r);
    let out = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    out + qx.max(qy).min(0.0) - r
}

/// 「AI」标签：左上角、尺寸（1 倍单位）
const LABEL: (f32, f32, f32, f32) = (12.0, 19.5, 15.5, 10.0);
const FONT_PX: f32 = 1.45;

fn label_text(p: (f32, f32)) -> bool {
    let (lx, ly, lw, lh) = LABEL;
    let tw = 7.0 * FONT_PX;
    let th = 5.0 * FONT_PX;
    let x0 = lx + (lw - tw) / 2.0;
    let y0 = ly + (lh - th) / 2.0;
    let cx = ((p.0 - x0) / FONT_PX).floor();
    let cy = ((p.1 - y0) / FONT_PX).floor();
    if cx < 0.0 || cy < 0.0 || cx >= 7.0 || cy >= 5.0 {
        return false;
    }
    let (col, row) = (cx as usize, cy as usize);
    let (g, c) = match col {
        0..=2 => (&GLYPH_A, col),
        4..=6 => (&GLYPH_I, col - 4),
        _ => return false,
    };
    g[row] >> (2 - c) & 1 == 1
}

/// 颜色与不透明度的「叠在上面」
fn over(dst: &mut (f32, f32, f32, f32), c: (f32, f32, f32), a: f32) {
    // dst 是预乘的 (r,g,b,a)
    dst.0 = c.0 * a + dst.0 * (1.0 - a);
    dst.1 = c.1 * a + dst.1 * (1.0 - a);
    dst.2 = c.2 * a + dst.2 * (1.0 - a);
    dst.3 = a + dst.3 * (1.0 - a);
}

fn render(px: &mut [u32], size: i32, s: f32, pulse: Option<f32>) {
    let white = (1.0, 1.0, 1.0);
    let black = (0.0, 0.0, 0.0);
    let aa = 1.0 / s; // 一个像素在单位坐标里的宽度
    let border = 1.7;
    for y in 0..size {
        for x in 0..size {
            let p = ((x as f32 + 0.5) / s - TIP, (y as f32 + 0.5) / s - TIP);
            let mut c = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            // 点击时的扩散圈
            if let Some(t) = pulse {
                let r = 3.0 + 15.0 * t;
                let th = 2.4 * (1.0 - 0.5 * t);
                let d = ((p.0 * p.0 + p.1 * p.1).sqrt() - r).abs();
                let cov = ((th / 2.0 - d) / aa + 0.5).clamp(0.0, 1.0) * 0.85 * (1.0 - t);
                if cov > 0.0 {
                    over(&mut c, ACCENT, cov);
                }
            }
            // 箭头的柔和阴影
            let (sd, sin) = arrow_sd((p.0 - 0.7, p.1 - 1.2));
            let sh = if sin { 0.30 } else { 0.30 * (1.0 - sd / 2.2).max(0.0) };
            if sh > 0.0 {
                over(&mut c, black, sh);
            }
            // 箭头：强调色描边 + 白色内填充
            let (d, inside) = arrow_sd(p);
            let cov = if inside { 1.0 } else { (1.0 - d / aa).clamp(0.0, 1.0) };
            if cov > 0.0 {
                let wmix = if inside { ((d - border) / aa + 0.5).clamp(0.0, 1.0) } else { 0.0 };
                let col = (ACCENT.0 + (white.0 - ACCENT.0) * wmix, ACCENT.1 + (white.1 - ACCENT.1) * wmix, ACCENT.2 + (white.2 - ACCENT.2) * wmix);
                over(&mut c, col, cov);
            }
            // 「AI」标签：强调色圆角底 + 白字（2×2 超采样）
            let (lx, ly, lw, lh) = LABEL;
            let ld = rrect_sd(p, lx, ly, lw, lh, lh / 2.0);
            let lcov = (0.5 - ld / aa).clamp(0.0, 1.0);
            if lcov > 0.0 {
                let mut tcov = 0.0;
                for (ox, oy) in [(-0.25f32, -0.25f32), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)] {
                    if label_text((p.0 + ox * aa, p.1 + oy * aa)) {
                        tcov += 0.25;
                    }
                }
                let col = (ACCENT.0 + (1.0 - ACCENT.0) * tcov, ACCENT.1 + (1.0 - ACCENT.1) * tcov, ACCENT.2 + (1.0 - ACCENT.2) * tcov);
                over(&mut c, col, lcov);
            }
            let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
            px[(y * size + x) as usize] = (to8(c.3) << 24) | (to8(c.0) << 16) | (to8(c.1) << 8) | to8(c.2);
        }
    }
}
