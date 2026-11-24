//! Windows XP 兼容版专用（构建时加 `--cfg lemon_galaxy_xp`，见 scripts/build-windows-legacy.sh）。
//!
//! XP 版靠 YY-Thunks 补上 Vista 起才有的系统接口：链接时它的目标文件提供 `__imp__函数名@N`，
//! 链接器就不再从系统 DLL 导入这些函数，运行时有真的用真的，没有就走它的替代实现。
//! YY-Thunks 没补、而依赖库又静态导入的，在这里用同样的办法补上；CI 用 scripts/check-legacy-imports.py
//! 核对最终导入表里每个函数 XP 都有。

use std::ffi::c_void;
use std::sync::OnceLock;

#[repr(C)]
struct WsaBuf {
    len: u32,
    buf: *mut u8,
}

#[repr(C)]
struct WsaMsg {
    name: *mut c_void,
    namelen: i32,
    buffers: *mut WsaBuf,
    count: u32,
    control: WsaBuf,
    flags: u32,
}

type WsaSendMsgFn = unsafe extern "system" fn(usize, *const WsaMsg, u32, *mut u32, *mut c_void, *const c_void) -> i32;

#[link(name = "ws2_32")]
extern "system" {
    fn WSASendTo(s: usize, buffers: *const WsaBuf, count: u32, sent: *mut u32, flags: u32, to: *const c_void, tolen: i32, ov: *mut c_void, cr: *const c_void) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleA(name: *const u8) -> *mut c_void;
    fn LoadLibraryA(name: *const u8) -> *mut c_void;
    fn GetProcAddress(m: *mut c_void, name: *const u8) -> *mut c_void;
}

/// WSASendMsg 是 Vista 起才有的（quinn-udp 静态导入）。有就调真的；XP 上退成不带控制信息的 WSASendTo
/// （LemonGalaxy 的 QUIC 走自己的 UDP 套接字，平时用不到 quinn-udp 的发送）。
unsafe extern "system" fn wsa_send_msg(s: usize, msg: *const WsaMsg, flags: u32, sent: *mut u32, ov: *mut c_void, cr: *const c_void) -> i32 {
    static REAL: OnceLock<usize> = OnceLock::new();
    let real = *REAL.get_or_init(|| unsafe {
        let mut m = GetModuleHandleA(c"ws2_32.dll".as_ptr() as _);
        if m.is_null() {
            m = LoadLibraryA(c"ws2_32.dll".as_ptr() as _);
        }
        if m.is_null() {
            0
        } else {
            GetProcAddress(m, c"WSASendMsg".as_ptr() as _) as usize
        }
    });
    unsafe {
        if real != 0 {
            let f: WsaSendMsgFn = std::mem::transmute(real);
            return f(s, msg, flags, sent, ov, cr);
        }
        let m = &*msg;
        WSASendTo(s, m.buffers, m.count, sent, flags, m.name, m.namelen, ov, cr)
    }
}

/// 32 位的导入符号带 stdcall 修饰（参数 6 个、共 24 字节）；开头的 \x01 让编译器不再加下划线前缀。
#[cfg(target_arch = "x86")]
#[used]
#[export_name = "\x01__imp__WSASendMsg@24"]
static IMP_WSASENDMSG: WsaSendMsgFn = wsa_send_msg;
