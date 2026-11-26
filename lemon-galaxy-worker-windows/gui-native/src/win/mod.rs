//! Win32 的薄封装：宽字符串、DPI、字体与颜色、窗口类与消息分派（Rust 闭包）、系统控件、界面线程的调度
//! （别的线程投递、延后执行、定时器、后台任务）、消息循环。
//!
//! 只静态导入 XP 就有的函数；Vista 起才有的在 `compat` 里运行时加载。所有界面对象只在界面线程上用。

pub mod compat;
pub mod dialog;
pub mod layout;

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicI32, AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{COLORREF, GlobalFree, HANDLE, HGLOBAL, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, GetFocus, IsWindowEnabled, SetFocus};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

pub type Proc = dyn Fn(HWND, u32, WPARAM, LPARAM) -> Option<LRESULT>;

/// 窗口类：顶层窗口（主窗口、对话框；带程序图标）、子容器（页面）、「i」说明图标
pub const CLASS_TOP: &str = "LemonGalaxyWorkerUI";
pub const CLASS_PANEL: &str = "LemonGalaxyPanel";
pub const CLASS_INFO: &str = "LemonGalaxyInfo";

/// 程序图标（build.rs 写的图标组 1）
pub const APP_ICON: u16 = 1;

pub const VK_RETURN_: usize = 0x0D;
pub const VK_ESCAPE_: usize = 0x1B;

/// 界面线程的唤醒消息（别的线程投递了事情）
pub const WM_APP_WAKE: u32 = WM_APP + 1;
/// 托盘图标的回调消息
pub const WM_APP_TRAY: u32 = WM_APP + 2;

// ---------------- 字符串 ----------------

/// 以 0 结尾的 UTF-16
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn from_wide(b: &[u16]) -> String {
    let n = b.iter().position(|c| *c == 0).unwrap_or(b.len());
    String::from_utf16_lossy(&b[..n])
}

pub fn pcw(v: &[u16]) -> PCWSTR {
    PCWSTR(v.as_ptr())
}

pub fn loword(v: usize) -> u32 {
    (v & 0xffff) as u32
}

pub fn hiword(v: usize) -> u32 {
    ((v >> 16) & 0xffff) as u32
}

pub fn key(h: HWND) -> isize {
    h.0 as isize
}

pub fn hwnd(k: isize) -> HWND {
    HWND(k as *mut core::ffi::c_void)
}

pub fn hinstance() -> HINSTANCE {
    unsafe { GetModuleHandleW(PCWSTR::null()).map(|m| HINSTANCE(m.0)).unwrap_or_default() }
}

// ---------------- DPI、颜色 ----------------

static DPI: AtomicI32 = AtomicI32::new(96);

/// 设计尺寸（96 DPI 下的像素）换成当前系统 DPI 下的像素
pub fn px(dip: i32) -> i32 {
    (dip * DPI.load(Ordering::Relaxed) + 48) / 96
}

pub fn dpi() -> i32 {
    DPI.load(Ordering::Relaxed)
}

pub fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

pub fn sys(idx: SYS_COLOR_INDEX) -> u32 {
    unsafe { GetSysColor(idx) }
}

/// a 与 b 按 t（0–1，b 的比例）混合
pub fn blend(a: u32, b: u32, t: f32) -> u32 {
    let ch = |c: u32, s: u32| -> u8 {
        let x = ((c >> s) & 0xff) as f32;
        x.round() as u8
    };
    let mix = |s: u32| -> u8 {
        let (x, y) = (ch(a, s) as f32, ch(b, s) as f32);
        (x + (y - x) * t).round().clamp(0.0, 255.0) as u8
    };
    rgb(mix(0), mix(8), mix(16))
}

/// 文字的语气（颜色）
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Normal,
    Dim,
    Error,
    Warning,
    Accent,
}

pub fn high_contrast() -> bool {
    let mut hc = HIGHCONTRASTW { cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32, ..Default::default() };
    unsafe { SystemParametersInfoW(SPI_GETHIGHCONTRAST, hc.cbSize, Some(&mut hc as *mut _ as _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)).is_ok() && hc.dwFlags.0 & HCF_HIGHCONTRASTON.0 != 0 }
}

pub fn tone_color(t: Tone) -> u32 {
    if high_contrast() {
        return if t == Tone::Dim { sys(COLOR_GRAYTEXT) } else { sys(COLOR_WINDOWTEXT) };
    }
    match t {
        Tone::Normal => sys(COLOR_WINDOWTEXT),
        Tone::Dim => blend(sys(COLOR_WINDOWTEXT), sys(COLOR_WINDOW), 0.45),
        Tone::Error => rgb(0xC4, 0x2B, 0x1C),
        Tone::Warning => rgb(0x9A, 0x5B, 0x00),
        Tone::Accent => rgb(0x00, 0x66, 0xB4),
    }
}

/// 提示条、徽标的底色
pub fn tone_fill(t: Tone) -> u32 {
    if high_contrast() {
        return sys(COLOR_WINDOW);
    }
    match t {
        Tone::Error => rgb(0xFD, 0xE7, 0xE9),
        Tone::Warning => rgb(0xFF, 0xF4, 0xCE),
        Tone::Accent => rgb(0xE5, 0xF1, 0xFB),
        Tone::Normal | Tone::Dim => blend(sys(COLOR_WINDOW), sys(COLOR_WINDOWTEXT), 0.07),
    }
}

/// 分组框、分隔线的颜色
pub fn line_color() -> u32 {
    blend(sys(COLOR_WINDOW), sys(COLOR_BTNSHADOW), 0.55)
}

thread_local! {
    static BRUSHES: RefCell<HashMap<u32, isize>> = RefCell::new(HashMap::new());
}

/// 纯色画刷（缓存，程序退出前不释放）
pub fn brush(color: u32) -> HBRUSH {
    BRUSHES.with(|b| {
        let k = *b.borrow_mut().entry(color).or_insert_with(|| unsafe { CreateSolidBrush(COLORREF(color)).0 as isize });
        HBRUSH(k as *mut core::ffi::c_void)
    })
}

// ---------------- 字体 ----------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Font {
    Normal,
    Bold,
    /// 页面标题
    Title,
    /// 概览抬头、占位页标题
    Hero,
    Mono,
    /// 临时协助的设备码、验证码
    Code,
}

