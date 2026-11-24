//! USB HID 键盘用法码（协议里的 `hid_usage`）→ Linux evdev 键码。
//! 表照抄内核 drivers/hid/hid-input.c 的 `hid_keyboard[]`（0 表示没有对应键）。
//! Mutter 与桌面门户直接收 evdev 键码；X11（evdev 驱动、Xvfb 默认键盘）是 evdev 键码 + 8。

#[rustfmt::skip]
const HID_KEYBOARD: [u8; 256] = [
      0,  0,  0,  0, 30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38,
     50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45, 21, 44,  2,  3,
      4,  5,  6,  7,  8,  9, 10, 11, 28,  1, 14, 15, 57, 12, 13, 26,
     27, 43, 43, 39, 40, 41, 51, 52, 53, 58, 59, 60, 61, 62, 63, 64,
     65, 66, 67, 68, 87, 88, 99, 70,119,110,102,104,111,107,109,106,
    105,108,103, 69, 98, 55, 74, 78, 96, 79, 80, 81, 75, 76, 77, 71,
     72, 73, 82, 83, 86,127,116,117,183,184,185,186,187,188,189,190,
    191,192,193,194,134,138,130,132,128,129,131,137,133,135,136,113,
    115,114,  0,  0,  0,121,  0, 89, 93,124, 92, 94, 95,  0,  0,  0,
    122,123, 90, 91, 85,  0,  0,  0,  0,  0,  0,  0,111,  0,  0,  0,
      0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,
      0,  0,  0,  0,  0,  0,179,180,  0,  0,  0,  0,  0,  0,  0,  0,
      0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,
      0,  0,  0,  0,  0,  0,  0,  0,111,  0,  0,  0,  0,  0,  0,  0,
     29, 42, 56,125, 97, 54,100,126,164,166,165,163,161,115,114,113,
    150,158,159,128,136,177,178,176,142,152,173,140,  0,  0,  0,  0,
];

/// HID 用法码 → evdev 键码；没有对应键时为 None。
pub fn evdev(hid: u32) -> Option<u32> {
    let k = *HID_KEYBOARD.get(hid as usize)?;
    (k != 0).then_some(k as u32)
}

// 组合键用到的 evdev 键码
pub const KEY_ESC: u32 = 1;
pub const KEY_TAB: u32 = 15;
pub const KEY_LEFTCTRL: u32 = 29;
pub const KEY_LEFTSHIFT: u32 = 42;
pub const KEY_LEFTALT: u32 = 56;
pub const KEY_D: u32 = 32;
pub const KEY_DELETE: u32 = 111;
pub const KEY_LEFTMETA: u32 = 125;
pub const KEY_ENTER: u32 = 28;

// evdev 鼠标按键
pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;
pub const BTN_SIDE: u32 = 0x113;
pub const BTN_EXTRA: u32 = 0x114;

pub fn button(b: galaxy_proto::peer::MouseButton) -> u32 {
    use galaxy_proto::peer::MouseButton::*;
    match b {
        Left => BTN_LEFT,
        Right => BTN_RIGHT,
        Middle => BTN_MIDDLE,
        Back => BTN_SIDE,
        Forward => BTN_EXTRA,
    }
}

/// 滚轮：协议按像素（正 dy 向上），约 40 像素一格。累计余数，返回这次要滚的格数（正为向上 / 向右）。
#[derive(Default)]
pub struct WheelAcc {
    x: i32,
    y: i32,
}

pub const PIXELS_PER_STEP: i32 = 40;

impl WheelAcc {
    pub fn add(&mut self, dx: i32, dy: i32) -> (i32, i32) {
        self.x += dx;
        self.y += dy;
        let sx = self.x / PIXELS_PER_STEP;
        let sy = self.y / PIXELS_PER_STEP;
        self.x -= sx * PIXELS_PER_STEP;
        self.y -= sy * PIXELS_PER_STEP;
        (sx, sy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_keys() {
        assert_eq!(evdev(0x04), Some(30)); // A
        assert_eq!(evdev(0x1E), Some(2)); // 1
        assert_eq!(evdev(0x28), Some(KEY_ENTER));
        assert_eq!(evdev(0x29), Some(KEY_ESC));
        assert_eq!(evdev(0x2B), Some(KEY_TAB));
        assert_eq!(evdev(0x4C), Some(KEY_DELETE));
        assert_eq!(evdev(0x50), Some(105)); // 左
        assert_eq!(evdev(0xE0), Some(KEY_LEFTCTRL));
        assert_eq!(evdev(0xE3), Some(KEY_LEFTMETA));
        assert_eq!(evdev(0xE8), Some(164)); // 播放 / 暂停
        assert_eq!(evdev(0x00), None);
        assert_eq!(evdev(0x1234), None);
    }

    #[test]
    fn wheel_accumulates() {
        let mut w = WheelAcc::default();
        assert_eq!(w.add(0, 30), (0, 0));
        assert_eq!(w.add(0, 30), (0, 1));
        assert_eq!(w.add(0, -100), (0, -2));
        assert_eq!(w.add(85, 0), (2, 0));
    }
}
