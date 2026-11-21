//! 服务与画面代理之间的消息（标准输入输出上的长度前缀帧）。

use galaxy_proto::peer::{ClipboardData, DisplayInfo, ImageData, InputEvent};
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
    Log { error: bool, text: String },
}

impl FromAgent {
    pub fn reply_id(&self) -> Option<u64> {
        match self {
            Self::Displays { id, .. } | Self::Started { id, .. } | Self::Screenshot { id, .. } | Self::Clipboard { id, .. } | Self::Pong { id } | Self::PrivacyResult { id, .. } => Some(*id),
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
