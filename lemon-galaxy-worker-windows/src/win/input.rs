//! 键鼠注入（SendInput）。坐标是某块屏幕的物理像素，换算到虚拟桌面的绝对坐标。
//! 键盘按 USB HID 用法码换成扫描码发送，与主控端键盘布局无关。

use super::capture::{attach_input_desktop, find_monitor, Monitor};
use galaxy_proto::peer::{InputEvent, MouseButton, SpecialCombo, TouchPhase};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN,
    MOUSEEVENTF_XUP, MOUSEINPUT, MOUSE_EVENT_FLAGS, VIRTUAL_KEY, VK_LWIN, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK, VK_PAUSE, VK_VOLUME_DOWN, VK_VOLUME_MUTE, VK_VOLUME_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN};

/// HID 用法码（键盘页）→ (扫描码, 是否扩展键)。
fn hid_to_scan(u: u32) -> Option<(u16, bool)> {
    let s = |c: u16| Some((c, false));
    let e = |c: u16| Some((c, true));
    match u {
        0x04 => s(0x1E),
        0x05 => s(0x30),
        0x06 => s(0x2E),
        0x07 => s(0x20),
        0x08 => s(0x12),
        0x09 => s(0x21),
        0x0A => s(0x22),
        0x0B => s(0x23),
        0x0C => s(0x17),
        0x0D => s(0x24),
        0x0E => s(0x25),
        0x0F => s(0x26),
        0x10 => s(0x32),
        0x11 => s(0x31),
        0x12 => s(0x18),
        0x13 => s(0x19),
        0x14 => s(0x10),
        0x15 => s(0x13),
        0x16 => s(0x1F),
        0x17 => s(0x14),
        0x18 => s(0x16),
        0x19 => s(0x2F),
        0x1A => s(0x11),
        0x1B => s(0x2D),
        0x1C => s(0x15),
        0x1D => s(0x2C),
        0x1E..=0x26 => s((u - 0x1E + 0x02) as u16),
        0x27 => s(0x0B),
        0x28 => s(0x1C),
        0x29 => s(0x01),
        0x2A => s(0x0E),
        0x2B => s(0x0F),
        0x2C => s(0x39),
        0x2D => s(0x0C),
        0x2E => s(0x0D),
        0x2F => s(0x1A),
        0x30 => s(0x1B),
        0x31 | 0x32 => s(0x2B),
        0x33 => s(0x27),
        0x34 => s(0x28),
        0x35 => s(0x29),
        0x36 => s(0x33),
        0x37 => s(0x34),
        0x38 => s(0x35),
        0x39 => s(0x3A),
        0x3A..=0x43 => s((u - 0x3A + 0x3B) as u16),
        0x44 => s(0x57),
        0x45 => s(0x58),
        0x46 => e(0x37),
        0x47 => s(0x46),
        0x49 => e(0x52),
        0x4A => e(0x47),
        0x4B => e(0x49),
        0x4C => e(0x53),
        0x4D => e(0x4F),
        0x4E => e(0x51),
        0x4F => e(0x4D),
        0x50 => e(0x4B),
        0x51 => e(0x50),
        0x52 => e(0x48),
        0x53 => s(0x45),
        0x54 => e(0x35),
        0x55 => s(0x37),
        0x56 => s(0x4A),
        0x57 => s(0x4E),
        0x58 => e(0x1C),
        0x59 => s(0x4F),
        0x5A => s(0x50),
        0x5B => s(0x51),
        0x5C => s(0x4B),
        0x5D => s(0x4C),
        0x5E => s(0x4D),
        0x5F => s(0x47),
        0x60 => s(0x48),
        0x61 => s(0x49),
        0x62 => s(0x52),
        0x63 => s(0x53),
        0x64 => s(0x56),
        0x65 => e(0x5D),
        0x67 => s(0x59),
        0x68..=0x72 => s((u - 0x68 + 0x64) as u16),
        0x73 => s(0x76),
        0x87 => s(0x73),
        0x88 => s(0x70),
        0x89 => s(0x7D),
        0x8A => s(0x79),
        0x8B => s(0x7B),
        0x90 => s(0x72),
        0x91 => s(0x71),
        0xE0 => s(0x1D),
        0xE1 => s(0x2A),
        0xE2 => s(0x38),
        0xE3 => e(0x5B),
        0xE4 => e(0x1D),
        0xE5 => s(0x36),
        0xE6 => e(0x38),
        0xE7 => e(0x5C),
        _ => None,
    }
}

