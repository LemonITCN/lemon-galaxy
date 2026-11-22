//! 由原生界面层实现的回调接口。

use crate::types::*;

/// 本地安全存储：Apple Keychain / Android Keystore / Windows DPAPI。
#[uniffi::export(with_foreign)]
pub trait SecureStore: Send + Sync {
    fn get(&self, key: String) -> Option<Vec<u8>>;
    fn set(&self, key: String, value: Vec<u8>);
    fn remove(&self, key: String);
}

#[uniffi::export(with_foreign)]
pub trait ClientListener: Send + Sync {
    fn on_state(&self, state: ClientState);
    fn on_devices_changed(&self);
    fn on_vault_changed(&self);
    fn on_session_revoked(&self);
}

#[uniffi::export(with_foreign)]
pub trait PeerListener: Send + Sync {
    fn on_path_changed(&self, path: PathInfo);
    fn on_closed(&self, reason: String);
}

#[uniffi::export(with_foreign)]
pub trait TerminalSink: Send + Sync {
    fn on_output(&self, data: Vec<u8>);
    fn on_exit(&self, code: Option<i32>);
}

#[uniffi::export(with_foreign)]
pub trait TransferListener: Send + Sync {
    fn on_progress(&self, done: u64, total: u64);
    fn on_finished(&self, error: Option<String>);
}

#[uniffi::export(with_foreign)]
pub trait VideoSink: Send + Sync {
    fn on_config(&self, codec: VideoCodec, width: u32, height: u32);
    /// Annex B 码流（关键帧前带参数集）
    fn on_frame(&self, data: Vec<u8>, pts_us: u64, keyframe: bool);
    /// 光标位置（编码画面像素，光标热点）
    fn on_cursor(&self, x: i32, y: i32, visible: bool);
    fn on_closed(&self, reason: String);
}

#[uniffi::export(with_foreign)]
pub trait AgentListener: Send + Sync {
    fn on_event(&self, event: AgentEvent);
}

/// 本机被控的平台能力（macOS Swift / Windows C#）。
#[uniffi::export(with_foreign)]
pub trait HostPlatform: Send + Sync {
    fn list_displays(&self) -> Vec<DisplayInfo>;
    /// 开始截屏编码，编码后的帧推给 `sink`；返回实际分辨率与编码，失败返回 None。
    fn start_capture(&self, display_id: u32, options: CaptureOptions, sink: std::sync::Arc<crate::host::EncodedFrameSink>) -> Option<CaptureStarted>;
    fn stop_capture(&self, display_id: u32);
    fn request_keyframe(&self, display_id: u32);
    fn set_bitrate(&self, display_id: u32, kbps: u32);
    /// 画面里要不要画光标（主控端在本地画时关掉）；支持时在 capabilities 里报 "client_cursor"。
    fn set_cursor_in_video(&self, display_id: u32, show: bool);
    /// 坐标是 `display_id` 这块屏幕的像素。
    fn inject(&self, display_id: u32, event: InputEvent);
    fn screenshot(&self, display_id: u32, max_width: u32, max_height: u32) -> Option<ImageData>;
    fn ui_tree(&self, display_id: u32, max_nodes: u32) -> Option<String>;
    /// AI 应用操作（docs/computer-use.md）：请求 JSON，返回给模型的文字与可选窗口截图；不支持返回 None。
    /// 在后台线程调用，可以阻塞（单次最多约 10 秒）。支持时在 capabilities 里报 "computer_use"。
    fn computer_use(&self, request_json: String) -> Option<ComputerUseReply>;
    fn clipboard_get(&self) -> Option<ClipboardData>;
    fn clipboard_set(&self, data: ClipboardData);
    /// 隐私屏：本机屏幕黑掉、本地键鼠失效，远端照常看到和操作。返回是否做到了（不支持返回 false）。
    fn set_privacy(&self, on: bool) -> bool;
    /// 开始采集系统声音，样本推给 `sink`；不支持或没授权返回 false。同一时间只有一路。
    fn start_audio(&self, sink: std::sync::Arc<crate::host::AudioPcmSink>) -> bool;
    fn stop_audio(&self);
    /// 已获得的系统授权对应的能力："display" "screenshot" "input" "ui_tree" "clipboard" "privacy" "audio"
    fn capabilities(&self) -> Vec<String>;
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct CaptureStarted {
    pub width: u32,
    pub height: u32,
    pub codec: VideoCodec,
}

#[uniffi::export(with_foreign)]
pub trait HostListener: Send + Sync {
    fn on_status(&self, status: HostStatus);
    /// 临时协助请求：界面弹窗后调用 HostService.respond_assist
    fn on_assist_request(&self, request_id: String, from_user: String, from_device: String);
    fn on_session_started(&self, info: HostSessionInfo);
    fn on_session_ended(&self, session_id: String);
    fn on_ai_activity(&self, session_id: String, description: String);
}

/// SSH 直连时向用户要东西。内核在自己的线程上同步调用，界面可以阻塞等用户回答。
#[uniffi::export(with_foreign)]
pub trait SecretPrompt: Send + Sync {
    /// 密码、私钥口令、键盘交互的回答；返回 None 表示取消。`echo` 为假时输入框应遮挡。
    fn ask(&self, prompt: String, echo: bool) -> Option<String>;
    /// 首次连接或主机密钥变化时确认。`changed` 为真表示与记录的不一致（可能被冒充），界面应醒目警告。
    fn confirm_host_key(&self, host: String, port: u16, key_type: String, fingerprint: String, changed: bool) -> bool;
}
