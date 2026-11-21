//! 两端 QUIC 会话里的消息（postcard 帧），见 docs/protocol.md 第 8 节。

use crate::cert::SignedBlob;
use crate::{HostInfo, Permissions};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeerRole {
    Controller,
    Host,
}

/// 控制流上双方互发的第一条消息。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hello {
    pub proto: u32,
    pub role: PeerRole,
    pub app_version: String,
    /// 设备证书（账号签名）；临时协助模式下被控端没有
    pub cert: Option<SignedBlob>,
    /// 设备公钥（协助模式下用于识别，正常模式下与证书里的一致）
    pub device_pubkey: [u8; 32],
    /// Ed25519(device_key, SESSION_AUTH_DOMAIN || exporter || role)
    pub sig: Vec<u8>,
    /// 服务器签发的连接票据（主控端发送；离线局域网模式没有）
    pub ticket: Option<SignedBlob>,
    /// 临时协助模式
    pub assist: bool,
}

/// 协助模式的 SPAKE2 消息。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AssistAuth {
    Spake(Vec<u8>),
    /// 双方用 SPAKE2 结果对 exporter 计算的确认码
    Confirm(Vec<u8>),
    /// 被控端本人是否允许
    Decision { allowed: bool },
    Failed { reason: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HelloAck {
    pub host: HostInfo,
    pub account: Option<u64>,
    pub permissions: Permissions,
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum HandshakeResult {
    Ok(HelloAck),
    Rejected { code: String, message: String },
}

/// 控制流上的后续消息。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ControlMsg {
    Ping(u64),
    Pong(u64),
    Bye { reason: String },
    PermissionsChanged(Permissions),
}

/// 发起者来源：用户手动 / AI。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Origin {
    User,
    Ai { conversation: String, step: u32 },
}

impl Origin {
    pub fn is_ai(&self) -> bool {
        matches!(self, Origin::Ai { .. })
    }
}

/// 每条功能流的第一帧。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum StreamOpen {
    Terminal(TerminalOpen),
    Mgmt,
    Files(FileOp),
    Exec(ExecRequest),
    PortForward { host: String, port: u16 },
    Display(DisplayRequest),
    Clipboard,
}

/// 功能流被拒绝等通用错误。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerError {
    pub code: String,
    pub message: String,
}

impl PeerError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.to_string(), message: message.into() }
    }
    pub fn denied(message: impl Into<String>) -> Self {
        Self::new("permission_denied", message)
    }
}

impl std::fmt::Display for PeerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