/// 没有扫描码、只能按虚拟键发送的键。
fn hid_to_vk(u: u32) -> Option<VIRTUAL_KEY> {
    match u {
        0x48 => Some(VK_PAUSE),
        0x7F => Some(VK_VOLUME_MUTE),
        0x80 => Some(VK_VOLUME_UP),
        0x81 => Some(VK_VOLUME_DOWN),
        0xE8 => Some(VK_MEDIA_PLAY_PAUSE),
        0xEA => Some(VK_MEDIA_PREV_TRACK),
        0xEB => Some(VK_MEDIA_NEXT_TRACK),
        _ => None,
    }
}

fn key_input(vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } } }
}

fn mouse_input(dx: i32, dy: i32, data: i32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, mouseData: data as u32, dwFlags: flags, time: 0, dwExtraInfo: 0 } } }
}

fn send(inputs: &[INPUT]) {
    if inputs.is_empty() {
        return;
    }
    let n = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if (n as usize) < inputs.len() {
        // 输入桌面可能刚切换（锁屏 / UAC），切过去再试一次
        if attach_input_desktop().is_some() {
            unsafe {
                SendInput(inputs, std::mem::size_of::<INPUT>() as i32);
            }
        }
    }
}

fn scan_key(scan: u16, ext: bool, down: bool) -> INPUT {
    let mut f = KEYEVENTF_SCANCODE;
    if ext {
        f |= KEYEVENTF_EXTENDEDKEY;
    }
    if !down {
        f |= KEYEVENTF_KEYUP;
    }
    key_input(VIRTUAL_KEY(0), scan, f)
}

fn vk_key(vk: VIRTUAL_KEY, down: bool) -> INPUT {
    key_input(vk, 0, if down { KEYBD_EVENT_FLAGS(0) } else { KEYEVENTF_KEYUP })
}

pub struct Injector {
    mon: Option<Monitor>,
    mon_id: u32,
    last: (i32, i32),
    touch_down: bool,
}

impl Injector {
    pub fn new() -> Self {
        Self { mon: None, mon_id: u32::MAX, last: (0, 0), touch_down: false }
    }

    fn abs(&mut self, display: u32, x: i32, y: i32) -> (i32, i32) {
        if self.mon_id != display || self.mon.is_none() {
            self.mon = find_monitor(display);
            self.mon_id = display;
        }
        let (ox, oy) = self.mon.as_ref().map(|m| (m.rect.left, m.rect.top)).unwrap_or((0, 0));
        let (gx, gy) = (x + ox, y + oy);
        self.last = (gx, gy);
        unsafe {
            let (vx, vy) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
            let (vw, vh) = (GetSystemMetrics(SM_CXVIRTUALSCREEN).max(2), GetSystemMetrics(SM_CYVIRTUALSCREEN).max(2));
            (((gx - vx) as i64 * 65535 / (vw - 1) as i64) as i32, ((gy - vy) as i64 * 65535 / (vh - 1) as i64) as i32)
        }
    }

