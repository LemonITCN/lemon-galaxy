//! Windows 的 AI 应用操作（docs/computer-use.md）：UI Automation + 窗口消息，不抢前台，带 AI 专属光标。
//! 被控端会话代理（用户会话里）与 Windows 主控端进程内共用。
//!
//! 线程模型：所有 UIA / COM 对象都放在一个专用工作线程（MTA）里，`handle` 可以在任意线程调用，
//! 把请求排进队列等结果；单次最多等 15 秒，超时就丢掉这个工作线程（多半卡在无响应的目标应用上），下次重建。
//! AI 光标是另一个有消息循环的线程（cursor.rs）。
//!
//! 注意 Windows 7 兼容版：Win8 / Win10 才有的接口一律运行时动态加载，不能静态导入。

mod cursor;
mod keys;
mod shot;

use galaxy_proto::peer::ImageData;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::core::{Interface, BSTR, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::Security::{GetTokenInformation, TokenIntegrityLevel, TOKEN_MANDATORY_LABEL, TOKEN_QUERY};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW, WaitForInputIdle, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::System::Variant::{VariantClear, VARIANT, VT_BOOL, VT_BSTR, VT_I4};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::*;

/// 单次请求最多等多久（平台接口约定约 10 秒，这里留点余量）
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// 动作后等界面刷新再截图
const SETTLE: Duration = Duration::from_millis(300);
/// AI 光标移到动作位置的时长（动作前等它走完）
const CURSOR_MOVE: Duration = Duration::from_millis(280);
/// 截图长边上限
const SHOT_MAX: u32 = 1280;

type Reply = (String, Option<ImageData>);

struct Job {
    req: String,
    reply: mpsc::Sender<Reply>,
}

/// 工作线程的代号与队列（超时丢弃后换新的代号）
static WORKER: Mutex<Option<(u64, mpsc::Sender<Job>)>> = Mutex::new(None);
static WORKER_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 处理一个请求（JSON），返回 (给模型的文字, 可选窗口截图)。None 表示不支持。
pub fn handle(request: &str) -> Option<(String, Option<ImageData>)> {
    let (gen, tx) = {
        let mut g = WORKER.lock().unwrap_or_else(|e| e.into_inner());
        g.get_or_insert_with(|| (WORKER_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed), spawn_worker())).clone()
    };
    let (rtx, rrx) = mpsc::channel();
    if tx.send(Job { req: request.to_string(), reply: rtx }).is_err() {
        *WORKER.lock().unwrap_or_else(|e| e.into_inner()) = None;
        return Some((err("应用操作的工作线程已退出，请重试"), None));
    }
    match rrx.recv_timeout(REQUEST_TIMEOUT) {
        Ok(r) => Some(r),
        Err(_) => {
            // 卡住的工作线程丢掉不管（队列关闭后它做完手上的事自己退出），下次重建
            let mut g = WORKER.lock().unwrap_or_else(|e| e.into_inner());
            if g.as_ref().map(|s| s.0 == gen).unwrap_or(false) {
                *g = None;
            }
            Some((err("操作超时，目标应用可能没有响应；稍后再试，或先 get_app_state 看看它的状态"), None))
        }
    }
}

fn spawn_worker() -> mpsc::Sender<Job> {
    let (tx, rx) = mpsc::channel::<Job>();
    let _ = std::thread::Builder::new().name("win-cu".into()).spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        dpi_thread_per_monitor();
        let mut st = State::default();
        while let Ok(job) = rx.recv() {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| st.dispatch(&job.req))).unwrap_or_else(|_| (err("内部错误（已记录），请重试"), None));
            let _ = job.reply.send(r);
        }
    });
    tx
}

fn err(s: &str) -> String {
    format!("错误：{s}")
}

// ---------------------------------------------------------------- 动态加载的新系统接口

fn proc_addr(dll: &str, name: &[u8]) -> Option<unsafe extern "system" fn() -> isize> {
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    let w = wide(dll);
    unsafe {
        let m = LoadLibraryW(PCWSTR(w.as_ptr())).ok()?;
        GetProcAddress(m, windows::core::PCSTR(name.as_ptr()))
    }
}

/// 当前线程按每屏 DPI 感知（Win10 1607 起），让 UIA、窗口矩形、截图都用物理像素
pub(crate) fn dpi_thread_per_monitor() {
    type F = unsafe extern "system" fn(isize) -> isize;
    if let Some(f) = proc_addr("user32.dll", b"SetThreadDpiAwarenessContext\0") {
        unsafe {
            let f: F = std::mem::transmute(f);
            // DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 = -4，老一点的 Win10 退回 PER_MONITOR_AWARE = -3
            if f(-4) == 0 {
                f(-3);
            }
        }
    }
}

/// 窗口所在屏的缩放比例（Win10 有 GetDpiForWindow），老系统返回 1
pub(crate) fn window_scale(hwnd: HWND) -> f32 {
    type F = unsafe extern "system" fn(HWND) -> u32;
    if let Some(f) = proc_addr("user32.dll", b"GetDpiForWindow\0") {
        let d = unsafe { std::mem::transmute::<_, F>(f)(hwnd) };
        if d > 0 {
            return d as f32 / 96.0;
        }
    }
    1.0
}

pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

// ---------------------------------------------------------------- 应用与窗口

#[derive(Clone)]
struct Win {
    hwnd: HWND,
    title: String,
    kind: WinKind,
}

/// 窗口类别：默认窗口取主窗口，不取 Z 序最上面的（浏览器最上面常是地址栏下拉、气泡、对话框）
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum WinKind {
    Main,
    Dialog,
    Popup,
}

impl WinKind {
    fn word(self) -> &'static str {
        match self {
            WinKind::Main => "",
            WinKind::Dialog => " 对话框",
            WinKind::Popup => " 弹出",
        }
    }
}

fn win_kind(h: HWND) -> WinKind {
    unsafe {
        let style = GetWindowLongW(h, GWL_STYLE) as u32;
        let caption = style & WS_CAPTION.0 == WS_CAPTION.0;
        let owned = GetWindow(h, GW_OWNER).map(|o| !o.is_invalid()).unwrap_or(false);
        if class_name(h) == "#32770" {
            return WinKind::Dialog;
        }
        if owned {
            return if caption { WinKind::Dialog } else { WinKind::Popup };
        }
        if style & WS_POPUP.0 != 0 && !caption && style & WS_THICKFRAME.0 == 0 && style & WS_MAXIMIZE.0 == 0 {
            // 没有标题栏、不能拉伸的无主弹出窗口（Chromium 的浏览器主窗口有 WS_CAPTION / WS_THICKFRAME）
            return WinKind::Popup;
        }
        WinKind::Main
    }
}

struct App {
    pid: u32,
    /// 可执行文件名（notepad.exe）
    exe: String,
    /// 显示名（文件说明，没有就用文件名主干）
    name: String,
    wins: Vec<Win>,
    front: bool,
}

pub(crate) fn window_text(h: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub(crate) fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn window_pid(h: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    pid
}

fn is_cloaked(h: HWND) -> bool {
    let mut v = 0u32;
    unsafe { DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut v as *mut u32 as *mut _, 4).is_ok() && v != 0 }
}

/// 窗口看得见的边框（去掉 Win10 起那圈看不见的拖动边）
pub(crate) fn frame_rect(h: HWND) -> RECT {
    let mut r = RECT::default();
    unsafe {
        if DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut RECT as *mut _, std::mem::size_of::<RECT>() as u32).is_err() || r.right <= r.left {
            let _ = GetWindowRect(h, &mut r);
        }
    }
    r
}

/// UWP 应用的顶层窗口属于 ApplicationFrameHost，真正的进程是里面 CoreWindow 的进程
fn real_pid(h: HWND) -> u32 {
    let pid = window_pid(h);
    if class_name(h) == "ApplicationFrameWindow" {
        let cls = wide("Windows.UI.Core.CoreWindow");
        if let Ok(c) = unsafe { FindWindowExW(Some(h), None, PCWSTR(cls.as_ptr()), PCWSTR::null()) } {
            if !c.is_invalid() {
                return window_pid(c);
            }
        }
    }
    pid
}

fn top_windows() -> Vec<HWND> {
    unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> windows::core::BOOL {
        let v = unsafe { &mut *(lp.0 as *mut Vec<HWND>) };
        v.push(h);
        true.into()
    }
    let mut v: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut v as *mut _ as isize));
    }
    v
}

/// 用户能看到的应用窗口（按 Z 序，前面的在上面）
fn app_window(h: HWND) -> bool {
    unsafe {
        if !IsWindowVisible(h).as_bool() || is_cloaked(h) {
            return false;
        }
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TOOLWINDOW.0 != 0 && ex & WS_EX_APPWINDOW.0 == 0 {
            return false;
        }
        let cls = class_name(h);
        if matches!(cls.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "Windows.UI.Core.CoreWindow") {
            return false;
        }
        let r = frame_rect(h);
        if r.right - r.left < 2 || r.bottom - r.top < 2 {
            return false;
        }
        !window_text(h).trim().is_empty()
    }
}