struct Fonts {
    normal: isize,
    bold: isize,
    title: isize,
    hero: isize,
    mono: isize,
    code: isize,
    height: i32,
}

static FONTS: OnceLock<Fonts> = OnceLock::new();

fn message_font() -> LOGFONTW {
    // XP 的 NONCLIENTMETRICSW 没有最后的 iPaddedBorderWidth：先按完整大小取，失败再按 XP 的大小取
    let full = std::mem::size_of::<NONCLIENTMETRICSW>() as u32;
    for size in [full, full - 4] {
        let mut m = NONCLIENTMETRICSW { cbSize: size, ..Default::default() };
        if unsafe { SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, size, Some(&mut m as *mut _ as _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) }.is_ok() {
            return m.lfMessageFont;
        }
    }
    let mut lf = LOGFONTW { lfHeight: -px(12), lfWeight: FW_NORMAL.0 as i32, lfCharSet: DEFAULT_CHARSET, ..Default::default() };
    let face = wide("Tahoma");
    lf.lfFaceName[..face.len()].copy_from_slice(&face);
    lf
}

fn make_font(base: &LOGFONTW, scale: f32, bold: bool, face: Option<&str>) -> isize {
    let mut lf = *base;
    lf.lfHeight = (lf.lfHeight as f32 * scale).round() as i32;
    lf.lfWidth = 0;
    if bold {
        lf.lfWeight = FW_BOLD.0 as i32;
    }
    if let Some(f) = face {
        lf.lfFaceName = [0; 32];
        let w = wide(f);
        lf.lfFaceName[..w.len()].copy_from_slice(&w);
        lf.lfCharSet = DEFAULT_CHARSET;
        lf.lfPitchAndFamily = (FIXED_PITCH.0 | FF_MODERN.0) as u8;
    }
    unsafe { CreateFontIndirectW(&lf).0 as isize }
}

/// 系统里有没有这个字体（选进去后看实际用的字体名）
fn font_exists(name: &str) -> bool {
    let f = make_font(&message_font(), 1.0, false, Some(name));
    let found = with_dc(|dc| unsafe {
        let old = SelectObject(dc, HGDIOBJ(f as _));
        let mut buf = [0u16; 64];
        GetTextFaceW(dc, Some(&mut buf));
        SelectObject(dc, old);
        from_wide(&buf).eq_ignore_ascii_case(name)
    });
    unsafe {
        let _ = DeleteObject(HGDIOBJ(f as _));
    }
    found
}

pub fn init_fonts() {
    let dc_dpi = with_dc(|dc| unsafe { GetDeviceCaps(Some(dc), LOGPIXELSY) });
    if dc_dpi > 0 {
        DPI.store(dc_dpi, Ordering::Relaxed);
    }
    let base = message_font();
    let mono = ["Consolas", "Lucida Console", "Courier New"].into_iter().find(|f| font_exists(f)).unwrap_or("Courier New");
    let normal = make_font(&base, 1.0, false, None);
    let height = with_dc(|dc| unsafe {
        let old = SelectObject(dc, HGDIOBJ(normal as _));
        let mut tm = TEXTMETRICW::default();
        let _ = GetTextMetricsW(dc, &mut tm);
        SelectObject(dc, old);
        tm.tmHeight
    });
    let _ = FONTS.set(Fonts {
        normal,
        bold: make_font(&base, 1.0, true, None),
        title: make_font(&base, 1.55, true, None),
        hero: make_font(&base, 1.3, true, None),
        mono: make_font(&base, 1.0, false, Some(mono)),
        code: make_font(&base, 2.1, true, Some(mono)),
        height: height.max(px(13)),
    });
}

pub fn font(f: Font) -> HFONT {
    let fs = FONTS.get().expect("init_fonts");
    let k = match f {
        Font::Normal => fs.normal,
        Font::Bold => fs.bold,
        Font::Title => fs.title,
        Font::Hero => fs.hero,
        Font::Mono => fs.mono,
        Font::Code => fs.code,
    };
    HFONT(k as _)
}

/// 正文一行的高度
pub fn line_h() -> i32 {
    FONTS.get().map(|f| f.height).unwrap_or(px(15))
}

/// 屏幕 DC 上做点事（量文字等）
pub fn with_dc<T>(f: impl FnOnce(HDC) -> T) -> T {
    unsafe {
        let dc = GetDC(None);
        let r = f(dc);
        ReleaseDC(None, dc);
        r
    }
}

/// 文字的大小：`wrap` 时按 `max_w` 折行
pub fn measure(fnt: HFONT, text: &str, max_w: i32, wrap: bool) -> (i32, i32) {
    let text = if text.is_empty() { " " } else { text };
    let mut w: Vec<u16> = text.encode_utf16().collect();
    with_dc(|dc| unsafe {
        let old = SelectObject(dc, HGDIOBJ(fnt.0));
        let mut r = RECT { left: 0, top: 0, right: max_w.max(1), bottom: 0 };
        // 与 STATIC 控件自己画字时的折行方式一致（DT_WORDBREAK | DT_EXPANDTABS）
        let flags = if wrap { DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX | DT_EXPANDTABS } else { DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX };
        DrawTextW(dc, &mut w, &mut r, flags);
        SelectObject(dc, old);
        (r.right - r.left, r.bottom - r.top)
    })
}

// ---------------- 窗口类与消息分派 ----------------

