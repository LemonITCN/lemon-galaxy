//! X11 键鼠（XTEST）。键码 = evdev 键码 + 8；键盘布局里没有的字符，临时把一个空闲键码映射成它再按（同 xdotool）。

use super::super::backend::{Controller, Monitor};
use super::super::hidmap::{self, WheelAcc};
use super::clipboard::Clipboard;
use super::connect;
use galaxy_proto::peer::{ClipboardData, InputEvent, MouseButton, SpecialCombo, TouchPhase};
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as _, Keycode, Window};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

const KEY_PRESS: u8 = 2;
const KEY_RELEASE: u8 = 3;
const BUTTON_PRESS: u8 = 4;
const BUTTON_RELEASE: u8 = 5;
const MOTION: u8 = 6;

pub struct X11Controller {
    conn: RustConnection,
    root: Window,
    min_kc: u8,
    max_kc: u8,
    wheel: WheelAcc,
    touch_down: bool,
    clip: Clipboard,
    /// 临时映射用过的键码（轮流用，减少反复改映射）
    spare: Vec<Keycode>,
    spare_next: usize,
}

impl X11Controller {
    pub fn new() -> Result<Self, String> {
        let (conn, screen) = connect()?;
        let root = conn.setup().roots[screen].root;
        if conn.xtest_get_version(2, 2).ok().and_then(|c| c.reply().ok()).is_none() {
            return Err("X 服务器没有 XTEST 扩展，不能远程操作键鼠".into());
        }
        let (min_kc, max_kc) = (conn.setup().min_keycode, conn.setup().max_keycode);
        let clip = Clipboard::start()?;
        Ok(Self { conn, root, min_kc, max_kc, wheel: WheelAcc::default(), touch_down: false, clip, spare: vec![], spare_next: 0 })
    }

    fn fake(&self, kind: u8, detail: u8, x: i16, y: i16) {
        let _ = self.conn.xtest_fake_input(kind, detail, 0, self.root, x, y, 0);
    }

    fn flush(&self) {
        let _ = self.conn.flush();
    }

    fn move_to(&self, m: &Monitor, x: i32, y: i32) {
        let gx = (m.x + x.clamp(0, m.width as i32 - 1)) as i16;
        let gy = (m.y + y.clamp(0, m.height as i32 - 1)) as i16;
        self.fake(MOTION, 0, gx, gy);
    }

    fn key(&self, evdev: u32, down: bool) {
        let kc = evdev + 8;
        if kc < self.min_kc as u32 || kc > self.max_kc as u32 {
            return;
        }
        self.fake(if down { KEY_PRESS } else { KEY_RELEASE }, kc as u8, 0, 0);
    }

    fn chord(&self, keys: &[u32]) {
        for k in keys {
            self.key(*k, true);
        }
        for k in keys.iter().rev() {
            self.key(*k, false);
        }
        self.flush();
    }

    fn click(&self, button: u8, times: i32) {
        for _ in 0..times {
            self.fake(BUTTON_PRESS, button, 0, 0);
            self.fake(BUTTON_RELEASE, button, 0, 0);
        }
    }

    /// 键盘映射：(每个键码几个键值, 键值表)
    fn keymap(&self) -> Option<(u8, Vec<u32>)> {
        let n = self.max_kc - self.min_kc + 1;
        let r = self.conn.get_keyboard_mapping(self.min_kc, n).ok()?.reply().ok()?;
        Some((r.keysyms_per_keycode, r.keysyms))
    }

    fn type_char(&mut self, c: char) {
        if c == '\n' || c == '\r' {
            self.key(hidmap::KEY_ENTER, true);
            self.key(hidmap::KEY_ENTER, false);
            return;
        }
        if c == '\t' {
            self.key(hidmap::KEY_TAB, true);
            self.key(hidmap::KEY_TAB, false);
            return;
        }
        let sym = keysym_for(c);
        let Some((per, map)) = self.keymap() else { return };
        let per_us = per as usize;
        // 布局里现成有：第 0 级直接按，第 1 级加 Shift
        for (i, chunk) in map.chunks(per_us).enumerate() {
            for (level, s) in chunk.iter().enumerate().take(2) {
                if *s == sym {
                    let kc = self.min_kc + i as u8;
                    let shift = level == 1;
                    if shift {
                        self.key(hidmap::KEY_LEFTSHIFT, true);
                    }
                    self.fake(KEY_PRESS, kc, 0, 0);
                    self.fake(KEY_RELEASE, kc, 0, 0);
                    if shift {
                        self.key(hidmap::KEY_LEFTSHIFT, false);
                    }
                    return;
                }
            }
        }
        // 没有：找空闲键码临时映射
        if self.spare.is_empty() {
            for (i, chunk) in map.chunks(per_us).enumerate().rev() {
                if chunk.iter().all(|s| *s == 0) {
                    self.spare.push(self.min_kc + i as u8);
                    if self.spare.len() >= 8 {
                        break;
                    }
                }
            }
            if self.spare.is_empty() {
                return;
            }
        }
        let kc = self.spare[self.spare_next % self.spare.len()];
        self.spare_next += 1;
        let syms = vec![sym; per_us.max(1)];
        let _ = self.conn.change_keyboard_mapping(1, kc, per.max(1), &syms);
        self.flush();
        // 等应用收到映射变化通知再按
        std::thread::sleep(Duration::from_millis(12));
        self.fake(KEY_PRESS, kc, 0, 0);
        self.fake(KEY_RELEASE, kc, 0, 0);
        self.flush();
        std::thread::sleep(Duration::from_millis(8));
    }