fn exe_path(pid: u32) -> Option<String> {
    unsafe {
        let ph = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(ph, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
        let _ = CloseHandle(ph);
        ok.then(|| String::from_utf16_lossy(&buf[..n as usize]))
    }
}

/// 可执行文件的「文件说明」（如「记事本」「Google Chrome」）
fn file_description(path: &str) -> Option<String> {
    let w = wide(path);
    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(w.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(PCWSTR(w.as_ptr()), None, size, data.as_mut_ptr() as *mut _).ok()?;
        let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        let tr = wide("\\VarFileInfo\\Translation");
        let mut langs: Vec<(u16, u16)> = Vec::new();
        if VerQueryValueW(data.as_ptr() as *const _, PCWSTR(tr.as_ptr()), &mut p, &mut len).as_bool() && len >= 4 {
            let s = std::slice::from_raw_parts(p as *const u16, (len / 2) as usize);
            for c in s.chunks(2) {
                if c.len() == 2 {
                    langs.push((c[0], c[1]));
                }
            }
        }
        langs.extend([(0x0409, 0x04b0), (0x0409, 0x04e4), (0x0804, 0x04b0)]);
        for (l, cp) in langs {
            let q = wide(&format!("\\StringFileInfo\\{l:04x}{cp:04x}\\FileDescription"));
            if VerQueryValueW(data.as_ptr() as *const _, PCWSTR(q.as_ptr()), &mut p, &mut len).as_bool() && len > 1 {
                let s = std::slice::from_raw_parts(p as *const u16, len as usize);
                let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
                let d = String::from_utf16_lossy(&s[..end]).trim().to_string();
                if !d.is_empty() {
                    return Some(d);
                }
            }
        }
        None
    }
}

/// 进程完整性级别（低 0x1000 / 中 0x2000 / 高 0x3000 / 系统 0x4000）
fn integrity(pid: Option<u32>) -> Option<u32> {
    unsafe {
        let ph = match pid {
            Some(p) => OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, p).ok()?,
            None => GetCurrentProcess(),
        };
        let mut tok = HANDLE::default();
        let ok = OpenProcessToken(ph, TOKEN_QUERY, &mut tok).is_ok();
        if pid.is_some() {
            let _ = CloseHandle(ph);
        }
        if !ok {
            return None;
        }
        let mut buf = vec![0u8; 256];
        let mut n = 0u32;
        let r = GetTokenInformation(tok, TokenIntegrityLevel, Some(buf.as_mut_ptr() as *mut _), buf.len() as u32, &mut n);
        let _ = CloseHandle(tok);
        r.ok()?;
        let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
        let sid = label.Label.Sid;
        let cnt = *windows::Win32::Security::GetSidSubAuthorityCount(sid);
        if cnt == 0 {
            return None;
        }
        Some(*windows::Win32::Security::GetSidSubAuthority(sid, (cnt - 1) as u32))
    }
}

// ---------------------------------------------------------------- 状态

/// 截图坐标系：截图像素 = (屏幕坐标 - 原点) × 比例
#[derive(Clone, Copy)]
struct Geo {
    left: i32,
    top: i32,
    scale: f64,
}

impl Geo {
    fn to_shot(&self, x: i32, y: i32) -> (i32, i32) {
        (((x - self.left) as f64 * self.scale).round() as i32, ((y - self.top) as f64 * self.scale).round() as i32)
    }
    fn to_screen(&self, x: f64, y: f64) -> (i32, i32) {
        (self.left + (x / self.scale).round() as i32, self.top + (y / self.scale).round() as i32)
    }
}

struct Snap {
    hwnd: HWND,
    elems: Vec<IUIAutomationElement>,
    /// 每个编号的简短说明（如 Button "确定"），用于动作应答
    labels: Vec<String>,
    geo: Geo,
}

#[derive(Default)]
struct State {
    /// 动作开始前的前台窗口（动作把目标应用拉到前台时还回去）
    fg_before: Option<(HWND, u32)>,
    uia: Option<IUIAutomation>,
    /// 按 pid 缓存最近一次 get_app_state 的控件
    snaps: HashMap<u32, Snap>,
    desc: HashMap<String, String>,
}

