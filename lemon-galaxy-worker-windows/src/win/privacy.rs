//! 隐私屏：本地每块屏幕盖一层黑窗（截屏时被排除，远端照常看到桌面，鼠标点击穿透到下面的窗口），
//! 并用低级键鼠钩子挡掉本地物理输入（远端注入的输入带 INJECTED 标记，放行）。
//!
//! 黑窗怎么不进截屏：
//! - Windows 10 2004 起：WDA_EXCLUDEFROMCAPTURE。
//! - XP / Server 2003、Vista / 7：没有桌面合成时，分层窗口不在屏幕表面上（GDI 另外叠上去），GDI 截屏不带
//!   CAPTUREBLT 就截不到它，截到的是下面的实时桌面（XP 虚拟机实测）。开着时 GDI 截屏去掉 CAPTUREBLT
//!   （代价：别的程序的分层窗口远端也看不到）；Vista / 7 开着 Aero 时先关掉桌面合成，关隐私屏时再打开。
//! - Windows 8 到 10 1909：桌面合成关不掉、截屏走桌面复制，黑窗一定会被截进去，不支持。

use super::capture::{attach_input_desktop, monitors};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetStockObject, HBRUSH, BLACK_BRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, KillTimer, PostThreadMessageW, RegisterClassW, SetTimer, SetWindowDisplayAffinity, SetWindowPos, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, HHOOK, HWND_TOPMOST, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WDA_EXCLUDEFROMCAPTURE, WH_KEYBOARD_LL, WH_MOUSE_LL,
    WINDOW_EX_STYLE, WM_QUIT, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP, WS_VISIBLE,
};

/// 老系统的隐私屏开着：GDI 截屏不能带 CAPTUREBLT（capture.rs）
static LEGACY_ON: AtomicBool = AtomicBool::new(false);

pub fn legacy_active() -> bool {
    LEGACY_ON.load(Ordering::Relaxed)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// WDA_EXCLUDEFROMCAPTURE
    Affinity,
    /// 分层黑窗 + GDI 截屏不带 CAPTUREBLT；dwm 表示要先关 Vista / 7 的桌面合成
    Layered { dwm: bool },
}

const TOO_OLD: &str = "这台电脑的系统不支持隐私屏（Windows 8 到 Windows 10 1909 没办法让黑窗不进截屏，Windows 10 请升级到 2004 或更新）";

fn mode() -> Result<Mode, String> {
    let (major, minor, build) = galaxy_host::win::os_version();
    if major >= 10 && build >= 19041 {
        Ok(Mode::Affinity)
    } else if major == 5 {
        Ok(Mode::Layered { dwm: false })
    } else if major == 6 && minor <= 1 {
        Ok(Mode::Layered { dwm: true })
    } else {
        Err(TOO_OLD.into())
    }
}

type DwmIsEnabledFn = unsafe extern "system" fn(*mut i32) -> i32;
type DwmEnableFn = unsafe extern "system" fn(u32) -> i32;

/// dwmapi.dll 是 Vista 起才有的，运行时加载
fn dwm_proc(name: &[u8]) -> Option<unsafe extern "system" fn() -> isize> {
    use windows::core::PCSTR;
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    unsafe {
        let m = LoadLibraryW(windows::core::w!("dwmapi.dll")).ok()?;
        GetProcAddress(m, PCSTR(name.as_ptr()))
    }
}

fn dwm_composition() -> bool {
    let Some(f) = dwm_proc(b"DwmIsCompositionEnabled\0") else { return false };
    let mut on = 0i32;
    unsafe { std::mem::transmute::<_, DwmIsEnabledFn>(f)(&mut on) == 0 && on != 0 }
}

fn dwm_enable(on: bool) -> bool {
    let Some(f) = dwm_proc(b"DwmEnableComposition\0") else { return false };
    // DWM_EC_DISABLECOMPOSITION = 0，DWM_EC_ENABLECOMPOSITION = 1
    unsafe { std::mem::transmute::<_, DwmEnableFn>(f)(on as u32) == 0 }
}

const LLKHF_INJECTED: u32 = 0x10;
const LLMHF_INJECTED: u32 = 0x01;

unsafe extern "system" fn kb_proc(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 {
        let k = &*(l.0 as *const KBDLLHOOKSTRUCT);
        if k.flags.0 & LLKHF_INJECTED == 0 {
            return LRESULT(1);
        }
    }
    CallNextHookEx(None, code, w, l)
}

unsafe extern "system" fn mouse_proc(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 {
        let m = &*(l.0 as *const MSLLHOOKSTRUCT);
        if m.flags & LLMHF_INJECTED == 0 {
            return LRESULT(1);
        }
    }
    CallNextHookEx(None, code, w, l)
}

