//! press_key：按键名解析与窗口消息投递（不动真实键盘）。
//!
//! 投递的 WM_KEYDOWN 不改变目标线程的键盘状态，应用用 GetKeyState 读 Ctrl / Shift 时看不到，
//! 所以组合键这样做：标准输入框里的常用快捷键直接换成对应的编辑框消息；其余的临时把本线程的输入状态
//! 挂到目标线程上、设好修饰键状态再投递，等目标取走消息后恢复（尽力而为，有的应用不认）。

use super::{class_name, post, send_timeout};
use std::time::Duration;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub(super) struct Combo {
    ctrl: bool,
    shift: bool,
    alt: bool,
    win: bool,
    vk: u16,
    ext: bool,
    /// 主键的写法（小写），用于输入框快捷键映射
    key: String,
}

fn named(k: &str) -> Option<(u16, bool)> {
    let v = |x: VIRTUAL_KEY| x.0;
    Some(match k {
        "return" | "enter" => (v(VK_RETURN), false),
        "tab" => (v(VK_TAB), false),
        "escape" | "esc" => (v(VK_ESCAPE), false),
        "space" | "spacebar" => (v(VK_SPACE), false),
        "backspace" | "back" => (v(VK_BACK), false),
        "delete" | "del" | "forwarddelete" => (v(VK_DELETE), true),
        "insert" | "ins" => (v(VK_INSERT), true),
        "up" | "uparrow" | "arrowup" => (v(VK_UP), true),
        "down" | "downarrow" | "arrowdown" => (v(VK_DOWN), true),
        "left" | "leftarrow" | "arrowleft" => (v(VK_LEFT), true),
        "right" | "rightarrow" | "arrowright" => (v(VK_RIGHT), true),
        "home" => (v(VK_HOME), true),
        "end" => (v(VK_END), true),
        "pageup" | "pgup" | "prior" => (v(VK_PRIOR), true),
        "pagedown" | "pgdn" | "next" => (v(VK_NEXT), true),
        "menu" | "apps" | "contextmenu" => (v(VK_APPS), true),
        _ => {
            let f = k.strip_prefix('f')?.parse::<u16>().ok()?;
            if !(1..=24).contains(&f) {
                return None;
            }
            (v(VK_F1) + f - 1, false)
        }
    })
}

pub(super) fn parse(s: &str) -> Result<Combo, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("key 是空的".into());
    }
    // 单独一个「+」也是合法按键
    let parts: Vec<&str> = if s == "+" { vec!["+"] } else { s.split('+').map(|p| p.trim()).filter(|p| !p.is_empty()).collect() };
    let (main, mods) = parts.split_last().ok_or("key 是空的")?;
    let mut c = Combo { ctrl: false, shift: false, alt: false, win: false, vk: 0, ext: false, key: main.to_lowercase() };
    for m in mods {
        match m.to_lowercase().as_str() {
            "ctrl" | "control" | "cmd" | "command" | "super" => c.ctrl = true,
            "shift" => c.shift = true,
            "alt" | "option" | "opt" => c.alt = true,
            "win" | "windows" | "meta" => c.win = true,
            o => return Err(format!("不认识的修饰键 {o}（可用 ctrl、shift、alt）")),
        }
    }
    let ml = main.to_lowercase();
    if let Some((vk, ext)) = named(&ml) {
        c.vk = vk;
        c.ext = ext;
        return Ok(c);
    }
    let mut chars = main.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => {
            let mut buf = [0u16; 2];
            let u = ch.encode_utf16(&mut buf);
            if u.len() != 1 {
                return Err(format!("按键 {main} 不能用 press_key 发送，请用 type_text"));
            }
            let r = unsafe { VkKeyScanW(u[0]) };
            if r == -1 {
                return Err(format!("当前键盘布局里没有 {main} 这个键，请用 type_text 输入"));
            }
            c.vk = (r as u16) & 0xff;
            if (r as u16 >> 8) & 1 != 0 {
                c.shift = true;
            }
            Ok(c)
        }
        _ => Err(format!("不认识的按键 {main}（例：Return、Tab、Escape、Backspace、Delete、Up、PageDown、F5、a、ctrl+s）")),
    }
}