impl State {
    fn uia(&mut self) -> Result<IUIAutomation, String> {
        if let Some(u) = &self.uia {
            return Ok(u.clone());
        }
        let u: IUIAutomation = unsafe {
            CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).or_else(|_| CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER))
        }
        .map_err(|e| err(&format!("创建 UI Automation 失败：{e}")))?;
        // 目标应用无响应时别一直等（Win8 起才有 IUIAutomation2）
        if let Ok(u2) = u.cast::<IUIAutomation2>() {
            unsafe {
                let _ = u2.SetConnectionTimeout(2000);
                let _ = u2.SetTransactionTimeout(5000);
            }
        }
        self.uia = Some(u.clone());
        Ok(u)
    }

    fn apps(&mut self) -> Vec<App> {
        let me = unsafe { GetCurrentProcessId() };
        let fg = unsafe { GetForegroundWindow() };
        let fg_pid = if fg.is_invalid() { 0 } else { real_pid(fg) };
        let mut list: Vec<App> = Vec::new();
        for h in top_windows() {
            if !app_window(h) {
                continue;
            }
            let pid = real_pid(h);
            if pid == me || pid == 0 {
                continue;
            }
            let title = window_text(h);
            let kind = win_kind(h);
            if let Some(a) = list.iter_mut().find(|a| a.pid == pid) {
                a.wins.push(Win { hwnd: h, title, kind });
                continue;
            }
            let path = exe_path(pid).unwrap_or_default();
            let exe = path.rsplit(['\\', '/']).next().unwrap_or("").to_string();
            let stem = exe.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or_else(|| exe.clone());
            let name = if path.is_empty() {
                title.clone()
            } else {
                self.desc.entry(path.clone()).or_insert_with(|| file_description(&path).unwrap_or_default()).clone()
            };
            let name = if name.is_empty() { if stem.is_empty() { title.clone() } else { stem } } else { name };
            list.push(App { pid, exe, name, wins: vec![Win { hwnd: h, title, kind }], front: pid == fg_pid });
        }
        // 窗口顺序：主窗口 > 前台窗口 > 面积大的（同类里稳定，编号尽量不变）
        for a in &mut list {
            a.wins.sort_by_key(|w| {
                let r = frame_rect(w.hwnd);
                (w.kind, w.hwnd != fg, -((r.right - r.left) as i64 * (r.bottom - r.top) as i64))
            });
        }
        list
    }

    /// 按 `app` 参数找应用：pid:N、显示名、可执行文件名（带不带 .exe）、窗口标题；先精确再前缀
    fn find_app(&mut self, q: &str) -> Result<App, String> {
        let q = q.trim();
        if q.is_empty() {
            return Err(err("缺少 app 参数（应用名、可执行文件名或 pid:1234）"));
        }
        let apps = self.apps();
        let ql = q.to_lowercase();
        if let Some(p) = ql.strip_prefix("pid:").and_then(|s| s.trim().parse::<u32>().ok()) {
            return apps.into_iter().find(|a| a.pid == p).ok_or_else(|| err(&format!("没有找到 pid {p} 的有窗口应用，请先 list_apps")));
        }
        let keys = |a: &App| -> Vec<String> {
            let exe = a.exe.to_lowercase();
            let stem = exe.strip_suffix(".exe").unwrap_or(&exe).to_string();
            let mut k = vec![a.name.to_lowercase(), exe, stem];
            k.extend(a.wins.iter().map(|w| w.title.to_lowercase()));
            k
        };
        let pick = |exact: bool| apps.iter().position(|a| keys(a).iter().any(|k| !k.is_empty() && if exact { *k == ql } else { k.starts_with(&ql) }));
        match pick(true).or_else(|| pick(false)) {
            Some(i) => Ok(apps.into_iter().nth(i).unwrap()),
            None => Err(err(&format!("没有找到正在运行的应用「{q}」，请先 list_apps 看看名称；没在运行可以 open_app"))),
        }
    }

    fn window_of(&self, app: &App, v: &Value) -> Result<(usize, HWND), String> {
        let i = v["window"].as_u64().unwrap_or(0) as usize;
        match app.wins.get(i) {
            Some(w) => Ok((i, w.hwnd)),
            None => Err(err(&format!("{} 只有 {} 个窗口（window 从 0 开始）", app.name, app.wins.len()))),
        }
    }

    fn dispatch(&mut self, req: &str) -> Reply {
        let v: Value = match serde_json::from_str(req) {
            Ok(v) => v,
            Err(e) => return (err(&format!("请求不是合法 JSON：{e}")), None),
        };
        let action = v["action"].as_str().unwrap_or("");
        let r = match action {
            "list_apps" => Ok((self.list_apps(), None)),
            "get_app_state" => self.get_app_state(&v),
            "click" => self.click(&v),
            "set_value" => self.set_value(&v),
            "type_text" => self.type_text(&v),
            "press_key" => self.press_key(&v),
            "scroll" => self.scroll(&v),
            "perform_action" => self.perform_action(&v),
            "open_app" => self.open_app(&v),
            "end" => {
                cursor::hide();
                self.snaps.clear();
                Ok(("已结束，AI 光标已收起".into(), None))
            }
            "" => Err(err("缺少 action")),
            a => Err(err(&format!("不支持的动作 {a}"))),
        };
        r.unwrap_or_else(|e| (e, None))
    }

    // ------------------------------------------------------------ list_apps / get_app_state

    fn list_apps(&mut self) -> String {
        let apps = self.apps();
        if apps.is_empty() {
            return "没有找到有窗口的应用（可能不在用户的桌面会话里，或屏幕已锁定）".into();
        }
        let mut s = format!("运行中的应用（{} 个）：\n", apps.len());
        for a in &apps {
            let first = a.wins.first().map(|w| w.title.as_str()).unwrap_or("");
            s += &format!("- {}（{}，pid {}）窗口 {} 个「{}」{}\n", a.name, a.exe, a.pid, a.wins.len(), trunc(first, 80), if a.front { " 前台" } else { "" });
        }
        s
    }

    fn get_app_state(&mut self, v: &Value) -> Result<Reply, String> {
        let app = self.find_app(v["app"].as_str().unwrap_or(""))?;
        let (wi, hwnd) = self.window_of(&app, v)?;
        let max_nodes = v["max_nodes"].as_u64().unwrap_or(400).clamp(10, 3000) as usize;
        let fr = frame_rect(hwnd);
        let iconic = unsafe { IsIconic(hwnd).as_bool() };
        let shot = if iconic { None } else { shot::capture(hwnd, SHOT_MAX) };
        let geo = match &shot {
            Some((_, g)) => *g,
            None => Geo { left: fr.left, top: fr.top, scale: 1.0 },
        };
        let uia = self.uia()?;
        let req = cache_request(&uia)?;
        let walker = unsafe { uia.ControlViewWalker() }.map_err(|e| err(&format!("UI Automation：{e}")))?;
        // Chromium（Chrome / Edge / Electron）第一次被读取时才打开辅助功能，控件树要过一会儿才填满：等一等再读
        // （只在本进程第一次读这个窗口时等；之后空的网页就是真的空，比如 about:blank）
        let chromium = class_name(hwnd).starts_with("Chrome_WidgetWin") && !self.snaps.get(&app.pid).map(|s| s.hwnd == hwnd).unwrap_or(false);
        let first_try = Instant::now();
        let mut tree = loop {
            let mut tree = Tree { uia: &uia, req: req.clone(), walker: walker.clone(), geo, win: fr, max: max_nodes, deadline: Instant::now() + Duration::from_secs(8), out: String::new(), elems: Vec::new(), labels: Vec::new(), depths: Vec::new(), roles: Vec::new(), truncated: false };
            let root = unsafe { uia.ElementFromHandleBuildCache(hwnd, &tree.req) }.map_err(|e| err(&format!("读取窗口控件失败：{e}")))?;
            tree.walk(&root, 0, true);
            if !chromium || tree.truncated || !tree.looks_unloaded() || first_try.elapsed() > Duration::from_millis(3000) {
                break tree;
            }
            std::thread::sleep(Duration::from_millis(400));
        };
        tree.depths.clear();
        let (w, h) = (fr.right - fr.left, fr.bottom - fr.top);
        let mut text = format!("应用 {}（{}，pid {}），窗口 {}/{}「{}」 位置 ({},{}) 大小 {}×{}", app.name, app.exe, app.pid, wi, app.wins.len(), trunc(&app.wins[wi].title, 100), fr.left, fr.top, w, h);
        match &shot {
            Some((img, _)) => text += &format!("，截图 {}×{}\n", img.width, img.height),
            None if iconic => text += "，窗口已最小化（没有截图；perform_action 对 [0] 执行 Restore 可以恢复，会显示窗口）\n",
            None => text += "，截图失败\n",
        }
        let others: Vec<String> = app
            .wins
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != wi)
            .map(|(j, w)| {
                let r = frame_rect(w.hwnd);
                format!("[{j}]「{}」{}×{}{}", trunc(&w.title, 50), r.right - r.left, r.bottom - r.top, w.kind.word())
            })
            .collect();
        if !others.is_empty() {
            text += &format!("其他窗口（用 window 参数指定）：{}\n", others.join("；"));
        }
        text += &tree.out;
        if tree.truncated {
            text += &format!("……（只列出前 {} 个控件，可以加大 max_nodes）\n", tree.elems.len());
        }
        if integrity(Some(app.pid)).zip(integrity(None)).map(|(t, m)| t > m).unwrap_or(false) {
            text += "注意：这个应用以更高权限（管理员）运行，点击、输入等窗口消息会被系统丢弃，只能读取。\n";
        }
        let Tree { elems, labels, .. } = tree;
        self.snaps.insert(app.pid, Snap { hwnd, elems, labels, geo });
        Ok((text, shot.map(|(i, _)| i)))
    }

    // ------------------------------------------------------------ 动作的公共部分

    /// 找应用 + 它的目标窗口（有快照用快照的窗口）
    fn target(&mut self, v: &Value) -> Result<(App, HWND), String> {
        let app = self.find_app(v["app"].as_str().unwrap_or(""))?;
        let hwnd = match (v.get("window"), self.snaps.get(&app.pid)) {
            (None, Some(s)) if unsafe { IsWindow(Some(s.hwnd)).as_bool() } => s.hwnd,
            _ => self.window_of(&app, v)?.1,
        };
        if let (Some(t), Some(m)) = (integrity(Some(app.pid)), integrity(None)) {
            if t > m {
                return Err(err(&format!("{} 以更高权限（管理员）运行，系统会丢弃发给它的窗口消息；可以改用全屏 mouse / keyboard 工具（会接管前台）", app.name)));
            }
        }
        let fg = unsafe { GetForegroundWindow() };
        self.fg_before = (!fg.is_invalid()).then(|| (fg, app.pid));
        Ok((app, hwnd))
    }

    /// 动作把目标应用拉到了前台（有的 UIA 代理会这样）：把前台还给原来的窗口
    fn give_back_foreground(&mut self) -> bool {
        let Some((prev, pid)) = self.fg_before.take() else { return false };
        let now = unsafe { GetForegroundWindow() };
        if now.is_invalid() || now == prev || real_pid(now) != pid || real_pid(prev) == pid || !unsafe { IsWindow(Some(prev)).as_bool() } {
            return false;
        }
        unsafe { SetForegroundWindow(prev).as_bool() }
    }

    fn element(&self, pid: u32, v: &Value) -> Result<Option<(usize, IUIAutomationElement, String)>, String> {
        let Some(i) = v.get("element").and_then(|e| e.as_u64().or_else(|| e.as_str().and_then(|s| s.trim_matches(['[', ']']).parse().ok()))) else {
            return Ok(None);
        };
        let i = i as usize;
        let Some(s) = self.snaps.get(&pid) else {
            return Err(err("还没有这个应用的控件编号，请先 get_app_state"));
        };
        let Some(e) = s.elems.get(i) else {
            return Err(err(&format!("控件编号 [{i}] 不存在（最近一次 get_app_state 只有 {} 个），请重新 get_app_state", s.elems.len())));
        };
        // 控件已经没了（界面变化）时，读属性会失败
        if unsafe { e.CurrentBoundingRectangle() }.is_err() {
            return Err(err(&format!("控件编号 [{i}] 已失效（界面变了），请先 get_app_state")));
        }
        Ok(Some((i, e.clone(), s.labels.get(i).cloned().unwrap_or_default())))
    }

    /// 截图坐标 → 屏幕坐标（按最近一次截图）
    fn point(&self, pid: u32, hwnd: HWND, v: &Value) -> Result<Option<(i32, i32)>, String> {
        let (Some(x), Some(y)) = (v["x"].as_f64(), v["y"].as_f64()) else {
            return Ok(None);
        };
        let geo = match self.snaps.get(&pid) {
            Some(s) => s.geo,
            None => {
                let fr = frame_rect(hwnd);
                Geo { left: fr.left, top: fr.top, scale: 1.0 }
            }
        };
        Ok(Some(geo.to_screen(x, y)))
    }

    /// 动作后稍等再截一张图，并更新这个应用的截图坐标系
    fn after(&mut self, pid: u32, hwnd: HWND, text: String) -> Result<Reply, String> {
        std::thread::sleep(SETTLE);
        let text = if self.give_back_foreground() { text + "（它把自己拉到了前台，已还给用户原来的窗口）" } else { text };
        cursor::touch();
        if !unsafe { IsWindow(Some(hwnd)).as_bool() } {
            return Ok((text + "（窗口已关闭）", None));
        }
        if unsafe { IsIconic(hwnd).as_bool() } {
            return Ok((text, None));
        }
        let shot = shot::capture(hwnd, SHOT_MAX);
        if let (Some((_, g)), Some(s)) = (&shot, self.snaps.get_mut(&pid)) {
            if s.hwnd == hwnd {
                s.geo = *g;
            }
        }
        Ok((text, shot.map(|(i, _)| i)))
    }

    // ------------------------------------------------------------ click

    fn click(&mut self, v: &Value) -> Result<Reply, String> {
        let (app, hwnd) = self.target(v)?;
        let count = v["count"].as_u64().unwrap_or(1).clamp(1, 2) as u32;
        let right = v["button"].as_str().map(|b| b.eq_ignore_ascii_case("right")).unwrap_or(false);
        if let Some((i, e, label)) = self.element(app.pid, v)? {
            let (cx, cy) = elem_center(&e).ok_or_else(|| err(&format!("[{i}] 没有位置，请先 get_app_state")))?;
            cursor::move_to(hwnd, cx, cy);
            std::thread::sleep(CURSOR_MOVE);
            cursor::pulse();
            let how = if count == 1 && !right { press_element(&e) } else { None };
            let text = match how {
                Some(how) => format!("已点击 [{i}] {label}（{how}）"),
                None => {
                    post_click(hwnd, cx, cy, right, count)?;
                    format!("已{}[{i}] {label}（窗口消息，坐标 {cx},{cy}）", click_word(right, count))
                }
            };
            return self.after(app.pid, hwnd, text);
        }
        let Some((sx, sy)) = self.point(app.pid, hwnd, v)? else {
            return Err(err("click 需要 element 或 x、y"));
        };
        cursor::move_to(hwnd, sx, sy);
        std::thread::sleep(CURSOR_MOVE);
        cursor::pulse();
        post_click(hwnd, sx, sy, right, count)?;
        let (x, y) = (v["x"].as_f64().unwrap_or(0.0), v["y"].as_f64().unwrap_or(0.0));
        let text = format!("已{}截图坐标 ({x:.0},{y:.0})（窗口消息；有些应用只认真实鼠标，没反应时可以改用控件编号或全屏 mouse 工具）", click_word(right, count));
        self.after(app.pid, hwnd, text)
    }

    // ------------------------------------------------------------ set_value

    fn set_value(&mut self, v: &Value) -> Result<Reply, String> {
        let (app, hwnd) = self.target(v)?;
        let value = v["value"].as_str().map(String::from).or_else(|| v.get("value").filter(|x| !x.is_null()).map(|x| x.to_string())).ok_or_else(|| err("set_value 需要 value"))?;
        let (i, e, label) = self.element(app.pid, v)?.ok_or_else(|| err("set_value 需要 element（控件编号）"))?;
        if let Some((cx, cy)) = elem_center(&e) {
            cursor::move_to(hwnd, cx, cy);
            std::thread::sleep(CURSOR_MOVE);
        }
        let how = set_element_value(&e, &value)?;
        self.after(app.pid, hwnd, format!("已设置 [{i}] {label} 的值为「{}」（{how}）", trunc(&value, 80)))
    }

    // ------------------------------------------------------------ type_text

    fn type_text(&mut self, v: &Value) -> Result<Reply, String> {
        let (app, hwnd) = self.target(v)?;
        let text = v["text"].as_str().ok_or_else(|| err("type_text 需要 text"))?.to_string();
        let replace = v["replace"].as_bool();
        if let Some((i, e, label)) = self.element(app.pid, v)? {
            if let Some((cx, cy)) = elem_center(&e) {
                cursor::move_to(hwnd, cx, cy);
                std::thread::sleep(CURSOR_MOVE);
            }
            let h = unsafe { e.CurrentNativeWindowHandle() }.ok().filter(|h| !h.is_invalid());
            // 单行输入框（地址栏、搜索框）默认先清空再输入，不然重复输入会接在旧内容后面
            let replace = replace.unwrap_or_else(|| single_line(&e, h));
            let how = match h {
                Some(h) if is_edit_class(h) => {
                    if replace {
                        send_timeout(h, EM_SETSEL, WPARAM(0), LPARAM(-1));
                    }
                    edit_insert(h, &text)?;
                    if replace { "替换原有内容" } else { "插入到光标处" }.to_string()
                }
                _ => {
                    // 没有自己窗口句柄的控件（网页、Chromium 地址栏、WPF / WinUI）：先让它获得焦点，再发键盘消息逐字输入。
                    // 不用 ValuePattern 直接写值：Chromium 的输入处理不知道（地址栏回车不跳转，网页收不到 input 事件）
                    let target = h.unwrap_or_else(|| focus_hwnd(hwnd));
                    if focus_element(&e) {
                        let mut how = String::from("键盘消息逐字输入");
                        if replace {
                            how = format!("{}后{how}", select_all(&e, target));
                        }
                        post_chars(target, &text);
                        how
                    } else {
                        match value_pattern(&e) {
                            Some(p) if !unsafe { p.CurrentIsReadOnly() }.map(|b| b.as_bool()).unwrap_or(true) => {
                                let old = if replace { String::new() } else { unsafe { p.CurrentValue() }.map(|b| b.to_string()).unwrap_or_default() };
                                unsafe { p.SetValue(&BSTR::from(format!("{old}{text}"))) }.map_err(|e| err(&format!("写入失败：{e}")))?;
                                format!("没能让它获得焦点，改用 ValuePattern {}；网页 / 浏览器可能不认，没效果时先 click 它再 type_text", if replace { "整体替换" } else { "追加到末尾" })
                            }
                            _ => {
                                post_chars(target, &text);
                                "没能让它获得焦点，键盘消息发给了窗口当前焦点".to_string()
                            }
                        }
                    }
                }
            };
            let now = settle_value(&e, &text);
            return self.after(app.pid, hwnd, format!("已在 [{i}] {label} 输入「{}」（{how}）{now}", trunc(&text, 80)));
        }
        let f = focus_hwnd(hwnd);
        let (cx, cy) = caret_pos(hwnd, f);
        cursor::move_to(hwnd, cx, cy);
        std::thread::sleep(CURSOR_MOVE);
        let replace = replace.unwrap_or(false);
        let how = if is_edit_class(f) {
            if replace {
                send_timeout(f, EM_SETSEL, WPARAM(0), LPARAM(-1));
            }
            edit_insert(f, &text)?;
            if replace { "替换焦点输入框的内容" } else { "插入到焦点输入框的光标处" }.to_string()
        } else {
            let mut how = String::from("键盘消息逐字输入到焦点控件");
            if replace {
                let a = keys::parse("ctrl+a").map_err(|e| err(&e))?;
                keys::send(f, &a).map_err(|e| err(&e))?;
                how = format!("ctrl+a 全选后{how}");
            }
            post_chars(f, &text);
            how
        };
        self.after(app.pid, hwnd, format!("已输入「{}」（{how}）", trunc(&text, 80)))
    }

    // ------------------------------------------------------------ press_key

    fn press_key(&mut self, v: &Value) -> Result<Reply, String> {
        let (app, hwnd) = self.target(v)?;
        let key = v["key"].as_str().ok_or_else(|| err("press_key 需要 key，如 Return、Tab、ctrl+s"))?;
        let combo = keys::parse(key).map_err(|e| err(&e))?;
        let f = focus_hwnd(hwnd);
        let (cx, cy) = caret_pos(hwnd, f);
        cursor::move_to(hwnd, cx, cy);
        std::thread::sleep(CURSOR_MOVE);
        let how = keys::send(f, &combo).map_err(|e| err(&e))?;
        self.after(app.pid, hwnd, format!("已按 {key}（{how}）"))
    }

    // ------------------------------------------------------------ scroll

    fn scroll(&mut self, v: &Value) -> Result<Reply, String> {
        let (app, hwnd) = self.target(v)?;
        let dir = v["direction"].as_str().unwrap_or("down").to_lowercase();
        if !matches!(dir.as_str(), "up" | "down" | "left" | "right") {
            return Err(err("direction 只能是 up / down / left / right"));
        }
        let pages = v["pages"].as_f64().unwrap_or(1.0).clamp(0.1, 20.0);
        let el = self.element(app.pid, v)?;
        let (sx, sy) = match (&el, self.point(app.pid, hwnd, v)?) {
            (Some((i, e, _)), _) => elem_center(e).ok_or_else(|| err(&format!("[{i}] 没有位置，请先 get_app_state")))?,
            (None, Some(p)) => p,
            (None, None) => {
                let fr = frame_rect(hwnd);
                ((fr.left + fr.right) / 2, (fr.top + fr.bottom) / 2)
            }
        };
        cursor::move_to(hwnd, sx, sy);
        std::thread::sleep(CURSOR_MOVE);
        // 先找能滚动的控件（自己或往上几层）用 ScrollPattern
        let start = match &el {
            Some((_, e, _)) => Some(e.clone()),
            None => {
                let uia = self.uia()?;
                unsafe { uia.ElementFromPoint(POINT { x: sx, y: sy }) }.ok().filter(|e| unsafe { e.CurrentProcessId() }.map(|p| p as u32 == app.pid).unwrap_or(false))
            }
        };
        if let Some(s) = start {
            let uia = self.uia()?;
            if let Some(p) = scroll_ancestor(&uia, &s, &dir) {
                let big = pages >= 1.0;
                let n = if big { pages.round().max(1.0) as u32 } else { ((pages * 10.0).round() as u32).max(1) };
                let (inc, dec) = if big { (ScrollAmount_LargeIncrement, ScrollAmount_LargeDecrement) } else { (ScrollAmount_SmallIncrement, ScrollAmount_SmallDecrement) };
                let mut ok = true;
                for _ in 0..n {
                    let r = unsafe {
                        match dir.as_str() {
                            "down" => p.Scroll(ScrollAmount_NoAmount, inc),
                            "up" => p.Scroll(ScrollAmount_NoAmount, dec),
                            "right" => p.Scroll(inc, ScrollAmount_NoAmount),
                            _ => p.Scroll(dec, ScrollAmount_NoAmount),
                        }
                    };
                    if r.is_err() {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    return self.after(app.pid, hwnd, format!("已向{}滚动 {pages} 页（ScrollPattern）", dir_word(&dir)));
                }
            }
        }
        // 退回滚轮消息：每页约 5 格
        let notches = ((pages * 5.0).round() as i32).max(1);
        post_wheel(hwnd, sx, sy, &dir, notches);
        self.after(app.pid, hwnd, format!("已向{}滚动 {pages} 页（滚轮消息）", dir_word(&dir)))
    }

    // ------------------------------------------------------------ perform_action

    fn perform_action(&mut self, v: &Value) -> Result<Reply, String> {
        let (app, hwnd) = self.target(v)?;
        let name = v["name"].as_str().ok_or_else(|| err("perform_action 需要 name"))?;
        let (i, e, label) = self.element(app.pid, v)?.ok_or_else(|| err("perform_action 需要 element（控件编号）"))?;
        let n = name.to_lowercase().replace(['_', '-', ' '], "");
        let n = n.strip_prefix("ax").unwrap_or(&n).to_string();
        let center = elem_center(&e);
        if let Some((cx, cy)) = center {
            cursor::move_to(hwnd, cx, cy);
            std::thread::sleep(CURSOR_MOVE);
        }
        let r: Result<(), String> = unsafe {
            match n.as_str() {
                "invoke" | "press" => pat::<IUIAutomationInvokePattern>(&e, UIA_InvokePatternId).and_then(|p| p.Invoke().map_err(com_err)),
                "toggle" => pat::<IUIAutomationTogglePattern>(&e, UIA_TogglePatternId).and_then(|p| p.Toggle().map_err(com_err)),
                "select" => pat::<IUIAutomationSelectionItemPattern>(&e, UIA_SelectionItemPatternId).and_then(|p| p.Select().map_err(com_err)),
                "addtoselection" => pat::<IUIAutomationSelectionItemPattern>(&e, UIA_SelectionItemPatternId).and_then(|p| p.AddToSelection().map_err(com_err)),
                "removefromselection" => pat::<IUIAutomationSelectionItemPattern>(&e, UIA_SelectionItemPatternId).and_then(|p| p.RemoveFromSelection().map_err(com_err)),
                "expand" => pat::<IUIAutomationExpandCollapsePattern>(&e, UIA_ExpandCollapsePatternId).and_then(|p| p.Expand().map_err(com_err)),
                "collapse" => pat::<IUIAutomationExpandCollapsePattern>(&e, UIA_ExpandCollapsePatternId).and_then(|p| p.Collapse().map_err(com_err)),
                "scrollintoview" | "scrolltovisible" => pat::<IUIAutomationScrollItemPattern>(&e, UIA_ScrollItemPatternId).and_then(|p| p.ScrollIntoView().map_err(com_err)),
                "dodefaultaction" => pat::<IUIAutomationLegacyIAccessiblePattern>(&e, UIA_LegacyIAccessiblePatternId).and_then(|p| p.DoDefaultAction().map_err(com_err)),
                "increment" | "decrement" => pat::<IUIAutomationRangeValuePattern>(&e, UIA_RangeValuePatternId).and_then(|p| {
                    let cur = p.CurrentValue().map_err(com_err)?;
                    let step = p.CurrentSmallChange().ok().filter(|s| *s > 0.0).unwrap_or(1.0);
                    let nv = if n == "increment" { cur + step } else { cur - step };
                    let nv = nv.clamp(p.CurrentMinimum().unwrap_or(f64::MIN), p.CurrentMaximum().unwrap_or(f64::MAX));
                    p.SetValue(nv).map_err(com_err)
                }),
                "focus" | "setfocus" | "raise" => e.SetFocus().map_err(com_err),
                "showmenu" | "contextmenu" => match center {
                    Some((cx, cy)) => post_click(hwnd, cx, cy, true, 1),
                    None => Err("控件没有位置".into()),
                },
                "close" => pat::<IUIAutomationWindowPattern>(&e, UIA_WindowPatternId).and_then(|p| p.Close().map_err(com_err)),
                "minimize" => pat::<IUIAutomationWindowPattern>(&e, UIA_WindowPatternId).and_then(|p| p.SetWindowVisualState(WindowVisualState_Minimized).map_err(com_err)),
                "maximize" => pat::<IUIAutomationWindowPattern>(&e, UIA_WindowPatternId).and_then(|p| p.SetWindowVisualState(WindowVisualState_Maximized).map_err(com_err)),
                "restore" => {
                    // 恢复最小化窗口但不激活
                    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                    Ok(())
                }
                _ => Err(format!("不认识的动作 {name}；可用：Invoke、Toggle、Select、AddToSelection、RemoveFromSelection、Expand、Collapse、ScrollIntoView、DoDefaultAction、Increment、Decrement、ShowMenu、Focus（会把窗口拉到前台）、Close、Minimize、Maximize、Restore")),
            }
        };
        r.map_err(|e| err(&format!("[{i}] {label} 执行 {name} 失败：{e}")))?;
        cursor::pulse();
        self.after(app.pid, hwnd, format!("已对 [{i}] {label} 执行 {name}"))
    }

    // ------------------------------------------------------------ open_app

    fn open_app(&mut self, v: &Value) -> Result<Reply, String> {
        let q = v["app"].as_str().unwrap_or("").trim().to_string();
        if q.is_empty() {
            return Err(err("open_app 需要 app（可执行文件名或路径，如 notepad、chrome、C:\\Tools\\x.exe）"));
        }
        if let Ok(a) = self.find_app(&q) {
            return Ok((format!("{}（{}，pid {}）已在运行，窗口 {} 个", a.name, a.exe, a.pid, a.wins.len()), None));
        }
        let fg_before = unsafe { GetForegroundWindow() };
        let pid = if crate::win::is_system() { launch_as_session_user(&q)? } else { launch_shell(&q)? };
        // 等它的窗口出来（最多约 5 秒）
        let deadline = Instant::now() + Duration::from_secs(5);
        let stem = q.rsplit(['\\', '/']).next().unwrap_or(&q).to_lowercase();
        let stem = stem.strip_suffix(".exe").unwrap_or(&stem).to_string();
        loop {
            let apps = self.apps();
            if let Some(a) = apps.iter().find(|a| (pid != 0 && a.pid == pid) || a.exe.to_lowercase().strip_suffix(".exe") == Some(stem.as_str())) {
                // 不少应用（对话框式的、自己调 SetForegroundWindow 的）不管启动参数照样抢到前台：尽量还给原来的窗口
                std::thread::sleep(Duration::from_millis(400));
                let fg = unsafe { GetForegroundWindow() };
                if !fg_before.is_invalid() && fg != fg_before && a.wins.iter().any(|w| w.hwnd == fg) && unsafe { IsWindow(Some(fg_before)).as_bool() } {
                    unsafe {
                        let _ = SetForegroundWindow(fg_before);
                    }
                }
                return Ok((format!("已启动 {}（{}，pid {}），窗口「{}」", a.name, a.exe, a.pid, a.wins.first().map(|w| trunc(&w.title, 80)).unwrap_or_default()), None));
            }
            if Instant::now() > deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        Ok((format!("已启动 {q}{}，暂时还没看到它的窗口；稍后 list_apps 再看", if pid != 0 { format!("（pid {pid}）") } else { String::new() }), None))
    }
}

/// 以本进程用户启动（主控端进程内）：ShellExecuteEx 能认「App Paths」里的名字（chrome、msedge 等）
fn launch_shell(q: &str) -> Result<u32, String> {
    let file = wide(q);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI,
        lpFile: PCWSTR(file.as_ptr()),
        nShow: SW_SHOWNOACTIVATE.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.map_err(|e| err(&format!("启动 {q} 失败：{e}（请给可执行文件名或完整路径）")))?;
    let mut pid = 0u32;
    // 32 位系统上这个结构是 1 字节对齐的，先把字段拷出来再用
    let hp = info.hProcess;
    if !hp.is_invalid() {
        unsafe {
            let _ = WaitForInputIdle(hp, 5000);
            pid = windows::Win32::System::Threading::GetProcessId(hp);
            let _ = CloseHandle(hp);
        }
    }
    Ok(pid)
}

/// 把应用名解析成可执行文件路径：带路径的原样用；否则查「App Paths」注册表，再按 PATH 搜索
fn resolve_exe(q: &str) -> Option<String> {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    if q.contains('\\') || q.contains('/') {
        return std::path::Path::new(q).exists().then(|| q.to_string());
    }
    let name = if q.to_lowercase().ends_with(".exe") { q.to_string() } else { format!("{q}.exe") };
    let key = wide(&format!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{name}"));
    let mut buf = [0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    if unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, PCWSTR(key.as_ptr()), PCWSTR::null(), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len)) }.is_ok() {
        let n = (len as usize / 2).saturating_sub(1).min(buf.len());
        let p = String::from_utf16_lossy(&buf[..n]).trim_matches(['"', '\0']).to_string();
        if std::path::Path::new(&p).exists() {
            return Some(p);
        }
    }
    let path = std::env::var("PATH").unwrap_or_default();
    let sysroot = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let mut dirs: Vec<String> = vec![format!("{sysroot}\\System32"), sysroot.clone()];
    dirs.extend(path.split(';').filter(|d| !d.is_empty()).map(String::from));
    dirs.into_iter().map(|d| std::path::Path::new(&d).join(&name)).find(|p| p.exists()).map(|p| p.to_string_lossy().to_string())
}

