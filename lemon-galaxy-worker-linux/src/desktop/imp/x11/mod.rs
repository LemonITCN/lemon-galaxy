//! X11 后端（含登录界面与 Xvfb 虚拟桌面）。纯 Rust 的 x11rb，按 DISPLAY / XAUTHORITY 环境变量连接。
//!
//! - 截屏：MIT-SHM（优先 memfd 传给服务器，其次 SysV 共享内存，都不行时普通 GetImage），按 RandR 显示器切块；
//!   Damage 扩展说有变化才截，另外每秒兜底截一次比较（合成器绘制偶尔不报 Damage）。
//! - 光标：XFixes 取图像（换光标时才取），位置用 QueryPointer。
//! - 键鼠：XTEST。剪贴板：clipboard.rs。

mod capture;
mod clipboard;
mod input;

pub use input::keysym_for;

use super::backend::{assign_ids, Backend, Capturer, Controller, Monitor};
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

pub fn connect() -> Result<(RustConnection, usize), String> {
    x11rb::connect(None).map_err(|e| format!("连不上 X 服务器（DISPLAY={}）：{e}", std::env::var("DISPLAY").unwrap_or_default()))
}

pub struct X11Backend {
    is_virtual: bool,
}

impl X11Backend {
    /// 连一次确认能用。
    pub fn new(is_virtual: bool) -> Result<Self, String> {
        let (conn, screen) = connect()?;
        let s = &conn.setup().roots[screen];
        let depth_ok = conn.setup().pixmap_formats.iter().any(|f| f.depth == s.root_depth && f.bits_per_pixel == 32);
        if !depth_ok {
            return Err(format!("X 服务器色深 {} 不是 32 位像素，暂不支持", s.root_depth));
        }
        Ok(Self { is_virtual })
    }
}

/// 当前的显示器（桌面坐标）。没有 RandR 1.5 时整个根窗口算一块。
pub fn monitors(conn: &RustConnection, screen: usize, is_virtual: bool) -> Vec<Monitor> {
    let s = &conn.setup().roots[screen];
    let root = s.root;
    let whole = || vec![Monitor { id: 0, name: if is_virtual { "虚拟桌面".into() } else { "屏幕".into() }, x: 0, y: 0, width: s.width_in_pixels as u32, height: s.height_in_pixels as u32, primary: true, scale: 1.0, is_virtual }];
    let has_randr = conn.randr_query_version(1, 5).ok().and_then(|c| c.reply().ok()).map(|r| (r.major_version, r.minor_version) >= (1, 5)).unwrap_or(false);
    if !has_randr {
        return whole();
    }
    let Some(reply) = conn.randr_get_monitors(root, true).ok().and_then(|c| c.reply().ok()) else { return whole() };
    let mut v = vec![];
    for m in reply.monitors {
        if m.width == 0 || m.height == 0 {
            continue;
        }
        let name = conn.get_atom_name(m.name).ok().and_then(|c| c.reply().ok()).map(|r| String::from_utf8_lossy(&r.name).to_string()).unwrap_or_default();
        // 屏幕外的部分截不到：裁到根窗口范围内
        let (x, y) = (m.x.max(0) as i32, m.y.max(0) as i32);
        let w = (m.width as i32).min(s.width_in_pixels as i32 - x);
        let h = (m.height as i32).min(s.height_in_pixels as i32 - y);
        if w <= 0 || h <= 0 {
            continue;
        }
        let name = if is_virtual { "虚拟桌面".to_string() } else if name.is_empty() { "屏幕".to_string() } else { name };
        v.push(Monitor { id: 0, name, x, y, width: w as u32, height: h as u32, primary: m.primary, scale: 1.0, is_virtual });
    }
    if v.is_empty() {
        return whole();
    }
    assign_ids(v)
}

impl Backend for X11Backend {
    fn name(&self) -> &'static str {
        "x11"
    }

    fn monitors(&self) -> Vec<Monitor> {
        match connect() {
            Ok((c, s)) => monitors(&c, s, self.is_virtual),
            Err(_) => vec![],
        }
    }

    fn capturer(&self, m: &Monitor) -> Result<Box<dyn Capturer>, String> {
        capture::X11Capturer::new(m.clone(), self.is_virtual).map(|c| Box::new(c) as _)
    }

    fn controller(&self) -> Result<Box<dyn Controller>, String> {
        input::X11Controller::new().map(|c| Box::new(c) as _)
    }
}