    pub fn inject(&mut self, display: u32, ev: InputEvent) {
        const ABS: MOUSE_EVENT_FLAGS = MOUSE_EVENT_FLAGS(MOUSEEVENTF_MOVE.0 | MOUSEEVENTF_ABSOLUTE.0 | MOUSEEVENTF_VIRTUALDESK.0);
        match ev {
            InputEvent::MouseMove { x, y } => {
                let (ax, ay) = self.abs(display, x, y);
                send(&[mouse_input(ax, ay, 0, ABS)]);
            }
            InputEvent::MouseButton { button, down, x, y } => {
                let (ax, ay) = self.abs(display, x, y);
                let (flag, data) = match (button, down) {
                    (MouseButton::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
                    (MouseButton::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
                    (MouseButton::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
                    (MouseButton::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
                    (MouseButton::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
                    (MouseButton::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
                    (MouseButton::Back, true) => (MOUSEEVENTF_XDOWN, 1),
                    (MouseButton::Back, false) => (MOUSEEVENTF_XUP, 1),
                    (MouseButton::Forward, true) => (MOUSEEVENTF_XDOWN, 2),
                    (MouseButton::Forward, false) => (MOUSEEVENTF_XUP, 2),
                };
                send(&[mouse_input(ax, ay, data, MOUSE_EVENT_FLAGS(ABS.0 | flag.0))]);
            }
            InputEvent::Wheel { dx, dy } => {
                // 协议按像素，Windows 一格 120；约 40 像素一格。正 dy 向上滚
                let mut v = vec![];
                if dy != 0 {
                    v.push(mouse_input(0, 0, dy * 3, MOUSEEVENTF_WHEEL));
                }
                if dx != 0 {
                    v.push(mouse_input(0, 0, -dx * 3, MOUSEEVENTF_HWHEEL));
                }
                send(&v);
            }
            InputEvent::Key { hid_usage, down } => {
                if let Some((scan, ext)) = hid_to_scan(hid_usage) {
                    send(&[scan_key(scan, ext, down)]);
                } else if let Some(vk) = hid_to_vk(hid_usage) {
                    send(&[vk_key(vk, down)]);
                }
            }
            InputEvent::Text { text } => {
                let mut v = vec![];
                for u in text.encode_utf16() {
                    if u == '\n' as u16 || u == '\r' as u16 {
                        v.push(scan_key(0x1C, false, true));
                        v.push(scan_key(0x1C, false, false));
                        continue;
                    }
                    v.push(key_input(VIRTUAL_KEY(0), u, KEYEVENTF_UNICODE));
                    v.push(key_input(VIRTUAL_KEY(0), u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
                }
                send(&v);
            }
            InputEvent::Touch { phase, x, y, .. } => {
                // 单指触控按鼠标左键处理
                let (ax, ay) = self.abs(display, x, y);
                match phase {
                    TouchPhase::Began => {
                        self.touch_down = true;
                        send(&[mouse_input(ax, ay, 0, MOUSE_EVENT_FLAGS(ABS.0 | MOUSEEVENTF_LEFTDOWN.0))]);
                    }
                    TouchPhase::Moved => send(&[mouse_input(ax, ay, 0, ABS)]),
                    TouchPhase::Ended | TouchPhase::Cancelled => {
                        if self.touch_down {
                            self.touch_down = false;
                            send(&[mouse_input(ax, ay, 0, MOUSE_EVENT_FLAGS(ABS.0 | MOUSEEVENTF_LEFTUP.0))]);
                        }
                    }
                }
            }
            InputEvent::SpecialCombo(c) => self.combo(c),
        }
    }

    fn chord(&self, keys: &[(u16, bool)]) {
        let mut v: Vec<INPUT> = keys.iter().map(|(s, e)| scan_key(*s, *e, true)).collect();
        v.extend(keys.iter().rev().map(|(s, e)| scan_key(*s, *e, false)));
        send(&v);
    }

    fn combo(&self, c: SpecialCombo) {
        const CTRL: (u16, bool) = (0x1D, false);
        const ALT: (u16, bool) = (0x38, false);
        const SHIFT: (u16, bool) = (0x2A, false);
        const TAB: (u16, bool) = (0x0F, false);
        const ESC: (u16, bool) = (0x01, false);
        const WIN: (u16, bool) = (0x5B, true);
        const D: (u16, bool) = (0x20, false);
        match c {
            SpecialCombo::CtrlAltDel => {
                if !send_sas() {
                    // 系统策略不允许软件发 SAS 时退回直接按键（安全桌面不会响应，但普通桌面上能打开相应界面的程序会处理）
                    self.chord(&[CTRL, ALT, (0x53, true)]);
                }
            }
            SpecialCombo::LockScreen => unsafe {
                let _ = windows::Win32::System::Shutdown::LockWorkStation();
            },
            SpecialCombo::AltTab => self.chord(&[ALT, TAB]),
            SpecialCombo::WinKey => send(&[vk_key(VK_LWIN, true), vk_key(VK_LWIN, false)]),
            SpecialCombo::TaskManager => self.chord(&[CTRL, SHIFT, ESC]),
            SpecialCombo::ShowDesktop => self.chord(&[WIN, D]),
        }
    }
}

/// 发送安全注意序列（Ctrl+Alt+Del）。需要以 SYSTEM 运行，且系统策略 SoftwareSASGeneration 允许服务发送（安装服务时会设置）。
fn send_sas() -> bool {
    type SendSasFn = unsafe extern "system" fn(i32);
    unsafe {
        let name: Vec<u16> = "sas.dll\0".encode_utf16().collect();
        let Ok(m) = windows::Win32::System::LibraryLoader::LoadLibraryW(windows::core::PCWSTR(name.as_ptr())) else { return false };
        let Some(f) = windows::Win32::System::LibraryLoader::GetProcAddress(m, windows::core::s!("SendSAS")) else { return false };
        let f: SendSasFn = std::mem::transmute(f);
        f(0);
        true
    }
}