/// 被控端的会话代理以 SYSTEM 运行：拿这个会话登录用户的令牌启动（不能让应用以 SYSTEM 身份跑在用户桌面上）
fn launch_as_session_user(q: &str) -> Result<u32, String> {
    use windows_sys::Win32::System::RemoteDesktop::{ProcessIdToSessionId, WTSQueryUserToken};
    use windows_sys::Win32::System::Threading::{CreateProcessAsUserW, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTF_USESHOWWINDOW, STARTUPINFOW};
    let exe = resolve_exe(q).ok_or_else(|| err(&format!("找不到 {q}，请给可执行文件名（如 notepad、chrome）或完整路径")))?;
    unsafe {
        let mut session = 0u32;
        if ProcessIdToSessionId(GetCurrentProcessId(), &mut session) == 0 {
            return Err(err("查不到当前会话"));
        }
        let mut tok: windows_sys::Win32::Foundation::HANDLE = std::ptr::null_mut();
        if WTSQueryUserToken(session, &mut tok) == 0 {
            return Err(err("这个会话里没有登录的用户，没法启动应用"));
        }
        let env = crate::win::env_block(Some(tok), &[]);
        let app = wide(&exe);
        let mut cmd = wide(&crate::win::quote_arg(&exe));
        let dir = std::path::Path::new(&exe).parent().map(|d| wide(&d.to_string_lossy())).unwrap_or_else(|| wide("C:\\"));
        let mut desk = wide("winsta0\\default");
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        si.lpDesktop = desk.as_mut_ptr();
        si.dwFlags = STARTF_USESHOWWINDOW;
        si.wShowWindow = SW_SHOWNOACTIVATE.0 as u16;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = CreateProcessAsUserW(tok, app.as_ptr(), cmd.as_mut_ptr(), std::ptr::null(), std::ptr::null(), 0, CREATE_UNICODE_ENVIRONMENT, env.as_ptr() as *const _, dir.as_ptr(), &si, &mut pi);
        let e = std::io::Error::last_os_error();
        windows_sys::Win32::Foundation::CloseHandle(tok);
        if ok == 0 {
            return Err(err(&format!("启动 {exe} 失败：{e}")));
        }
        let _ = WaitForInputIdle(HANDLE(pi.hProcess), 5000);
        windows_sys::Win32::Foundation::CloseHandle(pi.hThread);
        windows_sys::Win32::Foundation::CloseHandle(pi.hProcess);
        Ok(pi.dwProcessId)
    }
}