    fn restore_spares(&mut self) {
        let Some((per, _)) = self.keymap() else { return };
        for kc in std::mem::take(&mut self.spare) {
            let _ = self.conn.change_keyboard_mapping(1, kc, per.max(1), &vec![0u32; per.max(1) as usize]);
        }
        self.flush();
    }
}

/// 字符 → X 键值：Latin-1 直接是码点，其他字符 0x01000000 + 码点
pub fn keysym_for(c: char) -> u32 {
    let u = c as u32;
    if (0x20..=0x7e).contains(&u) || (0xa0..=0xff).contains(&u) {
        u
    } else {
        0x0100_0000 | u
    }
}

fn button_code(b: MouseButton) -> u8 {
    match b {
        MouseButton::Left => 1,
        MouseButton::Middle => 2,
        MouseButton::Right => 3,
        MouseButton::Back => 8,
        MouseButton::Forward => 9,
    }
}

impl Controller for X11Controller {
    fn inject(&mut self, m: &Monitor, ev: InputEvent) {
        match ev {
            InputEvent::MouseMove { x, y } => self.move_to(m, x, y),
            InputEvent::MouseButton { button, down, x, y } => {
                self.move_to(m, x, y);
                self.fake(if down { BUTTON_PRESS } else { BUTTON_RELEASE }, button_code(button), 0, 0);
            }
            InputEvent::Wheel { dx, dy } => {
                let (sx, sy) = self.wheel.add(dx, dy);
                // 4 上 5 下 6 左 7 右；协议正 dy 向上、正 dx 向左（同 Windows 被控端）
                if sy > 0 {
                    self.click(4, sy);
                } else if sy < 0 {
                    self.click(5, -sy);
                }
                if sx > 0 {
                    self.click(6, sx);
                } else if sx < 0 {
                    self.click(7, -sx);
                }
            }
            InputEvent::Key { hid_usage, down } => {
                if let Some(k) = hidmap::evdev(hid_usage) {
                    self.key(k, down);
                }
            }
            InputEvent::Text { text } => {
                for c in text.chars() {
                    self.type_char(c);
                }
                if !self.spare.is_empty() {
                    self.flush();
                    std::thread::sleep(Duration::from_millis(20));
                    self.restore_spares();
                }
            }
            InputEvent::Touch { phase, x, y, .. } => match phase {
                TouchPhase::Began => {
                    self.move_to(m, x, y);
                    self.fake(BUTTON_PRESS, 1, 0, 0);
                    self.touch_down = true;
                }
                TouchPhase::Moved => self.move_to(m, x, y),
                TouchPhase::Ended | TouchPhase::Cancelled => {
                    if std::mem::take(&mut self.touch_down) {
                        self.move_to(m, x, y);
                        self.fake(BUTTON_RELEASE, 1, 0, 0);
                    }
                }
            },
            InputEvent::SpecialCombo(c) => {
                use hidmap::*;
                match c {
                    SpecialCombo::CtrlAltDel => self.chord(&[KEY_LEFTCTRL, KEY_LEFTALT, KEY_DELETE]),
                    SpecialCombo::LockScreen => super::super::actions::lock_screen(),
                    SpecialCombo::AltTab => self.chord(&[KEY_LEFTALT, KEY_TAB]),
                    SpecialCombo::WinKey => self.chord(&[KEY_LEFTMETA]),
                    SpecialCombo::TaskManager => {
                        if !super::super::actions::task_manager() {
                            self.chord(&[KEY_LEFTCTRL, KEY_LEFTALT, KEY_DELETE]);
                        }
                    }
                    SpecialCombo::ShowDesktop => self.chord(&[KEY_LEFTMETA, KEY_D]),
                }
            }
        }
        self.flush();
    }

    fn clipboard_get(&mut self) -> Option<ClipboardData> {
        self.clip.get()
    }

    fn clipboard_set(&mut self, d: ClipboardData) {
        self.clip.set(d)
    }
}