thread_local! {
    static PROCS: RefCell<HashMap<isize, Rc<Proc>>> = RefCell::new(HashMap::new());
    static CMDS: RefCell<HashMap<isize, Rc<dyn Fn(u32)>>> = RefCell::new(HashMap::new());
    static NOTIFYS: RefCell<HashMap<isize, Rc<dyn Fn(*const NMHDR) -> Option<LRESULT>>>> = RefCell::new(HashMap::new());
    static DRAWS: RefCell<HashMap<isize, Rc<dyn Fn(&DRAWITEMSTRUCT)>>> = RefCell::new(HashMap::new());
    static TONES: RefCell<HashMap<isize, Tone>> = RefCell::new(HashMap::new());
    /// 控件自己的底色（提示条里的文字），没有就用父窗口的
    static CTL_BG: RefCell<HashMap<isize, u32>> = RefCell::new(HashMap::new());
    /// 容器的底色
    static BG: RefCell<HashMap<isize, u32>> = RefCell::new(HashMap::new());
    /// 布局时用的固定宽度（像素）
    static WIDTHS: RefCell<HashMap<isize, i32>> = RefCell::new(HashMap::new());
    /// 这些顶层窗口走 IsDialogMessage（Tab 切换焦点、回车、Esc）
    static ROOTS: RefCell<Vec<isize>> = const { RefCell::new(Vec::new()) };
    /// 顶层窗口自己处理的按键（回车、Esc），返回 true 表示处理了
    static KEYS: RefCell<HashMap<isize, Rc<dyn Fn(u32) -> bool>>> = RefCell::new(HashMap::new());
    /// 顶层窗口 → 它的说明提示框
    static TIPS: RefCell<HashMap<isize, isize>> = RefCell::new(HashMap::new());
}

pub fn set_proc(h: HWND, f: impl Fn(HWND, u32, WPARAM, LPARAM) -> Option<LRESULT> + 'static) {
    PROCS.with(|m| m.borrow_mut().insert(key(h), Rc::new(f)));
}

/// 控件发给父窗口的 WM_COMMAND（参数是通知码）
pub fn on_command(c: Ctl, f: impl Fn(u32) + 'static) {
    CMDS.with(|m| m.borrow_mut().insert(key(c.0), Rc::new(f)));
}

/// 按钮、复选框被点
pub fn on_click(c: Ctl, f: impl Fn() + 'static) {
    on_command(c, move |code| {
        if code == BN_CLICKED {
            f()
        }
    });
}

pub fn on_notify(c: Ctl, f: impl Fn(*const NMHDR) -> Option<LRESULT> + 'static) {
    NOTIFYS.with(|m| m.borrow_mut().insert(key(c.0), Rc::new(f)));
}

pub fn on_draw(c: Ctl, f: impl Fn(&DRAWITEMSTRUCT) + 'static) {
    DRAWS.with(|m| m.borrow_mut().insert(key(c.0), Rc::new(f)));
}

pub fn set_tone(c: Ctl, t: Tone) {
    TONES.with(|m| m.borrow_mut().insert(key(c.0), t));
    c.redraw();
}

pub fn set_ctl_bg(c: Ctl, color: Option<u32>) {
    CTL_BG.with(|m| match color {
        Some(v) => m.borrow_mut().insert(key(c.0), v),
        None => m.borrow_mut().remove(&key(c.0)),
    });
}

pub fn set_bg(h: HWND, color: u32) {
    BG.with(|m| m.borrow_mut().insert(key(h), color));
}

pub fn bg_of(h: HWND) -> u32 {
    BG.with(|m| m.borrow().get(&key(h)).copied()).unwrap_or_else(|| sys(COLOR_WINDOW))
}

pub fn add_root(h: HWND) {
    ROOTS.with(|r| r.borrow_mut().push(key(h)));
}

/// 顶层窗口里按下回车、Esc 时先问它（不靠 IsDialogMessage 找默认按钮：按钮在容器里，它找不到）
pub fn on_root_key(h: HWND, f: impl Fn(u32) -> bool + 'static) {
    KEYS.with(|m| m.borrow_mut().insert(key(h), Rc::new(f)));
}

/// 焦点在展开着的下拉框上（回车、Esc 交给它收起列表）
pub fn combo_dropped() -> bool {
    let f = Ctl(unsafe { GetFocus() });
    !f.0.0.is_null() && f.class().eq_ignore_ascii_case("combobox") && f.send(CB_GETDROPPEDSTATE, 0, 0) != 0
}

fn forget(h: HWND) {
    let k = key(h);
    CMDS.with(|m| m.borrow_mut().remove(&k));
    NOTIFYS.with(|m| m.borrow_mut().remove(&k));
    DRAWS.with(|m| m.borrow_mut().remove(&k));
    TONES.with(|m| m.borrow_mut().remove(&k));
    CTL_BG.with(|m| m.borrow_mut().remove(&k));
    WIDTHS.with(|m| m.borrow_mut().remove(&k));
    BG.with(|m| m.borrow_mut().remove(&k));
    COMBO_IDS.with(|m| m.borrow_mut().remove(&k));
    SHIELDS.with(|m| m.borrow_mut().remove(&k));
    ROOTS.with(|r| r.borrow_mut().retain(|x| *x != k));
    KEYS.with(|m| m.borrow_mut().remove(&k));
}

unsafe extern "system" fn forget_child(h: HWND, _: LPARAM) -> windows::core::BOOL {
    forget(h);
    PROCS.with(|m| m.borrow_mut().remove(&key(h)));
    true.into()
}

unsafe extern "system" fn wnd_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let p = PROCS.with(|m| m.borrow().get(&key(h)).cloned());
    if let Some(p) = p {
        if let Some(r) = p(h, msg, wp, lp) {
            return r;
        }
    }
    unsafe {
        match msg {
            WM_COMMAND if lp.0 != 0 => {
                let f = CMDS.with(|m| m.borrow().get(&lp.0).cloned());
                if let Some(f) = f {
                    f(hiword(wp.0));
                    return LRESULT(0);
                }
            }
            WM_NOTIFY if lp.0 != 0 => {
                let nm = lp.0 as *const NMHDR;
                let from = key((*nm).hwndFrom);
                let f = NOTIFYS.with(|m| m.borrow().get(&from).cloned());
                if let Some(f) = f {
                    if let Some(r) = f(nm) {
                        return r;
                    }
                }
            }
            WM_DRAWITEM if lp.0 != 0 => {
                let d = &*(lp.0 as *const DRAWITEMSTRUCT);
                let f = DRAWS.with(|m| m.borrow().get(&key(d.hwndItem)).cloned());
                if let Some(f) = f {
                    f(d);
                    return LRESULT(1);
                }
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN | WM_CTLCOLORLISTBOX => {
                let dc = HDC(wp.0 as _);
                let c = lp.0;
                let bg = CTL_BG.with(|m| m.borrow().get(&c).copied()).unwrap_or_else(|| bg_of(h));
                let tone = TONES.with(|m| m.borrow().get(&c).copied());
                let disabled = !IsWindowEnabled(hwnd(c)).as_bool();
                let fg = if disabled { sys(COLOR_GRAYTEXT) } else { tone_color(tone.unwrap_or(Tone::Normal)) };
                SetTextColor(dc, COLORREF(fg));
                SetBkColor(dc, COLORREF(bg));
                // 只读文字框按不透明画（透明的话改文字、选中时会留下旧字的残影）
                let is_edit = Ctl(hwnd(c)).class().eq_ignore_ascii_case("edit");
                SetBkMode(dc, if is_edit { OPAQUE } else { TRANSPARENT });
                return LRESULT(brush(bg).0 as isize);
            }
            WM_DESTROY => {
                // 子孙控件的回调一并去掉（HWND 以后可能被复用）
                let _ = EnumChildWindows(Some(h), Some(forget_child), LPARAM(0));
                if let Some(t) = TIPS.with(|m| m.borrow_mut().remove(&key(h))) {
                    let _ = DestroyWindow(hwnd(t));
                }
            }
            WM_NCDESTROY => {
                forget(h);
                PROCS.with(|m| m.borrow_mut().remove(&key(h)));
            }
            _ => {}
        }
        DefWindowProcW(h, msg, wp, lp)
    }
}