fn click_word(right: bool, count: u32) -> &'static str {
    match (right, count) {
        (true, _) => "右键点击",
        (false, 2) => "双击",
        _ => "点击",
    }
}

fn dir_word(d: &str) -> &'static str {
    match d {
        "up" => "上",
        "down" => "下",
        "left" => "左",
        _ => "右",
    }
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "…"
    }
}

fn com_err(e: windows::core::Error) -> String {
    let m = e.message();
    if m.is_empty() {
        format!("{:#x}", e.code().0)
    } else {
        m
    }
}

fn pat<T: Interface>(e: &IUIAutomationElement, id: UIA_PATTERN_ID) -> Result<T, String> {
    unsafe { e.GetCurrentPatternAs::<T>(id) }.map_err(|_| "控件不支持这个动作".to_string())
}

fn value_pattern(e: &IUIAutomationElement) -> Option<IUIAutomationValuePattern> {
    unsafe { e.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) }.ok()
}

/// 单行输入框：标准编辑框没有 ES_MULTILINE；其余要是 Edit 类型，且网页的 aria 属性里没有 multiline=true
fn single_line(e: &IUIAutomationElement, h: Option<HWND>) -> bool {
    if let Some(h) = h.filter(|h| is_edit_class(*h)) {
        return unsafe { GetWindowLongW(h, GWL_STYLE) } as u32 & ES_MULTILINE_STYLE == 0;
    }
    if unsafe { e.CurrentControlType() }.map(|c| c.0).unwrap_or(0) != 50004 {
        return false;
    }
    let aria = unsafe { e.CurrentAriaProperties() }.map(|b| b.to_string().to_lowercase()).unwrap_or_default();
    !aria.contains("multiline=true")
}

