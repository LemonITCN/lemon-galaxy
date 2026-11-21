//! 平台能力钩子：截屏编码、输入注入、剪贴板、控件树。由各端原生层实现（macOS Swift、Windows Win32）。

use galaxy_proto::peer::{ClipboardData, DisplayInfo, ImageData, InputEvent, VideoCodec};
use std::sync::Arc;

/// 平台层编码好一帧后推给内核。
pub trait FrameSink: Send + Sync {
    fn push(&self, data: Vec<u8>, pts_us: u64, keyframe: bool);
    fn resized(&self, width: u32, height: u32);
}

pub struct CaptureStarted {
    pub width: u32,
    pub height: u32,
    pub codec: VideoCodec,
}

pub trait HostPlatform: Send + Sync {
    fn list_displays(&self) -> Vec<DisplayInfo> {
        vec![]
    }
    fn start_capture(&self, _display: u32, _max_fps: u32, _bitrate_kbps: u32, _codecs: Vec<VideoCodec>, _sink: Arc<dyn FrameSink>) -> Result<CaptureStarted, String> {
        Err("此设备不支持远程画面".into())
    }
    fn stop_capture(&self, _display: u32) {}
    fn request_keyframe(&self, _display: u32) {}
    fn set_bitrate(&self, _display: u32, _kbps: u32) {}
    /// 注入输入；坐标是 `display` 这块屏幕的像素。
    fn inject(&self, _display: u32, _ev: InputEvent) {}
    fn screenshot(&self, _display: u32, _max_width: u32, _max_height: u32) -> Option<ImageData> {
        None
    }
    fn ui_tree(&self, _display: u32, _max_nodes: u32) -> Option<String> {
        None
    }
    fn clipboard_get(&self) -> Option<ClipboardData> {
        None
    }
    fn clipboard_set(&self, _d: ClipboardData) {}
    /// 隐私屏开关。返回是否支持（不支持时什么也不做）。
    fn set_privacy(&self, _on: bool) -> bool {
        false
    }
    fn capabilities(&self) -> Vec<String> {
        vec![]
    }
}

/// 无图形界面（Linux 命令行 worker）。
pub struct NoPlatform;
impl HostPlatform for NoPlatform {}