pub fn register_classes() {
    unsafe {
        let inst = hinstance();
        let icon = |size: i32| LoadImageW(Some(inst), PCWSTR(APP_ICON as usize as *const u16), IMAGE_ICON, size, size, LR_DEFAULTCOLOR).map(|h| HICON(h.0)).unwrap_or_default();
        let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let top = wide(CLASS_TOP);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: inst,
            hIcon: icon(GetSystemMetrics(SM_CXICON)),
            hIconSm: icon(GetSystemMetrics(SM_CXSMICON)),
            hCursor: cursor,
            hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as usize as _),
            lpszClassName: pcw(&top),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        for name in [CLASS_PANEL, CLASS_INFO] {
            let n = wide(name);
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(wnd_proc),
                hInstance: inst,
                hCursor: cursor,
                lpszClassName: pcw(&n),
                ..Default::default()
            };
            RegisterClassExW(&wc);
        }
    }
}

/// 自己画、自己摆的子窗口（侧栏、提示条），消息由 `set_proc` 登记的闭包处理
pub fn child_window(parent: HWND, ex: u32, style: u32) -> HWND {
    let cls = wide(CLASS_PANEL);
    unsafe { CreateWindowExW(WINDOW_EX_STYLE(ex), pcw(&cls), PCWSTR::null(), WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | style), 0, 0, 0, 0, Some(parent), None, Some(hinstance()), None).unwrap_or_default() }
}

/// 程序图标（指定边长）
pub fn app_icon(size: i32) -> HICON {
    unsafe { LoadImageW(Some(hinstance()), PCWSTR(APP_ICON as usize as *const u16), IMAGE_ICON, size, size, LR_DEFAULTCOLOR).map(|h| HICON(h.0)).unwrap_or_default() }
}

// ---------------- 控件 ----------------

/// 一个窗口或控件（只是句柄，可随意复制）
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ctl(pub HWND);

static NEXT_ID: AtomicI32 = AtomicI32::new(1000);

pub fn create(class: &str, text: &str, style: u32, ex: u32, parent: HWND, id: Option<i32>) -> Ctl {
    let c = wide(class);
    let t = wide(text);
    let id = id.unwrap_or_else(|| NEXT_ID.fetch_add(1, Ordering::Relaxed));
    let h = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(ex),
            pcw(&c),
            pcw(&t),
            WINDOW_STYLE(style | WS_CHILD.0 | WS_VISIBLE.0),
            0,
            0,
            0,
            0,
            Some(parent),
            Some(HMENU(id as isize as _)),
            Some(hinstance()),
            None,
        )
        .unwrap_or_default()
    };
    let ctl = Ctl(h);
    ctl.set_font(Font::Normal);
    ctl
}

pub const SS_NOPREFIX_: u32 = 0x80;
pub const SS_CENTER_: u32 = 0x1;

/// 文字（自动折行，不认 & 前缀）
pub fn label(parent: HWND, text: &str) -> Ctl {
    create("STATIC", text, SS_NOPREFIX_, 0, parent, None)
}

pub fn label_font(parent: HWND, text: &str, f: Font) -> Ctl {
    let c = label(parent, text);
    c.set_font(f);
    c
}

pub fn label_tone(parent: HWND, text: &str, t: Tone) -> Ctl {
    let c = label(parent, text);
    set_tone(c, t);
    c
}

pub fn center_label(parent: HWND, text: &str, f: Font, t: Tone) -> Ctl {
    let c = create("STATIC", text, SS_NOPREFIX_ | SS_CENTER_, 0, parent, None);
    c.set_font(f);
    set_tone(c, t);
    c
}

/// 按钮。`id` 给 IDOK / IDCANCEL 时回车、Esc 会按它
pub fn button(parent: HWND, text: &str) -> Ctl {
    create("BUTTON", text, BS_PUSHBUTTON as u32 | WS_TABSTOP.0, 0, parent, None)
}

pub fn button_id(parent: HWND, text: &str, id: i32, default: bool) -> Ctl {
    let s = if default { BS_DEFPUSHBUTTON } else { BS_PUSHBUTTON } as u32;
    create("BUTTON", text, s | WS_TABSTOP.0, 0, parent, Some(id))
}

pub fn checkbox(parent: HWND, text: &str) -> Ctl {
    create("BUTTON", text, BS_AUTOCHECKBOX as u32 | WS_TABSTOP.0, 0, parent, None)
}

pub fn edit(parent: HWND, text: &str, password: bool) -> Ctl {
    let mut s = ES_AUTOHSCROLL as u32 | WS_TABSTOP.0;
    if password {
        s |= ES_PASSWORD as u32;
    }
    create("EDIT", text, s, WS_EX_CLIENTEDGE.0, parent, None)
}

/// 只读、无边框的文字框：看起来像文字，可以选中复制
pub fn value(parent: HWND, text: &str) -> Ctl {
    create("EDIT", text, ES_AUTOHSCROLL as u32 | ES_READONLY as u32, 0, parent, None)
}

