//! 服务与画面代理、剪贴板助手之间的消息（标准输入输出上的长度前缀帧）。
//! 剪贴板助手（clipole.rs）只用 Clip* 这几种，服务也只认它发来的这几种。

use galaxy_proto::peer::{ClipOffer, ClipboardData, DisplayInfo, ImageData, InputEvent};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

#[derive(Debug, Serialize, Deserialize)]
pub enum ToAgent {
    ListDisplays { id: u64 },
    StartCapture { id: u64, display: u32, max_fps: u32, bitrate_kbps: u32 },
    StopCapture { display: u32 },
    RequestKeyframe { display: u32 },
    SetBitrate { display: u32, kbps: u32 },
    Inject { display: u32, ev: InputEvent },
    Screenshot { id: u64, display: u32, max_w: u32, max_h: u32 },
    ClipboardGet { id: u64 },
    ClipboardSet { data: ClipboardData },
    Ping { id: u64 },
    Privacy { id: u64, on: bool },
    StartAudio { id: u64 },
    StopAudio,
    /// 加虚拟屏并扩展进桌面
    VirtualAdd { id: u64, width: u32, height: u32, refresh: u32 },
    VirtualRemove { id: u64, display: u32 },
    /// 查现有虚拟屏与是否需要无头兜底屏
    VirtualState { id: u64 },
    /// 画面里画不画光标（主控端自己画光标时关掉）
    CursorInVideo { display: u32, show: bool },
    /// AI 应用操作（docs/computer-use.md）：请求 JSON，在用户会话里由 galaxy_host::win_cu 执行
    ComputerUse { id: u64, request: String },
    /// 复制粘贴文件（docs/protocol.md 8.4.1，发给剪贴板助手）：主控端复制的文件清单，延迟放进剪贴板
    ClipOffer { id: u64, offer: ClipOffer },
    /// 打开清单某项的结果：文件现在的大小
    ClipOpened { stream: u64, result: Result<u64, String> },
    /// 一块数据（应答 ClipRead；空表示到了末尾）
    ClipData { stream: u64, result: Result<Vec<u8>, String> },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum FromAgent {
    Displays { id: u64, list: Vec<DisplayInfo> },
    /// 开始采集的结果：成功时是实际分辨率
    Started { id: u64, result: Result<(u32, u32), String> },
    Frame { display: u32, data: Vec<u8>, pts_us: u64, keyframe: bool },
    Resized { display: u32, width: u32, height: u32 },
    Screenshot { id: u64, image: Option<ImageData> },
    Clipboard { id: u64, data: Option<ClipboardData> },
    Pong { id: u64 },
    PrivacyResult { id: u64, result: Result<(), String> },
    /// 开始采集声音的结果：成功时是采样率与声道数
    AudioStarted { id: u64, result: Result<(u32, u16), String> },
    /// 交错排列的浮点样本
    Audio { pcm: Vec<f32>, rate: u32, channels: u16 },
    Log { error: bool, text: String },
    VirtualAdded { id: u64, result: Result<DisplayInfo, String> },
    VirtualRemoved { id: u64, result: Result<(), String> },
    VirtualState { id: u64, ids: Vec<u32>, needs_headless: bool },
    /// 光标热点在画面里的位置（画面像素），变化时才发
    Cursor { display: u32, x: i32, y: i32, visible: bool },
    /// 应用操作的结果：给模型的文字 + 可选窗口截图
    ComputerUse { id: u64, text: String, image: Option<ImageData> },
    /// 文件清单放进剪贴板的结果
    ClipOfferSet { id: u64, result: Result<(), String> },
    /// （剪贴板助手）有人粘贴：打开清单 `offer` 的第 `index` 项，从 `offset` 读起（`stream` 由助手编号）
    ClipOpen { stream: u64, offer: u64, index: u32, offset: u64 },
    /// 要下一块（最多 `max` 字节）。一条流同时只有一个请求，数据不会堆在管道里挤住键鼠事件
    ClipRead { stream: u64, max: u32 },
    /// 不读了
    ClipClose { stream: u64 },
}

impl FromAgent {
    pub fn reply_id(&self) -> Option<u64> {
        match self {
            Self::Displays { id, .. } | Self::Started { id, .. } | Self::Screenshot { id, .. } | Self::Clipboard { id, .. } | Self::Pong { id } | Self::PrivacyResult { id, .. } | Self::AudioStarted { id, .. } | Self::VirtualAdded { id, .. } | Self::VirtualRemoved { id, .. } | Self::VirtualState { id, .. } | Self::ComputerUse { id, .. } | Self::ClipOfferSet { id, .. } => Some(*id),
            _ => None,
        }
    }
}

pub fn write_msg<W: Write, T: Serialize>(w: &mut W, v: &T) -> io::Result<()> {
    w.write_all(&galaxy_proto::frame::encode(v))?;
    w.flush()
}

pub fn read_msg<R: Read, T: DeserializeOwned>(r: &mut R) -> io::Result<Option<T>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof || e.kind() == io::ErrorKind::BrokenPipe => return Ok(None),
        Err(e) => return Err(e),
    }
    let n = u32::from_le_bytes(len) as usize;
    galaxy_proto::frame::check_len(n, 64 * 1024 * 1024).map_err(io::Error::other)?;
    let mut body = vec![0u8; n];
    r.read_exact(&mut body)?;
    galaxy_proto::frame::decode_body(&body).map(Some).map_err(io::Error::other)
}