/// 让控件获得键盘焦点（UIA SetFocus）；已经有焦点就不动。Chromium 的控件在后台窗口里也能拿到焦点，不会激活窗口
fn focus_element(e: &IUIAutomationElement) -> bool {
    unsafe {
        if e.CurrentHasKeyboardFocus().map(|b| b.as_bool()).unwrap_or(false) {
            return true;
        }
        if e.SetFocus().is_err() {
            return false;
        }
    }
    std::thread::sleep(Duration::from_millis(60));
    true
}

/// 全选控件里的文字（随后输入的字会替换掉它）：TextPattern 选中全文，不行再发 ctrl+a
fn select_all(e: &IUIAutomationElement, target: HWND) -> &'static str {
    unsafe {
        if let Ok(tp) = e.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) {
            if tp.DocumentRange().and_then(|r| r.Select()).is_ok() {
                return "全选原有内容";
            }
        }
    }
    if let Ok(a) = keys::parse("ctrl+a") {
        let _ = keys::send(target, &a);
    }
    "ctrl+a 全选"
}

/// 等投递的键盘消息处理完，读回控件的值给模型核对（密码框、读不到值时不说）
fn settle_value(e: &IUIAutomationElement, text: &str) -> String {
    let p = match value_pattern(e) {
        Some(p) if !unsafe { e.CurrentIsPassword() }.map(|b| b.as_bool()).unwrap_or(false) => p,
        _ => return String::new(),
    };
    let deadline = Instant::now() + Duration::from_millis(200 + 4 * text.chars().count().min(500) as u64);
    let mut last = String::new();
    loop {
        std::thread::sleep(Duration::from_millis(80));
        let v = unsafe { p.CurrentValue() }.map(|b| b.to_string()).unwrap_or_default();
        if v.contains(text.trim_end_matches(['\n', '\r'])) || (Instant::now() > deadline && v == last) || Instant::now() > deadline + Duration::from_millis(600) {
            return format!("，现在的内容「{}」", trunc(&v, 120));
        }
        last = v;
    }
}

fn elem_center(e: &IUIAutomationElement) -> Option<(i32, i32)> {
    let r = unsafe { e.CurrentBoundingRectangle() }.ok()?;
    (r.right > r.left && r.bottom > r.top).then(|| ((r.left + r.right) / 2, (r.top + r.bottom) / 2))
}

/// 按控件点击：Invoke → Toggle → SelectionItem → ExpandCollapse → LegacyIAccessible 默认动作。返回用的方法
fn press_element(e: &IUIAutomationElement) -> Option<&'static str> {
    unsafe {
        // 标准 Win32 按钮（含复选框、单选框）：UIA 的 Invoke 代理会把窗口拉到前台，直接发 BM_CLICK
        if let Some(h) = e.CurrentNativeWindowHandle().ok().filter(|h| !h.is_invalid()) {
            if class_name(h).eq_ignore_ascii_case("button") || class_name(h).to_lowercase().contains(".button.") {
                post(h, BM_CLICK_MSG, 0, 0);
                return Some("按钮消息");
            }
        }
        if let Ok(p) = e.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId) {
            if p.Invoke().is_ok() {
                return Some("Invoke");
            }
        }
        if let Ok(p) = e.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId) {
            if p.Toggle().is_ok() {
                return Some("Toggle");
            }
        }
        if let Ok(p) = e.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId) {
            if p.Select().is_ok() {
                return Some("Select");
            }
        }
        if let Ok(p) = e.GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId) {
            let st = p.CurrentExpandCollapseState().unwrap_or(ExpandCollapseState_Collapsed);
            let r = if st == ExpandCollapseState_Expanded { p.Collapse() } else { p.Expand() };
            if r.is_ok() {
                return Some(if st == ExpandCollapseState_Expanded { "Collapse" } else { "Expand" });
            }
        }
        if let Ok(p) = e.GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(UIA_LegacyIAccessiblePatternId) {
            let has = p.CurrentDefaultAction().map(|s| !s.is_empty()).unwrap_or(false);
            if has && p.DoDefaultAction().is_ok() {
                return Some("默认动作");
            }
        }
    }
    None
}

fn set_element_value(e: &IUIAutomationElement, value: &str) -> Result<&'static str, String> {
    unsafe {
        // 有自己窗口句柄的标准输入框：用编辑框消息整体替换，不抢前台
        if let Some(h) = e.CurrentNativeWindowHandle().ok().filter(|h| !h.is_invalid()) {
            if is_edit_class(h) {
                if GetWindowLongW(h, GWL_STYLE) as u32 & ES_READONLY_STYLE != 0 {
                    return Err(err("这个输入框是只读的"));
                }
                send_timeout(h, EM_SETSEL, WPARAM(0), LPARAM(-1));
                let w = wide(&edit_newlines(h, value));
                send_timeout(h, EM_REPLACESEL, WPARAM(1), LPARAM(w.as_ptr() as isize)).ok_or_else(|| err("输入框没有响应"))?;
                return Ok("编辑框消息");
            }
        }
        if let Some(p) = value_pattern(e) {
            if p.CurrentIsReadOnly().map(|b| b.as_bool()).unwrap_or(false) {
                return Err(err("这个控件是只读的"));
            }
            p.SetValue(&BSTR::from(value)).map_err(|x| err(&format!("设置失败：{}", com_err(x))))?;
            return Ok("ValuePattern");
        }
        if let Ok(p) = e.GetCurrentPatternAs::<IUIAutomationRangeValuePattern>(UIA_RangeValuePatternId) {
            let n: f64 = value.trim().parse().map_err(|_| err("这个控件是数值型（滑块等），value 要是数字"))?;
            p.SetValue(n).map_err(|x| err(&format!("设置失败：{}", com_err(x))))?;
            return Ok("RangeValuePattern");
        }
        if let Ok(p) = e.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId) {
            let want = matches!(value.trim().to_lowercase().as_str(), "true" | "1" | "on" | "yes" | "checked" | "是" | "开");
            let on = p.CurrentToggleState().map(|s| s == ToggleState_On).unwrap_or(false);
            if on != want {
                p.Toggle().map_err(|x| err(&format!("设置失败：{}", com_err(x))))?;
            }
            return Ok("TogglePattern");
        }
    }
    Err(err("这个控件不能直接设置值；可以先 click 它，再 type_text"))
}

// ---------------------------------------------------------------- 控件树

const ES_READONLY_STYLE: u32 = 0x0800;
const ES_MULTILINE_STYLE: u32 = 0x0004;
const BM_CLICK_MSG: u32 = 0x00F5;
const EM_SETSEL: u32 = 0x00B1;
const EM_REPLACESEL: u32 = 0x00C2;

