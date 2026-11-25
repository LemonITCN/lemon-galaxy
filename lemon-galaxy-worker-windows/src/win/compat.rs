//! Windows 8.1 / 10 才有的接口，运行时动态加载（静态导入的话 Windows 7 上程序根本启动不了）。
//! 老系统兼容版（Windows 7、32 位 Vista / 7、XP）的导入表要求见 docs/research/windows-xp-32bit.md。

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

/// 释放 COM 分配的内存。windows 0.62 把 CoTaskMemFree 指向 Windows 8 才有的 combase.dll，
/// 用它的话 Windows 7 / Vista / XP 上程序根本启动不了；这里从各版本都有的 ole32.dll 导入。
pub unsafe fn co_task_mem_free(p: *const core::ffi::c_void) {
    #[link(name = "ole32")]
    extern "system" {
        fn CoTaskMemFree(pv: *const core::ffi::c_void);
    }
    if !p.is_null() {
        unsafe { CoTaskMemFree(p) }
    }
}

/// 分配 COM 内存（交给调用方用 CoTaskMemFree 释放）。同上，从 ole32.dll 导入而不是 combase.dll。
pub unsafe fn co_task_mem_alloc(n: usize) -> *mut core::ffi::c_void {
    #[link(name = "ole32")]
    extern "system" {
        fn CoTaskMemAlloc(cb: usize) -> *mut core::ffi::c_void;
    }
    unsafe { CoTaskMemAlloc(n) }
}

/// Windows 7 起才有的显示配置（虚拟屏用）与设备卸载接口，运行时加载：静态导入的话 Vista / XP 上程序起不来。
/// 签名与 windows 库的同名函数一致，老系统上返回「不支持」（虚拟屏本来就要 Windows 10 起）。
#[allow(non_snake_case)]
pub mod win7 {
    use windows::Win32::Devices::DeviceAndDriverInstallation::{HDEVINFO, SP_DEVINFO_DATA};
    use windows::Win32::Devices::Display::{DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_TOPOLOGY_ID, QUERY_DISPLAY_CONFIG_FLAGS, SET_DISPLAY_CONFIG_FLAGS};
    use windows::Win32::Foundation::{ERROR_NOT_SUPPORTED, HWND, WIN32_ERROR};

    pub unsafe fn GetDisplayConfigBufferSizes(flags: QUERY_DISPLAY_CONFIG_FLAGS, paths: *mut u32, modes: *mut u32) -> WIN32_ERROR {
        type F = unsafe extern "system" fn(QUERY_DISPLAY_CONFIG_FLAGS, *mut u32, *mut u32) -> WIN32_ERROR;
        match super::proc("user32.dll", b"GetDisplayConfigBufferSizes\0") {
            Some(f) => unsafe { std::mem::transmute::<_, F>(f)(flags, paths, modes) },
            None => ERROR_NOT_SUPPORTED,
        }
    }

    pub unsafe fn QueryDisplayConfig(flags: QUERY_DISPLAY_CONFIG_FLAGS, np: *mut u32, paths: *mut DISPLAYCONFIG_PATH_INFO, nm: *mut u32, modes: *mut DISPLAYCONFIG_MODE_INFO, topology: Option<*mut DISPLAYCONFIG_TOPOLOGY_ID>) -> WIN32_ERROR {
        type F = unsafe extern "system" fn(QUERY_DISPLAY_CONFIG_FLAGS, *mut u32, *mut DISPLAYCONFIG_PATH_INFO, *mut u32, *mut DISPLAYCONFIG_MODE_INFO, *mut DISPLAYCONFIG_TOPOLOGY_ID) -> WIN32_ERROR;
        match super::proc("user32.dll", b"QueryDisplayConfig\0") {
            Some(f) => unsafe { std::mem::transmute::<_, F>(f)(flags, np, paths, nm, modes, topology.unwrap_or(std::ptr::null_mut())) },
            None => ERROR_NOT_SUPPORTED,
        }
    }

    pub unsafe fn SetDisplayConfig(paths: Option<&[DISPLAYCONFIG_PATH_INFO]>, modes: Option<&[DISPLAYCONFIG_MODE_INFO]>, flags: SET_DISPLAY_CONFIG_FLAGS) -> i32 {
        type F = unsafe extern "system" fn(u32, *const DISPLAYCONFIG_PATH_INFO, u32, *const DISPLAYCONFIG_MODE_INFO, SET_DISPLAY_CONFIG_FLAGS) -> i32;
        let Some(f) = super::proc("user32.dll", b"SetDisplayConfig\0") else { return ERROR_NOT_SUPPORTED.0 as i32 };
        let (np, pp) = paths.map_or((0, std::ptr::null()), |p| (p.len() as u32, p.as_ptr()));
        let (nm, mp) = modes.map_or((0, std::ptr::null()), |m| (m.len() as u32, m.as_ptr()));
        unsafe { std::mem::transmute::<_, F>(f)(np, pp, nm, mp, flags) }
    }

    pub unsafe fn DisplayConfigGetDeviceInfo(packet: *mut DISPLAYCONFIG_DEVICE_INFO_HEADER) -> i32 {
        type F = unsafe extern "system" fn(*mut DISPLAYCONFIG_DEVICE_INFO_HEADER) -> i32;
        match super::proc("user32.dll", b"DisplayConfigGetDeviceInfo\0") {
            Some(f) => unsafe { std::mem::transmute::<_, F>(f)(packet) },
            None => ERROR_NOT_SUPPORTED.0 as i32,
        }
    }

    pub unsafe fn DiUninstallDevice(parent: HWND, set: HDEVINFO, data: *const SP_DEVINFO_DATA, flags: u32, reboot: Option<*mut windows::core::BOOL>) -> windows::core::Result<()> {
        type F = unsafe extern "system" fn(HWND, HDEVINFO, *const SP_DEVINFO_DATA, u32, *mut windows::core::BOOL) -> windows::core::BOOL;
        let Some(f) = super::proc("newdev.dll", b"DiUninstallDevice\0") else { return Err(windows::core::Error::from(ERROR_NOT_SUPPORTED)) };
        unsafe { std::mem::transmute::<_, F>(f)(parent, set, data, flags, reboot.unwrap_or(std::ptr::null_mut())) }.ok()
    }
}