pub fn combo(parent: HWND, items: &[(&str, &str)]) -> Ctl {
    let c = create("COMBOBOX", "", CBS_DROPDOWNLIST as u32 | WS_VSCROLL.0 | WS_TABSTOP.0, 0, parent, None);
    c.set_items(items);
    c
}

/// 转圈（不定进度条）
pub fn marquee(parent: HWND) -> Ctl {
    let c = create("msctls_progress32", "", PBS_MARQUEE, 0, parent, None);
    c.send(PBM_SETMARQUEE, 1, 40);
    set_width(c, px(110));
    c
}

/// 超链接（SysLink，Common Controls 6；XP 起都有）。文字里用 `<a>…</a>` 标出链接
pub fn link(parent: HWND, text: &str, url: &'static str) -> Ctl {
    let c = create("SysLink", text, WS_TABSTOP.0, 0, parent, None);
    on_notify(c, move |nm| {
        let code = unsafe { (*nm).code };
        if code == NM_CLICK || code == NM_RETURN {
            open_url(url);
            return Some(LRESULT(0));
        }
        None
    });
    c
}

/// 「i」说明图标：鼠标停在上面（或点一下）显示说明
pub fn info(parent: HWND, text: &str) -> Ctl {
    let c = create(CLASS_INFO, "", 0, 0, parent, None);
    set_width(c, px(16));
    set_proc(c.0, move |h, msg, _wp, _lp| match msg {
        WM_PAINT => {
            paint_info(h);
            Some(LRESULT(0))
        }
        WM_ERASEBKGND => Some(LRESULT(1)),
        WM_LBUTTONUP => {
            if let Some(t) = tip_of(h) {
                unsafe { SendMessageW(t, TTM_POPUP, None, None) };
            }
            Some(LRESULT(0))
        }
        _ => None,
    });
    add_tip(c, text);
    c
}

fn paint_info(h: HWND) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let dc = BeginPaint(h, &mut ps);
        let mut rc = RECT::default();
        let _ = GetClientRect(h, &mut rc);
        let parent = GetParent(h).unwrap_or_default();
        let bg = bg_of(parent);
        FillRect(dc, &rc, brush(bg));
        let s = (rc.right - rc.left).min(rc.bottom - rc.top) - 1;
        let x = (rc.right - rc.left - s) / 2;
        let y = (rc.bottom - rc.top - s) / 2;
        let color = tone_color(Tone::Dim);
        let pen = CreatePen(PS_SOLID, 1, COLORREF(color));
        let old_pen = SelectObject(dc, HGDIOBJ(pen.0));
        let old_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));
        let _ = Ellipse(dc, x, y, x + s, y + s);
        SelectObject(dc, old_brush);
        SelectObject(dc, old_pen);
        let _ = DeleteObject(HGDIOBJ(pen.0));
        let old_font = SelectObject(dc, HGDIOBJ(font(Font::Bold).0));
        SetTextColor(dc, COLORREF(color));
        SetBkMode(dc, TRANSPARENT);
        let mut t: Vec<u16> = "i".encode_utf16().collect();
        let mut r = RECT { left: x, top: y, right: x + s, bottom: y + s };
        DrawTextW(dc, &mut t, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(dc, old_font);
        let _ = EndPaint(h, &ps);
    }
}

fn root_of(h: HWND) -> HWND {
    unsafe { GetAncestor(h, GA_ROOT) }
}

fn tip_of(h: HWND) -> Option<HWND> {
    let r = key(root_of(h));
    TIPS.with(|m| m.borrow().get(&r).copied()).map(hwnd)
}

/// 给控件加悬停说明（同一个顶层窗口共用一个提示框）
pub fn add_tip(c: Ctl, text: &str) {
    let root = root_of(c.0);
    let t = match tip_of(c.0) {
        Some(t) => t,
        None => unsafe {
            let cls = wide("tooltips_class32");
            let t = CreateWindowExW(WS_EX_TOPMOST, pcw(&cls), PCWSTR::null(), WINDOW_STYLE(WS_POPUP.0 | TTS_ALWAYSTIP | TTS_NOPREFIX), 0, 0, 0, 0, Some(root), None, Some(hinstance()), None).unwrap_or_default();
            SendMessageW(t, TTM_SETMAXTIPWIDTH, None, Some(LPARAM(px(340) as isize)));
            SendMessageW(t, TTM_SETDELAYTIME, Some(WPARAM(TTDT_AUTOPOP as usize)), Some(LPARAM(30000)));
            SendMessageW(t, TTM_SETDELAYTIME, Some(WPARAM(TTDT_INITIAL as usize)), Some(LPARAM(250)));
            TIPS.with(|m| m.borrow_mut().insert(key(root), key(t)));
            t
        },
    };
    let mut text = wide(text);
    // XP 的 Common Controls 也认的大小（不含 lpReserved）
    let size = std::mem::offset_of!(TTTOOLINFOW, lpReserved) as u32;
    let ti = TTTOOLINFOW {
        cbSize: size,
        uFlags: TTF_IDISHWND | TTF_SUBCLASS,
        hwnd: unsafe { GetParent(c.0).unwrap_or_default() },
        uId: key(c.0) as usize,
        lpszText: PWSTR(text.as_mut_ptr()),
        ..Default::default()
    };
    unsafe { SendMessageW(t, TTM_ADDTOOLW, None, Some(LPARAM(&ti as *const _ as isize))) };
}

pub fn set_width(c: Ctl, w: i32) {
    WIDTHS.with(|m| m.borrow_mut().insert(key(c.0), w));
}

pub fn fixed_width(c: Ctl) -> Option<i32> {
    WIDTHS.with(|m| m.borrow().get(&key(c.0)).copied())
}

/// 按钮的标准高度
pub fn button_h() -> i32 {
    px(23).max(line_h() + px(9))
}

/// 输入框的高度
pub fn edit_h() -> i32 {
    px(21).max(line_h() + px(8))
}

impl Ctl {
    pub fn send(&self, msg: u32, wp: usize, lp: isize) -> isize {
        unsafe { SendMessageW(self.0, msg, Some(WPARAM(wp)), Some(LPARAM(lp))).0 }
    }