struct Tree<'a> {
    #[allow(dead_code)]
    uia: &'a IUIAutomation,
    req: IUIAutomationCacheRequest,
    walker: IUIAutomationTreeWalker,
    geo: Geo,
    /// 窗口在屏幕上的范围
    win: RECT,
    max: usize,
    deadline: Instant,
    out: String,
    elems: Vec<IUIAutomationElement>,
    labels: Vec<String>,
    /// 每个编号的缩进层级与控件类型（判断网页是否还没加载出控件）
    depths: Vec<usize>,
    roles: Vec<i32>,
    truncated: bool,
}

fn cache_request(uia: &IUIAutomation) -> Result<IUIAutomationCacheRequest, String> {
    unsafe {
        let r = uia.CreateCacheRequest().map_err(|e| err(&format!("UI Automation：{e}")))?;
        for p in [
            UIA_ControlTypePropertyId,
            UIA_NamePropertyId,
            UIA_HelpTextPropertyId,
            UIA_BoundingRectanglePropertyId,
            UIA_IsOffscreenPropertyId,
            UIA_IsEnabledPropertyId,
            UIA_HasKeyboardFocusPropertyId,
            UIA_IsPasswordPropertyId,
            UIA_ValueValuePropertyId,
            UIA_ValueIsReadOnlyPropertyId,
            UIA_IsValuePatternAvailablePropertyId,
            UIA_IsInvokePatternAvailablePropertyId,
            UIA_IsTogglePatternAvailablePropertyId,
            UIA_IsSelectionItemPatternAvailablePropertyId,
            UIA_IsExpandCollapsePatternAvailablePropertyId,
            UIA_IsScrollPatternAvailablePropertyId,
            UIA_IsScrollItemPatternAvailablePropertyId,
            UIA_IsRangeValuePatternAvailablePropertyId,
            UIA_ToggleToggleStatePropertyId,
            UIA_ExpandCollapseExpandCollapseStatePropertyId,
            UIA_SelectionItemIsSelectedPropertyId,
            UIA_RangeValueValuePropertyId,
        ] {
            let _ = r.AddProperty(p);
        }
        Ok(r)
    }
}

/// 读缓存的属性值（布尔 / 整数 / 字符串 / 浮点）
enum Var {
    Bool(bool),
    Int(i32),
    Str(String),
    F64(f64),
    None,
}

fn cached(e: &IUIAutomationElement, id: UIA_PROPERTY_ID) -> Var {
    unsafe {
        let Ok(mut v) = e.GetCachedPropertyValue(id) else { return Var::None };
        let vt = v.Anonymous.Anonymous.vt;
        let r = if vt == VT_BOOL {
            Var::Bool(v.Anonymous.Anonymous.Anonymous.boolVal.as_bool())
        } else if vt == VT_I4 {
            Var::Int(v.Anonymous.Anonymous.Anonymous.lVal)
        } else if vt == VT_BSTR {
            Var::Str((*v.Anonymous.Anonymous.Anonymous.bstrVal).to_string())
        } else if vt == windows::Win32::System::Variant::VT_R8 {
            Var::F64(v.Anonymous.Anonymous.Anonymous.dblVal)
        } else {
            Var::None
        };
        let _ = VariantClear(&mut v as *mut VARIANT);
        r
    }
}

fn cbool(e: &IUIAutomationElement, id: UIA_PROPERTY_ID) -> bool {
    matches!(cached(e, id), Var::Bool(true))
}

fn control_name(id: i32) -> &'static str {
    match id {
        50000 => "Button",
        50001 => "Calendar",
        50002 => "CheckBox",
        50003 => "ComboBox",
        50004 => "Edit",
        50005 => "Hyperlink",
        50006 => "Image",
        50007 => "ListItem",
        50008 => "List",
        50009 => "Menu",
        50010 => "MenuBar",
        50011 => "MenuItem",
        50012 => "ProgressBar",
        50013 => "RadioButton",
        50014 => "ScrollBar",
        50015 => "Slider",
        50016 => "Spinner",
        50017 => "StatusBar",
        50018 => "Tab",
        50019 => "TabItem",
        50020 => "Text",
        50021 => "ToolBar",
        50022 => "ToolTip",
        50023 => "Tree",
        50024 => "TreeItem",
        50025 => "Custom",
        50026 => "Group",
        50027 => "Thumb",
        50028 => "DataGrid",
        50029 => "DataItem",
        50030 => "Document",
        50031 => "SplitButton",
        50032 => "Window",
        50033 => "Pane",
        50034 => "Header",
        50035 => "HeaderItem",
        50036 => "Table",
        50037 => "TitleBar",
        50038 => "Separator",
        50039 => "SemanticZoom",
        50040 => "AppBar",
        _ => "Control",
    }
}

fn quote(s: &str, n: usize) -> String {
    let t = trunc(s, n).replace('\\', "\\\\").replace('"', "\\\"").replace('\r', "").replace('\n', "\\n");
    format!("\"{t}\"")
}

impl Tree<'_> {
    /// Chromium 的辅助功能还没填满：控件很少，或者有空的网页文档
    fn looks_unloaded(&self) -> bool {
        if self.elems.len() < 8 {
            return true;
        }
        (0..self.roles.len()).any(|i| self.roles[i] == 50030 && self.depths.get(i + 1).map(|d| *d <= self.depths[i]).unwrap_or(true))
    }

    fn walk(&mut self, e: &IUIAutomationElement, depth: usize, root: bool) {
        if self.elems.len() >= self.max || Instant::now() > self.deadline {
            self.truncated = true;
            return;
        }
        let shown = self.line(e, depth, root);
        if depth > 60 {
            return;
        }
        let child_depth = if shown { depth + 1 } else { depth };
        let mut c = unsafe { self.walker.GetFirstChildElementBuildCache(e, &self.req) }.ok();
        while let Some(ch) = c {
            if self.elems.len() >= self.max || Instant::now() > self.deadline {
                self.truncated = true;
                return;
            }
            self.walk(&ch, child_depth, false);
            c = unsafe { self.walker.GetNextSiblingElementBuildCache(&ch, &self.req) }.ok();
        }
    }

    /// 写一行；返回是否写了（没写的布局容器，子节点不加缩进）
    fn line(&mut self, e: &IUIAutomationElement, depth: usize, root: bool) -> bool {
        let ct = unsafe { e.CachedControlType() }.map(|c| c.0).unwrap_or(0);
        let role = control_name(ct);
        let name = unsafe { e.CachedName() }.map(|b| b.to_string()).unwrap_or_default();
        let name = if name.trim().is_empty() { unsafe { e.CachedHelpText() }.map(|b| b.to_string()).unwrap_or_default() } else { name };
        let r = unsafe { e.CachedBoundingRectangle() }.unwrap_or_default();
        let offscreen = unsafe { e.CachedIsOffscreen() }.map(|b| b.as_bool()).unwrap_or(false);
        let empty = r.right <= r.left || r.bottom <= r.top;
        // 整个落在窗口外面的（隐藏的、滚出去的）不列出，但子节点照样看
        let outside = !empty && (r.right <= self.win.left || r.left >= self.win.right || r.bottom <= self.win.top || r.top >= self.win.bottom);
        if !root && (outside || (offscreen && empty)) {
            return false;
        }
        let has = |id| cbool(e, id);
        let value_pat = has(UIA_IsValuePatternAvailablePropertyId);
        let password = unsafe { e.CachedIsPassword() }.map(|b| b.as_bool()).unwrap_or(false);
        let value = if value_pat && !password {
            match cached(e, UIA_ValueValuePropertyId) {
                Var::Str(s) if !s.is_empty() && s != name => Some(s),
                _ => None,
            }
        } else if has(UIA_IsRangeValuePatternAvailablePropertyId) {
            match cached(e, UIA_RangeValueValuePropertyId) {
                Var::F64(f) => Some(format!("{f}")),
                _ => None,
            }
        } else {
            None
        };
        let settable = (value_pat && !has(UIA_ValueIsReadOnlyPropertyId)) || has(UIA_IsRangeValuePatternAvailablePropertyId);
        let focused = unsafe { e.CachedHasKeyboardFocus() }.map(|b| b.as_bool()).unwrap_or(false);
        let enabled = unsafe { e.CachedIsEnabled() }.map(|b| b.as_bool()).unwrap_or(true);
        let mut actions: Vec<&str> = Vec::new();
        let mut states: Vec<&str> = Vec::new();
        if has(UIA_IsInvokePatternAvailablePropertyId) {
            actions.push("Invoke");
        }
        if has(UIA_IsTogglePatternAvailablePropertyId) {
            actions.push("Toggle");
            if let Var::Int(s) = cached(e, UIA_ToggleToggleStatePropertyId) {
                states.push(match s {
                    1 => "checked",
                    2 => "mixed",
                    _ => "unchecked",
                });
            }
        }
        if has(UIA_IsSelectionItemPatternAvailablePropertyId) {
            actions.push("Select");
            if has(UIA_SelectionItemIsSelectedPropertyId) {
                states.push("selected");
            }
        }
        if has(UIA_IsExpandCollapsePatternAvailablePropertyId) {
            match cached(e, UIA_ExpandCollapseExpandCollapseStatePropertyId) {
                Var::Int(1) => {
                    actions.push("Collapse");
                    states.push("expanded");
                }
                Var::Int(3) => {}
                _ => {
                    actions.push("Expand");
                    states.push("collapsed");
                }
            }
        }
        if has(UIA_IsScrollPatternAvailablePropertyId) {
            actions.push("Scroll");
        }
        if has(UIA_IsScrollItemPatternAvailablePropertyId) && offscreen {
            actions.push("ScrollIntoView");
        }
        // 没名字、没值、没动作的布局容器和空文字不单独列出
        let layout = matches!(ct, 50033 | 50026 | 50025 | 50020 | 50038 | 50027) || ct == 0;
        if !root && layout && name.trim().is_empty() && value.is_none() && actions.is_empty() && !focused && !settable {
            return false;
        }
        if !root && empty && name.trim().is_empty() {
            return false;
        }
        let i = self.elems.len();
        let mut s = format!("{}[{i}] {role}", "  ".repeat(depth));
        let label = if name.trim().is_empty() { role.to_string() } else { format!("{role} {}", quote(&name, 60)) };
        if !name.trim().is_empty() {
            s += &format!(" {}", quote(&name, 120));
        }
        if let Some(v) = &value {
            s += &format!(" value={}", quote(v, 200));
        }
        if settable {
            s += " settable";
        }
        if focused {
            s += " focused";
        }
        if !enabled {
            s += " disabled";
        }
        if offscreen {
            s += " offscreen";
        }
        for st in &states {
            s += " ";
            s += st;
        }
        if !actions.is_empty() {
            s += &format!(" actions={}", actions.join(","));
        }
        if !empty {
            let (x, y) = self.geo.to_shot(r.left, r.top);
            let w = ((r.right - r.left) as f64 * self.geo.scale).round() as i32;
            let h = ((r.bottom - r.top) as f64 * self.geo.scale).round() as i32;
            s += &format!(" ({x},{y} {w}×{h})");
        }
        s.push('\n');
        self.out += &s;
        self.elems.push(e.clone());
        self.labels.push(label);
        self.depths.push(depth);
        self.roles.push(ct);
        true
    }
}

