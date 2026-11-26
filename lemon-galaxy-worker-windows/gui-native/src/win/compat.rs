//! XP 上没有、Vista 起才有的接口：一律运行时 `GetProcAddress`（静态导入的话 XP 上程序根本起不来，
//! CI 用 scripts/check-legacy-imports.py 逐个核对导入表）。另有系统版本、WOW64 判断。

use windows::core::PCSTR;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::SystemInformation::{GetVersionExW, OSVERSIONINFOW};
use windows::Win32::System::Threading::GetCurrentProcess;

fn proc(dll: &str, name: &[u8]) -> Option<unsafe extern "system" fn() -> isize> {
    let w = super::wide(dll);
    unsafe {
        let m = LoadLibraryW(super::pcw(&w)).ok()?;
        GetProcAddress(m, PCSTR(name.as_ptr()))
    }
}

/// 进程按系统 DPI 绘制。清单里已经声明了（Vista 起认）；这里再调一次 SetProcessDPIAware（Vista 起才有），
/// 清单没生效时也不会被系统放大成模糊的位图。XP 本来就不缩放。
pub fn set_dpi_aware() {
    type F = unsafe extern "system" fn() -> i32;
    if let Some(f) = proc("user32.dll", b"SetProcessDPIAware\0") {
        unsafe {
            let f: F = std::mem::transmute(f);
            f();
        }
    }
}

/// (主版本, 次版本)：XP 5.1、2003 5.2、Vista 6.0、7 6.1。8.1 起没有清单声明时报 6.2，这里只用来区分 XP。
pub fn os_version() -> (u32, u32) {
    let mut v = OSVERSIONINFOW { dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32, ..Default::default() };
    unsafe {
        if GetVersionExW(&mut v).is_ok() {
            (v.dwMajorVersion, v.dwMinorVersion)
        } else {
            (5, 1)
        }
    }
}

/// XP / 2003：没有 UAC
pub fn legacy() -> bool {
    os_version().0 < 6
}

/// 32 位程序跑在 64 位 Windows 上（IsWow64Process 是 XP SP2 起才有的，运行时找）
pub fn is_wow64() -> bool {
    type F = unsafe extern "system" fn(HANDLE, *mut i32) -> i32;
    if cfg!(target_pointer_width = "64") {
        return false;
    }
    let Some(f) = proc("kernel32.dll", b"IsWow64Process\0") else { return false };
    unsafe {
        let f: F = std::mem::transmute(f);
        let mut r = 0i32;
        f(GetCurrentProcess(), &mut r) != 0 && r != 0
    }
}

// 32 位老系统版要链 YY-Thunks，它的替代实现里引用了 CoTaskMemAlloc / CoTaskMemFree。windows 0.62 把这两个函数指向
// Windows 8 才有的 combase.dll，链接器要是拿那个去满足引用，XP / Vista 上程序根本起不来（CI 导入表核对会拦下）。
// 这里声明从各版本都有的 ole32.dll 导入并留住引用，链接器就用系统的 ole32.lib（被控端同样做法，见 src/win/compat.rs）。
#[link(name = "ole32")]
extern "system" {
    fn CoTaskMemAlloc(cb: usize) -> *mut core::ffi::c_void;
    fn CoTaskMemFree(pv: *const core::ffi::c_void);
}

#[used]
static KEEP_OLE32_COTASKMEM: (unsafe extern "system" fn(usize) -> *mut core::ffi::c_void, unsafe extern "system" fn(*const core::ffi::c_void)) = (CoTaskMemAlloc, CoTaskMemFree);
