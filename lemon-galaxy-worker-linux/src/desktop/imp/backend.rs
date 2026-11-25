//! 画面后端的公共抽象：X11、Mutter（GNOME Wayland）、桌面门户（其他 Wayland）都实现这些。

use galaxy_proto::peer::{ClipboardData, DisplayInfo, InputEvent};
use std::time::Duration;

/// 一块屏幕（桌面坐标）。`id` 主屏为 0，其余按位置从左到右、从上到下 1、2…（会话切换后仍能对上主屏）。
#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    pub id: u32,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
    pub scale: f32,
    pub is_virtual: bool,
}

impl Monitor {
    pub fn info(&self) -> DisplayInfo {
        DisplayInfo { id: self.id, name: self.name.clone(), width: self.width, height: self.height, scale: self.scale, primary: self.primary, is_virtual: self.is_virtual }
    }
}

/// 按「主屏 0，其余按位置」分配 id。
pub fn assign_ids(mut v: Vec<Monitor>) -> Vec<Monitor> {
    v.sort_by_key(|m| (!m.primary, m.x, m.y));
    if !v.iter().any(|m| m.primary) {
        if let Some(m) = v.first_mut() {
            m.primary = true;
        }
    }
    for (i, m) in v.iter_mut().enumerate() {
        m.id = i as u32;
    }
    v
}

/// 一帧 BGRA（每行 width*4 字节，紧密排列）。
#[derive(Default, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Frame {
    pub fn resize(&mut self, w: u32, h: u32) {
        self.width = w;
        self.height = h;
        self.data.resize(w as usize * h as usize * 4, 0);
    }
}

/// 光标：图像是预乘 alpha 的 ARGB（X11 XFixes 与 Mutter 元数据都是这个格式），热点已经减掉。
#[derive(Clone, Default)]
pub struct CursorImage {
    pub serial: u64,
    pub width: u32,
    pub height: u32,
    pub hot_x: i32,
    pub hot_y: i32,
    pub argb: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorPos {
    /// 热点在这块屏幕里的位置（屏幕像素）
    pub x: i32,
    pub y: i32,
    pub visible: bool,
}

/// 把光标画进帧里（帧是这块屏幕的画面，坐标同屏幕像素）。
pub fn draw_cursor(f: &mut Frame, img: &CursorImage, pos: CursorPos) {
    if !pos.visible || img.width == 0 {
        return;
    }
    let (x0, y0) = (pos.x - img.hot_x, pos.y - img.hot_y);
    for cy in 0..img.height as i32 {
        let y = y0 + cy;
        if y < 0 || y >= f.height as i32 {
            continue;
        }
        for cx in 0..img.width as i32 {
            let x = x0 + cx;
            if x < 0 || x >= f.width as i32 {
                continue;
            }
            let p = img.argb[(cy * img.width as i32 + cx) as usize];
            let a = p >> 24;
            if a == 0 {
                continue;
            }
            let i = (y as usize * f.width as usize + x as usize) * 4;
            let px = &mut f.data[i..i + 4];
            // 预乘 alpha：dst = src + dst * (1 - a)
            let inv = 255 - a;
            let blend = |s: u32, d: u8| -> u8 { (s + (d as u32 * inv + 127) / 255).min(255) as u8 };
            px[0] = blend(p & 0xff, px[0]);
            px[1] = blend((p >> 8) & 0xff, px[1]);
            px[2] = blend((p >> 16) & 0xff, px[2]);
        }
    }
}

/// 某块屏的采集器（每路画面一个，跑在自己的线程里）。
pub trait Capturer: Send {
    /// 等下一帧，最多等 `timeout`。画面变了写进 `frame` 返回 Ok(true)；没变返回 Ok(false)。
    /// 返回 Err 时调用方会重建采集器。
    fn next(&mut self, frame: &mut Frame, timeout: Duration) -> Result<bool, String>;
    /// 当前画面尺寸（屏幕像素）
    fn size(&self) -> (u32, u32);
    /// 光标位置（相对这块屏）与图像（图像没变时可以返回同一个 serial）。
    fn cursor(&mut self) -> Option<(CursorPos, Option<CursorImage>)>;
    /// 帧里已经带光标（Mutter 嵌入模式）：这时不能再画，也不能去掉
    fn cursor_embedded(&self) -> bool {
        false
    }
    /// 主控端自己画光标时关掉画面里的光标（后端支持时）
    fn set_cursor_in_video(&mut self, _show: bool) {}
    fn kind(&self) -> &'static str;
}

/// 键鼠与剪贴板。
pub trait Controller: Send {
    fn inject(&mut self, display: &Monitor, ev: InputEvent);
    fn clipboard_get(&mut self) -> Option<ClipboardData>;
    fn clipboard_set(&mut self, d: ClipboardData);
    /// 有没有可用的剪贴板（门户后端没有）
    fn clipboard_available(&self) -> bool {
        true
    }
    /// 放一份有人粘贴时才准备的文件列表（没有 FUSE 时的兜底）
    fn clipboard_set_lazy(&mut self, _files: std::sync::Arc<dyn super::clipfmt::LazyFiles>) -> Result<(), String> {
        Err("这个桌面不支持远程剪贴板".into())
    }
}

/// 一个后端：列屏幕、开采集器、给出键鼠控制器。
pub trait Backend: Send + Sync {
    fn name(&self) -> &'static str;
    fn monitors(&self) -> Vec<Monitor>;
    fn capturer(&self, m: &Monitor) -> Result<Box<dyn Capturer>, String>;
    fn controller(&self) -> Result<Box<dyn Controller>, String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_primary_first() {
        let m = |x, p| Monitor { id: 9, name: String::new(), x, y: 0, width: 10, height: 10, primary: p, scale: 1.0, is_virtual: false };
        let v = assign_ids(vec![m(1920, false), m(0, false), m(3840, true)]);
        assert_eq!(v.iter().map(|m| (m.id, m.x)).collect::<Vec<_>>(), vec![(0, 3840), (1, 0), (2, 1920)]);
        let v = assign_ids(vec![m(1920, false), m(0, false)]);
        assert!(v[0].primary && v[0].x == 0);
    }

    #[test]
    fn cursor_blend() {
        let mut f = Frame::default();
        f.resize(4, 4);
        let img = CursorImage { serial: 1, width: 2, height: 2, hot_x: 1, hot_y: 1, argb: vec![0xff_ff_00_00, 0, 0x80_00_00_80, 0xff_00_ff_00] };
        draw_cursor(&mut f, &img, CursorPos { x: 1, y: 1, visible: true });
        // (0,0) 是不透明红色
        assert_eq!(&f.data[0..3], &[0, 0, 255]);
        // (1,0) 透明，不变
        assert_eq!(&f.data[4..7], &[0, 0, 0]);
        // (0,1) 半透明蓝（预乘）
        assert_eq!(f.data[16], 0x80);
    }
}