    pub fn set_font(&self, f: Font) {
        self.send(WM_SETFONT, font(f).0 as usize, 1);
    }

    pub fn get_font(&self) -> HFONT {
        HFONT(self.send(WM_GETFONT, 0, 0) as _)
    }

    pub fn text(&self) -> String {
        unsafe {
            let n = GetWindowTextLengthW(self.0);
            let mut b = vec![0u16; n as usize + 1];
            GetWindowTextW(self.0, &mut b);
            from_wide(&b)
        }
    }

    pub fn set_text(&self, s: &str) {
        if self.text() == s {
            return;
        }
        let w = wide(s);
        unsafe {
            let _ = SetWindowTextW(self.0, pcw(&w));
        }
    }

    pub fn class(&self) -> String {
        let mut b = [0u16; 64];
        unsafe { GetClassNameW(self.0, &mut b) };
        from_wide(&b)
    }

    pub fn style(&self) -> u32 {
        unsafe { get_long(self.0, GWL_STYLE) as u32 }
    }

    pub fn visible(&self) -> bool {
        unsafe { IsWindowVisible(self.0).as_bool() }
    }

    pub fn show(&self, on: bool) {
        unsafe {
            let _ = ShowWindow(self.0, if on { SW_SHOWNA } else { SW_HIDE });
        }
    }

    pub fn enable(&self, on: bool) {
        unsafe {
            let _ = EnableWindow(self.0, on);
        }
    }

    pub fn checked(&self) -> bool {
        self.send(BM_GETCHECK, 0, 0) == BST_CHECKED.0 as isize
    }

    pub fn set_checked(&self, on: bool) {
        self.send(BM_SETCHECK, if on { BST_CHECKED.0 } else { BST_UNCHECKED.0 } as usize, 0);
    }

    pub fn focus(&self) {
        unsafe {
            let _ = SetFocus(Some(self.0));
        }
    }

    pub fn redraw(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.0), None, true);
        }
    }

    /// 销毁控件（系统控件收不到我们的 WM_NCDESTROY，回调在这里一并去掉）
    pub fn destroy(&self) {
        forget(self.0);
        unsafe {
            let _ = EnumChildWindows(Some(self.0), Some(forget_child), LPARAM(0));
            let _ = DestroyWindow(self.0);
        }
    }

    /// 按钮旁的盾牌（要管理员授权；Vista 起才画，XP 上什么都不做）
    pub fn set_shield(&self, on: bool) {
        self.send(BCM_SETSHIELD, 0, on as isize);
        SHIELDS.with(|s| {
            if on {
                s.borrow_mut().insert(key(self.0));
            } else {
                s.borrow_mut().remove(&key(self.0));
            }
        });
    }

    /// 输入框里的灰色提示
    pub fn set_cue(&self, s: &str) {
        let w = wide(s);
        self.send(EM_SETCUEBANNER, 0, w.as_ptr() as isize);
    }

    pub fn set_limit(&self, n: usize) {
        self.send(EM_LIMITTEXT, n, 0);
    }

    pub fn select_all(&self) {
        self.send(EM_SETSEL, 0, -1);
    }

    // 下拉框：每项 (id, 显示文字)，id 存在一个并行的表里
    pub fn set_items(&self, items: &[(&str, &str)]) {
        self.send(CB_RESETCONTENT, 0, 0);
        let ids: Vec<String> = items.iter().map(|(id, _)| id.to_string()).collect();
        for (_, text) in items {
            let w = wide(text);
            self.send(CB_ADDSTRING, 0, w.as_ptr() as isize);
        }
        COMBO_IDS.with(|m| m.borrow_mut().insert(key(self.0), ids));
    }

    pub fn selected_id(&self) -> Option<String> {
        let i = self.send(CB_GETCURSEL, 0, 0);
        if i < 0 {
            return None;
        }
        COMBO_IDS.with(|m| m.borrow().get(&key(self.0)).and_then(|v| v.get(i as usize).cloned()))
    }

    pub fn select_id(&self, id: &str) -> bool {
        let i = COMBO_IDS.with(|m| m.borrow().get(&key(self.0)).and_then(|v| v.iter().position(|x| x.eq_ignore_ascii_case(id))));
        match i {
            Some(i) => {
                self.send(CB_SETCURSEL, i, 0);
                true
            }
            None => false,
        }
    }

    pub fn select_index(&self, i: usize) {
        self.send(CB_SETCURSEL, i, 0);
    }
}

thread_local! {
    static COMBO_IDS: RefCell<HashMap<isize, Vec<String>>> = RefCell::new(HashMap::new());
    /// 画了盾牌的按钮（布局时给盾牌留出宽度）
    static SHIELDS: RefCell<std::collections::HashSet<isize>> = RefCell::new(std::collections::HashSet::new());
}

pub unsafe fn get_long(h: HWND, idx: WINDOW_LONG_PTR_INDEX) -> isize {
    #[cfg(target_pointer_width = "64")]
    {
        unsafe { GetWindowLongPtrW(h, idx) }
    }
    #[cfg(target_pointer_width = "32")]
    {
        unsafe { GetWindowLongW(h, idx) as isize }
    }
}