/// 自己或往上几层里能往这个方向滚的 ScrollPattern
fn scroll_ancestor(uia: &IUIAutomation, e: &IUIAutomationElement, dir: &str) -> Option<IUIAutomationScrollPattern> {
    let walker = unsafe { uia.ControlViewWalker() }.ok()?;
    let mut cur = Some(e.clone());
    for _ in 0..8 {
        let c = cur?;
        if let Ok(p) = unsafe { c.GetCurrentPatternAs::<IUIAutomationScrollPattern>(UIA_ScrollPatternId) } {
            let ok = unsafe {
                if dir == "up" || dir == "down" {
                    p.CurrentVerticallyScrollable()
                } else {
                    p.CurrentHorizontallyScrollable()
                }
            }
            .map(|b| b.as_bool())
            .unwrap_or(false);
            if ok {
                return Some(p);
            }
        }
        cur = unsafe { walker.GetParentElement(&c) }.ok();
    }
    None
}

// ---------------------------------------------------------------- 窗口消息

const SMTO_ABORTIFHUNG_FLAG: SEND_MESSAGE_TIMEOUT_FLAGS = SMTO_ABORTIFHUNG;

pub(crate) fn send_timeout(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> Option<usize> {
    let mut r = 0usize;
    let ok = unsafe { SendMessageTimeoutW(h, msg, w, l, SMTO_ABORTIFHUNG_FLAG, 3000, Some(&mut r)) };
    (ok.0 != 0).then_some(r)
}

pub(crate) fn post(h: HWND, msg: u32, w: usize, l: isize) {
    unsafe {
        let _ = PostMessageW(Some(h), msg, WPARAM(w), LPARAM(l));
    }
}

pub(crate) fn make_lparam(x: i32, y: i32) -> isize {
    (((y as u16 as u32) << 16) | (x as u16 as u32)) as i32 as isize
}

fn is_edit_class(h: HWND) -> bool {
    if h.is_invalid() {
        return false;
    }
    let c = class_name(h).to_lowercase();
    c == "edit" || c.starts_with("richedit") || c.contains(".edit.") || c.contains(".richedit")
}

fn edit_newlines(h: HWND, s: &str) -> String {
    let s = s.replace("\r\n", "\n");
    if class_name(h).eq_ignore_ascii_case("edit") || class_name(h).to_lowercase().contains(".edit.") {
        s.replace('\n', "\r\n")
    } else {
        s.replace('\n', "\r")
    }
}

/// 插入到编辑框光标处（替换选中内容），不抢前台
fn edit_insert(h: HWND, text: &str) -> Result<(), String> {
    if unsafe { GetWindowLongW(h, GWL_STYLE) } as u32 & ES_READONLY_STYLE != 0 {
        return Err(err("焦点输入框是只读的"));
    }
    let w = wide(&edit_newlines(h, text));
    send_timeout(h, EM_REPLACESEL, WPARAM(1), LPARAM(w.as_ptr() as isize)).map(|_| ()).ok_or_else(|| err("输入框没有响应"))
}

fn post_chars(h: HWND, text: &str) {
    for c in text.replace("\r\n", "\n").encode_utf16() {
        let c = if c == '\n' as u16 { '\r' as u16 } else { c };
        post(h, WM_CHAR, c as usize, 1);
    }
}

/// 目标窗口线程里有键盘焦点的控件（不是前台也记着）；不在这个窗口里就用窗口本身
pub(crate) fn focus_hwnd(top: HWND) -> HWND {
    let tid = unsafe { GetWindowThreadProcessId(top, None) };
    let mut gi = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
    if unsafe { GetGUIThreadInfo(tid, &mut gi) }.is_ok() {
        let f = gi.hwndFocus;
        if !f.is_invalid() && (f == top || unsafe { IsChild(top, f).as_bool() }) {
            return f;
        }
    }
    top
}

/// 输入光标在屏幕上的位置（没有就用焦点控件左侧中间）
fn caret_pos(top: HWND, focus: HWND) -> (i32, i32) {
    let tid = unsafe { GetWindowThreadProcessId(top, None) };
    let mut gi = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
    unsafe {
        if GetGUIThreadInfo(tid, &mut gi).is_ok() && !gi.hwndCaret.is_invalid() && (gi.hwndCaret == top || IsChild(top, gi.hwndCaret).as_bool()) {
            let mut p = POINT { x: gi.rcCaret.left, y: gi.rcCaret.bottom };
            let _ = windows::Win32::Graphics::Gdi::ClientToScreen(gi.hwndCaret, &mut p);
            return (p.x, p.y);
        }
    }
    let r = frame_rect(focus);
    (r.left + ((r.right - r.left) / 8).min(40), (r.top + r.bottom) / 2)
}

/// 坐标下最深的子窗口
fn deepest_child(top: HWND, sx: i32, sy: i32) -> HWND {
    let mut h = top;
    for _ in 0..32 {
        let mut p = POINT { x: sx, y: sy };
        unsafe {
            let _ = ScreenToClient(h, &mut p);
        }
        let c = unsafe { ChildWindowFromPointEx(h, p, CWP_SKIPINVISIBLE | CWP_SKIPTRANSPARENT) };
        if c.is_invalid() || c == h {
            break;
        }
        h = c;
    }
    h
}

const MK_LBUTTON: usize = 0x0001;
const MK_RBUTTON: usize = 0x0002;

/// 往坐标下的子窗口投递鼠标消息（客户区坐标）；点在标题栏等非客户区时投非客户区消息
fn post_click(top: HWND, sx: i32, sy: i32, right: bool, count: u32) -> Result<(), String> {
    let fr = frame_rect(top);
    if sx < fr.left || sx >= fr.right || sy < fr.top || sy >= fr.bottom {
        return Err(err("坐标在窗口外面；坐标要按最近一次截图的像素给"));
    }
    let h = deepest_child(top, sx, sy);
    let hit = send_timeout(h, WM_NCHITTEST, WPARAM(0), LPARAM(make_lparam(sx, sy))).unwrap_or(1) as i32;
    // HTCLIENT = 1；其余（标题栏、关闭按钮等）走非客户区消息
    if hit != 1 && hit > 0 && !right {
        let lp = make_lparam(sx, sy);
        for i in 0..count {
            post(h, if i == 1 { WM_NCLBUTTONDBLCLK } else { WM_NCLBUTTONDOWN }, hit as usize, lp);
            post(h, WM_NCLBUTTONUP, hit as usize, lp);
        }
        return Ok(());
    }
    let mut p = POINT { x: sx, y: sy };
    unsafe {
        let _ = ScreenToClient(h, &mut p);
    }
    let lp = make_lparam(p.x, p.y);
    post(h, WM_MOUSEMOVE, 0, lp);
    let (down, up, dbl, mk) = if right { (WM_RBUTTONDOWN, WM_RBUTTONUP, WM_RBUTTONDBLCLK, MK_RBUTTON) } else { (WM_LBUTTONDOWN, WM_LBUTTONUP, WM_LBUTTONDBLCLK, MK_LBUTTON) };
    for i in 0..count {
        post(h, if i == 1 { dbl } else { down }, mk, lp);
        post(h, up, 0, lp);
    }
    Ok(())
}

fn post_wheel(top: HWND, sx: i32, sy: i32, dir: &str, notches: i32) {
    let h = deepest_child(top, sx, sy);
    let lp = make_lparam(sx, sy);
    let (msg, sign) = match dir {
        "up" => (WM_MOUSEWHEEL, 1),
        "down" => (WM_MOUSEWHEEL, -1),
        "right" => (WM_MOUSEHWHEEL, 1),
        _ => (WM_MOUSEHWHEEL, -1),
    };
    for _ in 0..notches {
        let delta = (sign * 120) as i16 as u16 as usize;
        post(h, msg, delta << 16, lp);
    }
}
