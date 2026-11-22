//! 平台能力钩子：截屏编码、输入注入、剪贴板、控件树。由各端原生层实现（macOS Swift、Windows Win32）。

use galaxy_proto::peer::{ClipboardData, DisplayInfo, ImageData, InputEvent, VideoCodec};
use std::sync::Arc;

/// 平台层编码好一帧后推给内核。
pub trait FrameSink: Send + Sync {
    fn push(&self, data: Vec<u8>, pts_us: u64, keyframe: bool);
    fn resized(&self, width: u32, height: u32);
}

#[derive(Clone, Debug)]
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
    /// 开始采集系统声音，样本推给 `sink`（见 audio.rs）。同一时间只有一路。
    fn start_audio(&self, _sink: Arc<dyn crate::audio::AudioSink>) -> Result<(), String> {
        Err("此设备不支持传声音".into())
    }
    fn stop_audio(&self) {}
    /// 屏幕前登录的系统用户（被控服务以 SYSTEM / root 运行时，临时协助按这个用户执行终端与文件）。
    fn interactive_user(&self) -> Option<String> {
        None
    }
    fn capabilities(&self) -> Vec<String> {
        vec![]
    }

    // ---------------- 虚拟屏 ----------------
    // 内核负责归属（哪个会话加的、无头兜底屏）与上限，平台层只管加删。这些调用可能阻塞几秒，内核在阻塞线程里调。

    /// 主控最多能加几块虚拟屏（不含无头兜底屏）；0 表示不支持（没有驱动或系统不支持）。
    fn virtual_display_max(&self) -> u32 {
        0
    }
    /// 加一块虚拟屏并扩展进桌面，返回它的屏幕信息（`is_virtual` 为真，id 可直接用来开画面）。
    fn add_virtual_display(&self, _width: u32, _height: u32, _refresh: u32) -> Result<DisplayInfo, String> {
        Err("此设备不支持虚拟屏".into())
    }
    /// 删掉一块虚拟屏并恢复显示配置。已经不存在时返回 Ok。
    fn remove_virtual_display(&self, _id: u32) -> Result<(), String> {
        Ok(())
    }
    /// 现存的虚拟屏 id（画面代理重启等情况下虚拟屏会随之消失，内核据此对账）。
    fn virtual_display_ids(&self) -> Vec<u32> {
        vec![]
    }
    /// 本机没有任何亮着的显示器（不算我们自己的虚拟屏），远程桌面需要一块无头兜底屏。
    fn needs_headless_display(&self) -> bool {
        false
    }
}

/// 无图形界面（Linux 命令行 worker）。
pub struct NoPlatform;
impl HostPlatform for NoPlatform {}

/// 端到端测试用：没有画面，只有一路 440 Hz 测试音（环境变量 LEMON_GALAXY_TEST_TONE=1 时 Linux 被控端使用）。
#[derive(Default)]
pub struct TonePlatform(crate::audio::ToneSource);
impl HostPlatform for TonePlatform {
    fn start_audio(&self, sink: Arc<dyn crate::audio::AudioSink>) -> Result<(), String> {
        self.0.start(sink);
        Ok(())
    }
    fn stop_audio(&self) {
        self.0.stop()
    }
    fn capabilities(&self) -> Vec<String> {
        vec!["audio".into()]
    }
}