/// 控件的首选大小（布局用）
pub fn pref(c: Ctl) -> (i32, i32) {
    let class = c.class().to_ascii_lowercase();
    let fixed = fixed_width(c);
    let f = c.get_font();
    match class.as_str() {
        "button" => {
            let kind = c.style() & 0xF;
            let (tw, th) = measure(f, &c.text(), 10_000, false);
            if kind == BS_AUTOCHECKBOX as u32 || kind == BS_CHECKBOX as u32 || kind == BS_AUTORADIOBUTTON as u32 {
                (fixed.unwrap_or(tw + px(22)), th.max(px(13)) + px(4))
            } else {
                let shield = if SHIELDS.with(|s| s.borrow().contains(&key(c.0))) && !compat::legacy() { px(20) } else { 0 };
                (fixed.unwrap_or((tw + px(26) + shield).max(px(80))), button_h())
            }
        }
        "edit" => {
            let ro = c.style() & ES_READONLY as u32 != 0;
            if ro {
                let (tw, th) = measure(f, &c.text(), 10_000, false);
                (fixed.unwrap_or(tw + px(10)), th + px(2))
            } else {
                (fixed.unwrap_or(px(220)), edit_h())
            }
        }
        "combobox" => {
            let mut rc = RECT::default();
            unsafe {
                let _ = GetWindowRect(c.0, &mut rc);
            }
            // 收起时的高度：窗口高度（控件自己定），拿不到时按选择框的行高算
            let item = c.send(CB_GETITEMHEIGHT, usize::MAX, 0) as i32;
            let h = if rc.bottom - rc.top > px(8) { rc.bottom - rc.top } else if item > 0 { item + 6 } else { edit_h() };
            (fixed.unwrap_or(px(200)), h)
        }
        "msctls_progress32" => (fixed.unwrap_or(px(110)), px(14)),
        "syslink" => {
            let plain = c.text().replace("<a>", "").replace("</a>", "");
            let (tw, th) = measure(f, &plain, 10_000, false);
            (fixed.unwrap_or(tw + px(4)), th + px(2))
        }
        _ if class == CLASS_INFO.to_ascii_lowercase() => (px(16), px(16)),
        _ => {
            let (tw, th) = measure(f, &c.text(), 10_000, false);
            (fixed.unwrap_or(tw), th)
        }
    }
}

// ---------------- 界面线程的调度 ----------------

static DISPATCHER: AtomicIsize = AtomicIsize::new(0);
type Job = Box<dyn FnOnce() + Send>;
static POSTED: Mutex<Vec<Job>> = Mutex::new(Vec::new());

thread_local! {
    static DEFERRED: RefCell<VecDeque<Box<dyn FnOnce()>>> = RefCell::new(VecDeque::new());
    static TIMERS: RefCell<HashMap<usize, Box<dyn FnOnce()>>> = RefCell::new(HashMap::new());
    static INTERVALS: RefCell<HashMap<usize, Rc<dyn Fn()>>> = RefCell::new(HashMap::new());
    static BG_DONE: RefCell<HashMap<u64, Box<dyn FnOnce(Box<dyn Any + Send>)>>> = RefCell::new(HashMap::new());
    static NEXT_TIMER: Cell<usize> = const { Cell::new(100) };
    static NEXT_BG: Cell<u64> = const { Cell::new(1) };
    static WAKE_HANDLERS: RefCell<Vec<Rc<Proc>>> = const { RefCell::new(Vec::new()) };
}

/// 隐藏的调度窗口（顶层、不显示）：别的线程投递的事、定时器、托盘回调都发给它
pub fn dispatcher() -> HWND {
    hwnd(DISPATCHER.load(Ordering::Relaxed))
}

pub fn init_dispatcher() -> HWND {
    let cls = wide(CLASS_TOP);
    let h = unsafe { CreateWindowExW(WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0), pcw(&cls), PCWSTR::null(), WS_POPUP, 0, 0, 0, 0, None, None, Some(hinstance()), None).unwrap_or_default() };
    DISPATCHER.store(key(h), Ordering::Relaxed);
    set_proc(h, |h, msg, wp, lp| {
        match msg {
            WM_APP_WAKE => {
                run_posted();
                return Some(LRESULT(0));
            }
            WM_TIMER => {
                let id = wp.0;
                if let Some(f) = TIMERS.with(|m| m.borrow_mut().remove(&id)) {
                    unsafe {
                        let _ = KillTimer(Some(h), id);
                    }
                    f();
                    return Some(LRESULT(0));
                }
                if let Some(f) = INTERVALS.with(|m| m.borrow().get(&id).cloned()) {
                    f();
                    return Some(LRESULT(0));
                }
            }
            _ => {}
        }
        // 托盘回调、TaskbarCreated、关机等交给登记的处理
        let hs: Vec<Rc<Proc>> = WAKE_HANDLERS.with(|v| v.borrow().clone());
        for f in hs {
            if let Some(r) = f(h, msg, wp, lp) {
                return Some(r);
            }
        }
        None
    });
    h
}

/// 调度窗口上的其他消息（托盘回调等）
pub fn on_dispatcher(f: impl Fn(HWND, u32, WPARAM, LPARAM) -> Option<LRESULT> + 'static) {
    WAKE_HANDLERS.with(|v| v.borrow_mut().push(Rc::new(f)));
}

/// 从任意线程把一件事交给界面线程做
pub fn ui_post(f: impl FnOnce() + Send + 'static) {
    if let Ok(mut q) = POSTED.lock() {
        q.push(Box::new(f));
    }
    unsafe {
        let _ = PostMessageW(Some(dispatcher()), WM_APP_WAKE, WPARAM(0), LPARAM(0));
    }
}

/// 界面线程上：等当前这条消息处理完再做（避免在回调里重入）
pub fn defer(f: impl FnOnce() + 'static) {
    DEFERRED.with(|q| q.borrow_mut().push_back(Box::new(f)));
    unsafe {
        let _ = PostMessageW(Some(dispatcher()), WM_APP_WAKE, WPARAM(0), LPARAM(0));
    }
}

fn run_posted() {
    loop {
        let jobs: Vec<Job> = POSTED.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
        let mut any = !jobs.is_empty();
        for j in jobs {
            j();
        }
        while let Some(f) = DEFERRED.with(|q| q.borrow_mut().pop_front()) {
            any = true;
            f();
        }
        if !any {
            break;
        }
    }
}

/// `ms` 毫秒后在界面线程上做一次
pub fn set_timeout(ms: u32, f: impl FnOnce() + 'static) -> usize {
    let id = NEXT_TIMER.with(|n| {
        let v = n.get();
        n.set(v + 1);
        v
    });
    TIMERS.with(|m| m.borrow_mut().insert(id, Box::new(f)));
    unsafe { SetTimer(Some(dispatcher()), id, ms, None) };
    id
}

/// 每 `ms` 毫秒做一次
pub fn set_interval(ms: u32, f: impl Fn() + 'static) -> usize {
    let id = NEXT_TIMER.with(|n| {
        let v = n.get();
        n.set(v + 1);
        v
    });
    INTERVALS.with(|m| m.borrow_mut().insert(id, Rc::new(f)));
    unsafe { SetTimer(Some(dispatcher()), id, ms, None) };
    id
}

