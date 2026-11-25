//! 服务与画面代理之间的消息（标准输入输出上的长度前缀帧，postcard 编码）。与 Windows 被控端同一套写法。

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
    CursorInVideo { display: u32, show: bool },
    Inject { display: u32, ev: InputEvent },
    Screenshot { id: u64, display: u32, max_w: u32, max_h: u32 },
    ClipboardGet { id: u64 },
    ClipboardSet { data: ClipboardData },
    StartAudio { id: u64 },
    StopAudio,
    Ping { id: u64 },
    /// 复制粘贴文件（docs/protocol.md 8.4.1）：把主控端复制的文件清单放进剪贴板，有人粘贴时才读。回 `ClipOfferSet`
    ClipOffer { id: u64, offer: ClipOffer },
    /// `FromAgent::ClipOpen` 的结果：文件现在的大小
    ClipOpened { handle: u64, result: Result<u64, String> },
    /// `FromAgent::ClipRead` 的结果：空数据表示读到末尾
    ClipData { handle: u64, result: Result<Vec<u8>, String> },
    /// 收尾后退出（把输入法切回原来的、卸掉虚拟目录）。服务换会话、停代理时先发这个，等一会再 kill
    Quit,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum FromAgent {
    /// 代理启动后先发：用的哪种后端（x11 / mutter / portal）与说明
    Hello { backend: String, detail: String },
    Displays { id: u64, list: Vec<DisplayInfo> },
    /// 开始采集的结果：成功时是实际分辨率
    Started { id: u64, result: Result<(u32, u32), String> },
    Frame { display: u32, data: Vec<u8>, pts_us: u64, keyframe: bool },
    Resized { display: u32, width: u32, height: u32 },
    /// 光标热点在画面里的位置（画面像素），变化时才发
    Cursor { display: u32, x: i32, y: i32, visible: bool },
    Screenshot { id: u64, image: Option<ImageData> },
    Clipboard { id: u64, data: Option<ClipboardData> },
    /// 本机剪贴板变了（主控端开着剪贴板同步时用来推送；当前内核按需拉取，留作以后用）
    ClipboardChanged,
    /// 开始采集声音的结果：成功时是采样率与声道数
    AudioStarted { id: u64, result: Result<(u32, u16), String> },
    /// 交错排列的浮点样本
    Audio { pcm: Vec<f32>, rate: u32, channels: u16 },
    Pong { id: u64 },
    Log { error: bool, text: String },
    /// 后端坏了（会话结束、显示服务器断开）：服务应当重新找会话、重启代理
    Lost { reason: String },
    ClipOfferSet { id: u64, result: Result<(), String> },
    /// 有人粘贴：打开清单 `offer` 第 `index` 项，从 `offset` 往后读（`handle` 由代理分配）
    ClipOpen { handle: u64, offer: u64, index: u32, offset: u64 },
    /// 要下一段（最多 `len` 字节）。一次只有一个在途，服务那边按需从主控端读，管道里不会堆积大块数据
    ClipRead { handle: u64, len: u32 },
    /// 不读了（服务丢掉读取器，主控端那条流随之停下）
    ClipClose { handle: u64 },
}

impl FromAgent {
    pub fn reply_id(&self) -> Option<u64> {
        match self {
            Self::Displays { id, .. } | Self::Started { id, .. } | Self::Screenshot { id, .. } | Self::Clipboard { id, .. } | Self::Pong { id } | Self::AudioStarted { id, .. } | Self::ClipOfferSet { id, .. } => Some(*id),
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
