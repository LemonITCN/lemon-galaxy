//! Windows 8.1 / 10 才有的接口，运行时动态加载（静态导入的话 Windows 7 上程序根本启动不了）。

use windows::core::PCSTR;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

fn proc(dll: &str, name: &[u8]) -> Option<unsafe extern "system" fn() -> isize> {
    let w: Vec<u16> = dll.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let m = LoadLibraryW(windows::core::PCWSTR(w.as_ptr())).ok()?;
        GetProcAddress(m, PCSTR(name.as_ptr()))
    }
}

/// 进程按每屏 DPI 感知：Win10 1703 起用 PER_MONITOR_AWARE_V2，8.1 用 shcore，更老退回 SetProcessDPIAware。
pub fn set_dpi_aware() {
    type CtxFn = unsafe extern "system" fn(isize) -> i32;
    type ShcoreFn = unsafe extern "system" fn(i32) -> i32;
    unsafe {
        if let Some(f) = proc("user32.dll", b"SetProcessDpiAwarenessContext\0") {
            let f: CtxFn = std::mem::transmute(f);
            // DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 = -4
            if f(-4) != 0 {
                return;
            }
        }
        if let Some(f) = proc("shcore.dll", b"SetProcessDpiAwareness\0") {
            let f: ShcoreFn = std::mem::transmute(f);
            // PROCESS_PER_MONITOR_DPI_AWARE = 2
            if f(2) == 0 {
                return;
            }
        }
        let _ = windows::Win32::UI::WindowsAndMessaging::SetProcessDPIAware();
    }
}

/// 屏幕缩放比例（Win8.1 起），老系统返回 1。
pub fn monitor_scale(hm: HMONITOR) -> f32 {
    type DpiFn = unsafe extern "system" fn(HMONITOR, i32, *mut u32, *mut u32) -> i32;
    unsafe {
        let Some(f) = proc("shcore.dll", b"GetDpiForMonitor\0") else { return 1.0 };
        let f: DpiFn = std::mem::transmute(f);
        let (mut x, mut y) = (96u32, 96u32);
        // MDT_EFFECTIVE_DPI = 0
        if f(hm, 0, &mut x, &mut y) == 0 {
            x as f32 / 96.0
        } else {
            1.0
        }
    }
}