/// 在后台线程做 `work`（会阻塞的事：提权运行、查服务、读文件），做完在界面线程上调 `then`
pub fn spawn_bg<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, then: impl FnOnce(T) + 'static) {
    let id = NEXT_BG.with(|n| {
        let v = n.get();
        n.set(v + 1);
        v
    });
    BG_DONE.with(|m| {
        m.borrow_mut().insert(
            id,
            Box::new(move |v: Box<dyn Any + Send>| {
                if let Ok(v) = v.downcast::<T>() {
                    then(*v)
                }
            }),
        )
    });
    let spawned = std::thread::Builder::new().name("ui-bg".into()).spawn(move || {
        let r = work();
        ui_post(move || {
            if let Some(f) = BG_DONE.with(|m| m.borrow_mut().remove(&id)) {
                f(Box::new(r));
            }
        });
    });
    if spawned.is_err() {
        BG_DONE.with(|m| m.borrow_mut().remove(&id));
    }
}

fn dialog_root(msg: &MSG) -> Option<HWND> {
    if msg.hwnd.0.is_null() {
        return None;
    }
    let root = key(root_of(msg.hwnd));
    ROOTS.with(|r| r.borrow().contains(&root)).then(|| hwnd(root))
}

/// 滚轮给鼠标下面的窗口（XP / 7 默认给有焦点的那个：点过左边的页面列表后，右边就滚不动了）
fn wheel_to_hover(msg: &MSG) -> bool {
    if msg.message != WM_MOUSEWHEEL {
        return false;
    }
    let pt = POINT { x: (msg.lParam.0 & 0xffff) as i16 as i32, y: ((msg.lParam.0 >> 16) & 0xffff) as i16 as i32 };
    let mut target = unsafe { WindowFromPoint(pt) };
    if target.0.is_null() || target == msg.hwnd {
        return false;
    }
    let root = key(root_of(target));
    if !ROOTS.with(|r| r.borrow().contains(&root)) {
        return false;
    }
    // 下拉框不跟着滚（免得悬停时误改选项），交给它所在的页面
    if Ctl(target).class().eq_ignore_ascii_case("combobox") {
        target = unsafe { GetParent(target).unwrap_or(target) };
    }
    unsafe { SendMessageW(target, WM_MOUSEWHEEL, Some(msg.wParam), Some(msg.lParam)) };
    true
}

fn dispatch(msg: &MSG) {
    unsafe {
        if wheel_to_hover(msg) {
            return;
        }
        if let Some(root) = dialog_root(msg) {
            if msg.message == WM_KEYDOWN && (msg.wParam.0 == VK_RETURN_ || msg.wParam.0 == VK_ESCAPE_) && !combo_dropped() {
                let f = KEYS.with(|m| m.borrow().get(&key(root)).cloned());
                if let Some(f) = f {
                    if f(msg.wParam.0 as u32) {
                        return;
                    }
                }
            }
            if IsDialogMessageW(root, msg).as_bool() {
                return;
            }
        }
        let _ = TranslateMessage(msg);
        DispatchMessageW(msg);
    }
}

/// 主消息循环，返回 WM_QUIT 的退出码
pub fn run_loop() -> i32 {
    let mut msg = MSG::default();
    loop {
        let r = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if r.0 == 0 {
            return msg.wParam.0 as i32;
        }
        if r.0 == -1 {
            return 1;
        }
        dispatch(&msg);
    }
}

/// 嵌套的消息循环（模态对话框用），`done` 返回 true 时结束；收到 WM_QUIT 时重新投递后结束
pub fn pump_until(done: impl Fn() -> bool) {
    let mut msg = MSG::default();
    while !done() {
        let r = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if r.0 == 0 {
            unsafe { PostQuitMessage(msg.wParam.0 as i32) };
            return;
        }
        if r.0 == -1 {
            return;
        }
        dispatch(&msg);
    }
}

// ---------------- 其他 ----------------

pub fn copy_text(s: &str) -> bool {
    let w = wide(s);
    unsafe {
        if OpenClipboard(Some(dispatcher())).is_err() {
            return false;
        }
        let _ = EmptyClipboard();
        let bytes = w.len() * 2;
        let ok = match GlobalAlloc(GMEM_MOVEABLE, bytes) {
            Ok(g) => {
                let p = GlobalLock(g) as *mut u16;
                if p.is_null() {
                    let _ = GlobalFree(Some(g));
                    false
                } else {
                    std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
                    let _ = GlobalUnlock(g);
                    // CF_UNICODETEXT = 13；成功后内存归系统
                    match SetClipboardData(13, Some(HANDLE(g.0))) {
                        Ok(_) => true,
                        Err(_) => {
                            let _ = GlobalFree(Some(HGLOBAL(g.0)));
                            false
                        }
                    }
                }
            }
            Err(_) => false,
        };
        let _ = CloseClipboard();
        ok
    }
}

pub fn open_url(url: &str) {
    let u = wide(url);
    let op = wide("open");
    unsafe {
        ShellExecuteW(None, pcw(&op), pcw(&u), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

pub fn client_rect(h: HWND) -> RECT {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(h, &mut rc);
    }
    rc
}

pub fn cursor_pos() -> POINT {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    p
}

/// 把窗口拉到前面（从托盘、另一个实例唤起时）
pub fn bring_to_front(h: HWND) {
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        } else {
            let _ = ShowWindow(h, SW_SHOW);
        }
        let _ = SetWindowPos(h, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        let _ = SetWindowPos(h, Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        let _ = SetForegroundWindow(h);
    }
}

/// 显示器工作区（窗口所在的那块屏）
pub fn work_area(h: Option<HWND>) -> RECT {
    unsafe {
        let m = match h {
            Some(h) => MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST),
            None => MonitorFromPoint(cursor_pos(), MONITOR_DEFAULTTOPRIMARY),
        };
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(m, &mut mi).as_bool() {
            mi.rcWork
        } else {
            RECT { left: 0, top: 0, right: GetSystemMetrics(SM_CXSCREEN), bottom: GetSystemMetrics(SM_CYSCREEN) }
        }
    }
}