unsafe extern "system" fn wnd_proc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    DefWindowProcW(h, msg, w, l)
}

/// 正在生效的隐私屏（所在线程的 id，关时给它发 WM_QUIT）。
pub struct Privacy {
    thread: u32,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Privacy {
    /// 开启。不支持（系统太老、建不了窗口）时返回 Err。
    pub fn start() -> Result<Self, String> {
        // 按版本号选做法：XP / Vista 兼容版里 YY-Thunks 会让 SetWindowDisplayAffinity「假装成功」，不能靠它的返回值
        let mode = mode()?;
        let (tx, rx) = mpsc::channel::<Result<u32, String>>();
        let join = std::thread::spawn(move || unsafe { run(tx, mode) });
        match rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Ok(thread)) => Ok(Self { thread, join: Some(join) }),
            Ok(Err(e)) => {
                let _ = join.join();
                Err(e)
            }
            Err(_) => Err("隐私屏启动超时".into()),
        }
    }
}

impl Drop for Privacy {
    fn drop(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

unsafe fn run(tx: mpsc::Sender<Result<u32, String>>, mode: Mode) {
    attach_input_desktop();
    // Vista / 7 开着桌面合成时分层窗口会被截进去：先关掉（只在本进程开着隐私屏期间有效）
    let mut dwm_off = false;
    if mode == (Mode::Layered { dwm: true }) && dwm_composition() {
        if !dwm_enable(false) || dwm_composition() {
            let _ = tx.send(Err("关不掉 Aero 桌面合成，隐私屏开不了".into()));
            return;
        }
        dwm_off = true;
    }
    // 黑窗出现之前就让截屏不带 CAPTUREBLT，一帧黑的都不进画面
    if mode != Mode::Affinity {
        LEGACY_ON.store(true, Ordering::Relaxed);
    }
    let inst: HINSTANCE = GetModuleHandleW(PCWSTR::null()).map(|m| m.into()).unwrap_or_default();
    let class = w!("LemonGalaxyPrivacy");
    let wc = WNDCLASSW { lpfnWndProc: Some(wnd_proc), hInstance: inst, lpszClassName: class, hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0), ..Default::default() };
    RegisterClassW(&wc);
    let mut wins: Vec<HWND> = Vec::new();
    for m in monitors() {
        let ex = WINDOW_EX_STYLE(WS_EX_TOPMOST.0 | WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0 | WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0);
        let Ok(h) = CreateWindowExW(ex, class, w!(""), WS_POPUP | WS_VISIBLE, m.rect.left, m.rect.top, m.width() as i32, m.height() as i32, None, None, Some(inst), None) else { continue };
        // 分层窗口要设一次不透明度才会显示
        let _ = windows::Win32::UI::WindowsAndMessaging::SetLayeredWindowAttributes(h, windows::Win32::Foundation::COLORREF(0), 255, windows::Win32::UI::WindowsAndMessaging::LWA_ALPHA);
        if mode == Mode::Affinity && SetWindowDisplayAffinity(h, WDA_EXCLUDEFROMCAPTURE).is_err() {
            for w in wins.iter().chain(std::iter::once(&h)) {
                let _ = DestroyWindow(*w);
            }
            let _ = tx.send(Err(TOO_OLD.into()));
            return;
        }
        wins.push(h);
    }
    if wins.is_empty() {
        LEGACY_ON.store(false, Ordering::Relaxed);
        if dwm_off {
            dwm_enable(true);
        }
        let _ = tx.send(Err("没有可用的屏幕".into()));
        return;
    }
    let kb: Option<HHOOK> = SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_proc), Some(inst), 0).ok();
    let ms: Option<HHOOK> = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), Some(inst), 0).ok();
    let _ = tx.send(Ok(GetCurrentThreadId()));
    // 每秒把黑窗顶回最上层（别的置顶窗口可能盖过来）
    let timer = SetTimer(None, 0, 1000, None);
    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        if msg.message == WM_TIMER {
            for w in &wins {
                let _ = SetWindowPos(*w, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
            continue;
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    let _ = KillTimer(None, timer);
    if let Some(h) = kb {
        let _ = UnhookWindowsHookEx(h);
    }
    if let Some(h) = ms {
        let _ = UnhookWindowsHookEx(h);
    }
    for w in wins {
        let _ = DestroyWindow(w);
    }
    // 黑窗没了再恢复 CAPTUREBLT 与桌面合成
    LEGACY_ON.store(false, Ordering::Relaxed);
    if dwm_off {
        dwm_enable(true);
    }
}
