//! 被控端记住每路输入（一路画面、一次 AI 操作）按下还没抬起的键、鼠标键与触摸，结束时全部抬起。
//!
//! 主控端断线、关掉画面、切走时，正按着的键（常见是 Ctrl、Shift、Cmd）的「抬起」可能永远不来：
//! Windows、macOS 被控机上它们就一直按着，本地的人点什么都成了 Ctrl+点击；GNOME Wayland 上普通键还会在
//! 每个新输入框里自动重复。主控端那边也做了兜底（失焦全部抬起、修饰键按实际状态同步），这里是最后一道。

use galaxy_proto::peer::{InputEvent, MouseButton, TouchPhase};

#[derive(Default)]
pub struct HeldInput {
    /// 按下顺序（HID 用法码）
    keys: Vec<u32>,
    buttons: Vec<MouseButton>,
    /// 还没抬起的触摸（编号）
    touches: Vec<u32>,
    pos: (i32, i32),
}

impl HeldInput {
    /// 记下一个要注入的事件（注入前调用）
    pub fn track(&mut self, ev: &InputEvent) {
        match ev {
            InputEvent::Key { hid_usage, down } => {
                self.keys.retain(|k| k != hid_usage);
                if *down {
                    self.keys.push(*hid_usage);
                }
            }
            InputEvent::MouseMove { x, y } => self.pos = (*x, *y),
            InputEvent::MouseButton { button, down, x, y } => {
                self.pos = (*x, *y);
                self.buttons.retain(|b| b != button);
                if *down {
                    self.buttons.push(*button);
                }
            }
            InputEvent::Touch { id, phase, x, y } => {
                self.pos = (*x, *y);
                self.touches.retain(|t| t != id);
                if matches!(phase, TouchPhase::Began | TouchPhase::Moved) {
                    self.touches.push(*id);
                }
            }
            InputEvent::Wheel { .. } | InputEvent::Text { .. } | InputEvent::SpecialCombo(_) => {}
        }
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty() && self.buttons.is_empty() && self.touches.is_empty()
    }

    /// 把还按着的全部抬起：先鼠标键与触摸，再按按下的相反顺序抬键（修饰键通常先按下，最后抬）
    pub fn release_all(&mut self) -> Vec<InputEvent> {
        let (x, y) = self.pos;
        let mut out: Vec<InputEvent> = self.buttons.drain(..).map(|button| InputEvent::MouseButton { button, down: false, x, y }).collect();
        out.extend(self.touches.drain(..).map(|id| InputEvent::Touch { id, phase: TouchPhase::Cancelled, x, y }));
        out.extend(self.keys.drain(..).rev().map(|hid_usage| InputEvent::Key { hid_usage, down: false }));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(h: u32, down: bool) -> InputEvent {
        InputEvent::Key { hid_usage: h, down }
    }

    #[test]
    fn releases_only_what_is_still_held_in_reverse_order() {
        let mut h = HeldInput::default();
        for ev in [key(0xE0, true), key(0x04, true), key(0x04, false), key(0xE1, true), key(0x06, true)] {
            h.track(&ev);
        }
        h.track(&InputEvent::MouseButton { button: MouseButton::Left, down: true, x: 10, y: 20 });
        h.track(&InputEvent::MouseMove { x: 30, y: 40 });
        let out = h.release_all();
        let keys: Vec<u32> = out.iter().filter_map(|e| if let InputEvent::Key { hid_usage, down: false } = e { Some(*hid_usage) } else { None }).collect();
        assert_eq!(keys, vec![0x06, 0xE1, 0xE0], "先抬后按的键，修饰键最后");
        assert!(matches!(out[0], InputEvent::MouseButton { button: MouseButton::Left, down: false, x: 30, y: 40 }));
        assert!(h.is_empty());
        assert!(h.release_all().is_empty());
    }

    #[test]
    fn repeated_down_counts_once_and_touch_is_cancelled() {
        let mut h = HeldInput::default();
        h.track(&key(0x04, true));
        h.track(&key(0x04, true));
        h.track(&InputEvent::Touch { id: 3, phase: TouchPhase::Began, x: 1, y: 2 });
        h.track(&InputEvent::Touch { id: 4, phase: TouchPhase::Began, x: 1, y: 2 });
        h.track(&InputEvent::Touch { id: 4, phase: TouchPhase::Ended, x: 1, y: 2 });
        let out = h.release_all();
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0], InputEvent::Touch { id: 3, phase: TouchPhase::Cancelled, .. }));
        assert!(matches!(out[1], InputEvent::Key { hid_usage: 0x04, down: false }));
    }
}