// ---------------- 终端 ----------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TerminalOpen {
    pub attach: Option<String>,
    pub shell: Option<String>,
    pub cols: u16,
    pub rows: u16,
    pub env: Vec<(String, String)>,
    pub origin: Origin,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TermIn {
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Detach,
    Kill,
    /// AI 读取当前屏幕文字
    Snapshot,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TermOut {
    Opened { session_id: String, title: String, attached_existing: bool },
    Output(Vec<u8>),
    Snapshot(String),
    Exited { code: Option<i32> },
    Error(PeerError),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TerminalSessionInfo {
    pub id: String,
    pub title: String,
    pub created_at: i64,
    pub attached: bool,
}

// ---------------- 管理 ----------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MgmtReq {
    ListTerminals,
    KillTerminal { id: String },
    ListDisplays,
    Screenshot { display: u32, max_width: u32, max_height: u32 },
    UiTree { display: u32, max_nodes: u32 },
    ClipboardGet,
    ClipboardSet(ClipboardData),
    WakeOnLan { mac: String },
    HostStatus,
    /// 不开画面直接注入输入（AI 操作图形界面用）
    Inject { events: Vec<InputEvent>, origin: Origin },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MgmtResp {
    Terminals(Vec<TerminalSessionInfo>),
    Displays(Vec<DisplayInfo>),
    Image(ImageData),
    Text(String),
    Clipboard(Option<ClipboardData>),
    Ok,
    Error(PeerError),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DisplayInfo {
    pub id: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub primary: bool,
    pub is_virtual: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageData {
    pub mime: String,
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ClipboardData {
    Text(String),
    Image(ImageData),
    Files(Vec<String>),
}

// ---------------- 文件 ----------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum FileOp {
    Home,
    List { path: String },
    Stat { path: String },
    Read { path: String, offset: u64, len: Option<u64> },
    Write { path: String, offset: u64, truncate: bool, mode: Option<u32>, expect_sha256: Option<Vec<u8>>, origin: Origin },
    Mkdir { path: String, parents: bool },
    Remove { path: String, recursive: bool, origin: Origin },
    Rename { from: String, to: String, overwrite: bool, origin: Origin },
    Chmod { path: String, mode: u32 },
    Realpath { path: String },
    Search { root: String, pattern: String, content: bool, max: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileKind {
    File,
    Dir,
    Symlink,
    Other,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub kind: FileKind,
    pub size: u64,
    pub mode: u32,
    pub modified_at: i64,
    pub owner: String,
    pub group: String,
    pub link_target: Option<String>,
}

/// 文件流上的消息（双向）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum FileMsg {
    Chunk(Vec<u8>),
    End,
    Path(String),
    Entries(Vec<FileEntry>),
    Entry(FileEntry),
    /// Read 开始时告知总大小
    Size(u64),
    /// Write 完成后的结果
    Written { size: u64, sha256: Vec<u8>, backup: Option<String> },
    Ok,
    Error(PeerError),
}

// ---------------- 命令执行 ----------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecRequest {
    pub command: String,
    pub shell: Option<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
    pub timeout_ms: u64,
    pub origin: Origin,
    pub max_output: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ExecIn {
    Stdin(Vec<u8>),
    CloseStdin,
    Cancel,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ExecDone {
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub timed_out: bool,
    pub truncated: bool,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ExecOut {
    Started,
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Done(ExecDone),
    Error(PeerError),
}

// ---------------- 端口映射 ----------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ForwardReply {
    Ok,
    Error(PeerError),
}

// ---------------- 画面 ----------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoCodec {
    H264,
    H265,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DisplayRequest {
    pub display_id: u32,
    pub codecs: Vec<VideoCodec>,
    pub max_fps: u32,
    pub bitrate_kbps: u32,
    /// 只看不控（权限不足或用户选择）
    pub view_only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TouchPhase {
    Began,
    Moved,
    Ended,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpecialCombo {
    CtrlAltDel,
    LockScreen,
    AltTab,
    WinKey,
    TaskManager,
    ShowDesktop,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InputEvent {
    MouseMove { x: i32, y: i32 },
    MouseButton { button: MouseButton, down: bool, x: i32, y: i32 },
    Wheel { dx: i32, dy: i32 },
    Key { hid_usage: u32, down: bool },
    Text { text: String },
    Touch { id: u32, phase: TouchPhase, x: i32, y: i32 },
    SpecialCombo(SpecialCombo),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum DisplayIn {
    Input(InputEvent),
    RequestKeyframe,
    SetBitrate(u32),
    Stop,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum DisplayOut {
    Started { width: u32, height: u32, codec: VideoCodec, channel: u8 },
    Resized { width: u32, height: u32 },
    Cursor { x: i32, y: i32, visible: bool },
    Error(PeerError),
}

/// 画面数据报头，见 docs/protocol.md 8.3。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoFragHeader {
    pub channel: u8,
    pub frame_id: u32,
    pub frag_idx: u16,
    pub frag_count: u16,
    pub keyframe: bool,
    /// 只在 frag_idx == 0 时有效
    pub pts_us: u64,
}

impl VideoFragHeader {
    pub const BASE_LEN: usize = 1 + 4 + 2 + 2 + 1;

    pub fn encode(&self, out: &mut Vec<u8>) {
        out.push(self.channel);
        out.extend_from_slice(&self.frame_id.to_le_bytes());
        out.extend_from_slice(&self.frag_idx.to_le_bytes());
        out.extend_from_slice(&self.frag_count.to_le_bytes());
        out.push(if self.keyframe { 1 } else { 0 });
        if self.frag_idx == 0 {
            out.extend_from_slice(&self.pts_us.to_le_bytes());
        }
    }

    /// 解析，返回 (头, 负载起始位置)。
    pub fn decode(b: &[u8]) -> Option<(Self, usize)> {
        if b.len() < Self::BASE_LEN {
            return None;
        }
        let channel = b[0];
        let frame_id = u32::from_le_bytes(b[1..5].try_into().ok()?);
        let frag_idx = u16::from_le_bytes(b[5..7].try_into().ok()?);
        let frag_count = u16::from_le_bytes(b[7..9].try_into().ok()?);
        let keyframe = b[9] & 1 == 1;
        let mut pos = Self::BASE_LEN;
        let mut pts_us = 0;
        if frag_idx == 0 {
            if b.len() < pos + 8 {
                return None;
            }
            pts_us = u64::from_le_bytes(b[pos..pos + 8].try_into().ok()?);
            pos += 8;
        }
        if frag_count == 0 || frag_idx >= frag_count {
            return None;
        }
        Some((Self { channel, frame_id, frag_idx, frag_count, keyframe, pts_us }, pos))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frag_header_roundtrip() {
        for idx in [0u16, 3] {
            let h = VideoFragHeader { channel: 2, frame_id: 77, frag_idx: idx, frag_count: 5, keyframe: true, pts_us: if idx == 0 { 123456 } else { 0 } };
            let mut v = Vec::new();
            h.encode(&mut v);
            v.extend_from_slice(b"payload");
            let (d, pos) = VideoFragHeader::decode(&v).unwrap();
            assert_eq!(d, h);
            assert_eq!(&v[pos..], b"payload");
        }
    }
}