fn lparam(vk: u16, ext: bool, up: bool, alt: bool) -> isize {
    let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } & 0xff;
    let mut l: u32 = 1 | (scan << 16);
    if ext {
        l |= 1 << 24;
    }
    if alt {
        l |= 1 << 29;
    }
    if up {
        l |= (1 << 30) | (1 << 31);
    }
    l as i32 as isize
}

fn is_edit(h: HWND) -> bool {
    let c = class_name(h).to_lowercase();
    c == "edit" || c.starts_with("richedit") || c.contains(".edit.") || c.contains(".richedit")
}

/// 发送按键，返回说明
pub(super) fn send(focus: HWND, c: &Combo) -> Result<String, String> {
    if c.win {
        return Err("Win 键组合没法在后台发送，请改用全屏 keyboard 工具（会接管前台）".into());
    }
    // 标准输入框里的常用快捷键：换成编辑框消息，最可靠
    if c.ctrl && !c.alt && is_edit(focus) {
        let m = match (c.key.as_str(), c.shift) {
            ("a", false) => Some((0x00B1u32, 0usize, -1isize)), // EM_SETSEL 全选
            ("c", false) => Some((WM_COPY, 0, 0)),
            ("x", false) => Some((WM_CUT, 0, 0)),
            ("v", false) => Some((WM_PASTE, 0, 0)),
            ("z", false) => Some((0x00C7, 0, 0)), // EM_UNDO
            _ => None,
        };
        if let Some((msg, w, l)) = m {
            send_timeout(focus, msg, WPARAM(w), LPARAM(l)).ok_or("输入框没有响应")?;
            return Ok("编辑框消息".into());
        }
    }
    if !c.ctrl && !c.shift {
        if c.alt {
            // Alt 组合用系统按键消息，Alt 状态编码在 lParam 里，菜单加速键能认
            post(focus, WM_SYSKEYDOWN, c.vk as usize, lparam(c.vk, c.ext, false, true));
            post(focus, WM_SYSKEYUP, c.vk as usize, lparam(c.vk, c.ext, true, true));
        } else {
            post(focus, WM_KEYDOWN, c.vk as usize, lparam(c.vk, c.ext, false, false));
            post(focus, WM_KEYUP, c.vk as usize, lparam(c.vk, c.ext, true, false));
        }
        return Ok("窗口消息".into());
    }
    // Ctrl / Shift 组合：临时共享目标线程的键盘状态
    let target = unsafe { GetWindowThreadProcessId(focus, None) };
    let me = unsafe { GetCurrentThreadId() };
    let attached = target != 0 && target != me && unsafe { AttachThreadInput(me, target, true).as_bool() };
    let mut saved = [0u8; 256];
    if attached {
        unsafe {
            let _ = GetKeyboardState(&mut saved);
            let mut ks = saved;
            if c.ctrl {
                ks[VK_CONTROL.0 as usize] |= 0x80;
                ks[VK_LCONTROL.0 as usize] |= 0x80;
            }
            if c.shift {
                ks[VK_SHIFT.0 as usize] |= 0x80;
                ks[VK_LSHIFT.0 as usize] |= 0x80;
            }
            if c.alt {
                ks[VK_MENU.0 as usize] |= 0x80;
                ks[VK_LMENU.0 as usize] |= 0x80;
            }
            let _ = SetKeyboardState(&ks);
        }
    }
    let (down, up) = if c.alt { (WM_SYSKEYDOWN, WM_SYSKEYUP) } else { (WM_KEYDOWN, WM_KEYUP) };
    post(focus, down, c.vk as usize, lparam(c.vk, c.ext, false, c.alt));
    post(focus, up, c.vk as usize, lparam(c.vk, c.ext, true, c.alt));
    if attached {
        // 等目标线程处理到这两条消息再恢复（先确认它在收消息，再留一点时间）
        let _ = send_timeout(focus, WM_NULL, WPARAM(0), LPARAM(0));
        std::thread::sleep(Duration::from_millis(150));
        unsafe {
            let _ = SetKeyboardState(&saved);
            let _ = AttachThreadInput(me, target, false);
        }
        Ok("窗口消息 + 临时修饰键状态；有的应用不认，没效果时可以改用全屏 keyboard 工具".into())
    } else {
        Ok("窗口消息（没能设置修饰键状态，组合键可能不生效；没效果时改用全屏 keyboard 工具）".into())
    }
}
