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
    /// 远端声音（docs/protocol.md 8.5）。放在最后，老版本被控端解不出来会直接关流。
    Audio(AudioRequest),
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
    /// 不开画面直接注入输入（AI 操作图形界面用）；坐标是 `display` 这块屏幕的像素
    Inject { display: u32, events: Vec<InputEvent>, origin: Origin },
    /// 隐私屏：被控端本地屏幕黑掉、本地键鼠失效，远端照常看到和操作。开启它的会话结束时自动关闭
    SetPrivacy { on: bool },
    /// 加一块虚拟屏（需要控制权限，能力列表有 "virtual_display"）。应答 `MgmtResp::Display`，之后按它的 id 开画面。
    /// 虚拟屏属于加它的会话，会话结束时自动删掉。`refresh` 为 0 时用 60 Hz
    VirtualDisplayAdd { width: u32, height: u32, refresh: u32 },
    /// 删掉本会话加的虚拟屏
    VirtualDisplayRemove { display_id: u32 },
    /// AI 应用操作（docs/computer-use.md）：请求是 JSON，需要控制权限，能力列表有 "computer_use"。放在最后，老版本的变体编号不变
    ComputerUse { request: String, origin: Origin },
    /// 复制粘贴文件（docs/protocol.md 8.4.1）：本机复制的文件清单放到被控端剪贴板，内容等对方粘贴时
    /// 由被控端经反向流（`ReverseOpen::ClipboardRead`）来读。需要剪贴板与文件权限，能力列表有 "clipboard_offer"。
    /// 放在最后，老版本的变体编号不变
    ClipboardOffer(ClipOffer),
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
    /// 新加的虚拟屏（应答 `VirtualDisplayAdd`）。放在最后，老版本的变体编号不变
    Display(DisplayInfo),
    /// 应用操作的结果（应答 `ComputerUse`）：给模型看的文字 + 可选的窗口截图
    ComputerUse { text: String, image: Option<ImageData> },
}

/// AI 应用操作能力（docs/computer-use.md）
pub const CAP_COMPUTER_USE: &str = "computer_use";

/// 被控端空闲时也每秒至少发一帧画面（保活）。主控端据此判断画面卡死：几秒没数据先要关键帧，再没有就结束画面重连。
/// 不报这个能力的被控端（如 macOS 被控端画面不变时不发帧）主控端不做这个判断。
pub const CAP_VIDEO_KEEPALIVE: &str = "video_keepalive";

/// 虚拟屏能力：能力列表里有 "virtual_display"，另有 "virtual_display_max=N" 给出主控最多能加几块。
pub const CAP_VIRTUAL_DISPLAY: &str = "virtual_display";
pub const CAP_VIRTUAL_DISPLAY_MAX: &str = "virtual_display_max";

// ---------------- 复制粘贴文件（粘贴时才传，docs/protocol.md 8.4.1） ----------------

/// 被控端支持「文件清单放进剪贴板、粘贴时才读内容」（`MgmtReq::ClipboardOffer`）。没有时主控端照旧整批上传。
pub const CAP_CLIPBOARD_OFFER: &str = "clipboard_offer";

/// 一份清单最多多少项（文件与文件夹合计）
pub const CLIP_OFFER_MAX_ENTRIES: usize = 10_000;

/// 剪贴板文件清单里的一项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipOfferEntry {
    /// 相对路径，用 '/' 分隔；顶层项（复制的那几项）没有 '/'。文件夹排在它的内容前面
    pub path: String,
    pub is_dir: bool,
    /// 文件大小（文件夹为 0）
    pub size: u64,
    /// 修改时间（Unix 秒，不知道时为 0）
    pub mtime: i64,
}

/// 剪贴板文件清单：复制时只交换这个，粘贴时才按项读内容。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipOffer {
    /// 复制方生成的随机编号；读内容时带上
    pub id: u64,
    pub entries: Vec<ClipOfferEntry>,
}

impl ClipOffer {
    pub fn total_bytes(&self) -> u64 {
        self.entries.iter().filter(|e| !e.is_dir).map(|e| e.size).sum()
    }

    /// 顶层项（复制的那几项）的下标
    pub fn top_level(&self) -> Vec<usize> {
        self.entries.iter().enumerate().filter(|(_, e)| !e.path.contains('/')).map(|(i, _)| i).collect()
    }

    /// 粘贴方落盘前必须校验：路径只能是安全的相对路径（不能逃出粘贴目标目录），父文件夹排在前面，不超上限。
    pub fn validate(&self) -> Result<(), String> {
        if self.entries.is_empty() {
            return Err("清单是空的".into());
        }
        if self.entries.len() > CLIP_OFFER_MAX_ENTRIES {
            return Err(format!("超过 {CLIP_OFFER_MAX_ENTRIES} 项"));
        }
        let mut dirs = std::collections::HashSet::new();
        let mut seen = std::collections::HashSet::new();
        for e in &self.entries {
            let p = e.path.as_str();
            if p.is_empty() || p.len() > 4096 || p.contains(['\\', '\0']) {
                return Err(format!("路径无效：{p}"));
            }
            if p.split('/').any(|c| c.is_empty() || c == "." || c == "..") {
                return Err(format!("路径无效：{p}"));
            }
            if let Some((parent, _)) = p.rsplit_once('/') {
                if !dirs.contains(parent) {
                    return Err(format!("{p} 的上级文件夹不在它前面"));
                }
            }
            if !seen.insert(p) {
                return Err(format!("重复的路径：{p}"));
            }
            if e.is_dir {
                dirs.insert(p);
            }
        }
        Ok(())
    }
}

/// 被控端向主控端开的流（反向）的第一帧。只有发过 `ClipboardOffer` 的主控端会收到。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ReverseOpen {
    /// 读清单 `offer_id` 第 `index` 项（必须是文件）从 `offset` 到末尾的内容。
    /// 主控端先回 `ClipReadReply`，是 Ok 时接着发原始字节（不分帧）直到文件末尾，然后结束流。
    /// 被控端中途不要了就 stop 这条流。
    ClipboardRead { offer_id: u64, index: u32, offset: u64 },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ClipReadReply {
    /// `size` 是文件现在的大小（可能和清单里的不同：复制后文件又变了）
    Ok { size: u64 },
    Error(PeerError),
}

/// 能力列表里 `key=value` 形式的值（如 `virtual_display_max=3`）。
pub fn capability_value<'a>(caps: &'a [String], key: &str) -> Option<&'a str> {
    caps.iter().find_map(|c| c.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
}

/// 虚拟屏分辨率与刷新率的合法范围（与 Windows 驱动的校验一致，再收紧到常用范围）。
pub fn virtual_display_mode_ok(width: u32, height: u32, refresh: u32) -> bool {
    (640..=7680).contains(&width) && (480..=4320).contains(&height) && (24..=240).contains(&refresh)
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
    /// 主控端自己在本地画光标（跟手）：被控端发出的画面里不画光标，只在位置变化时发 Cursor。
    /// 加在末尾，老版本被控端收到会关流——主控端只在能力列表有 "client_cursor" 时才发。
    ClientCursor(bool),
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

// ---------------- 声音 ----------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioCodec {
    Opus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioRequest {
    pub codecs: Vec<AudioCodec>,
    /// 期望码率，被控端可能调整
    pub bitrate_kbps: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AudioOut {
    /// 声音包走 DATAGRAM，首字节是 `channel`
    Started { codec: AudioCodec, sample_rate: u32, channels: u8, channel: u8 },
    Error(PeerError),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AudioIn {
    Stop,
}

/// 声音数据报：`channel u8 | seq u32 LE | 一个 Opus 包（20 毫秒）`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioPacketHeader {
    pub channel: u8,
    pub seq: u32,
}

impl AudioPacketHeader {
    pub const LEN: usize = 5;

    pub fn encode(&self, out: &mut Vec<u8>) {
        out.push(self.channel);
        out.extend_from_slice(&self.seq.to_le_bytes());
    }

    pub fn decode(b: &[u8]) -> Option<(Self, usize)> {
        if b.len() < Self::LEN {
            return None;
        }
        Some((Self { channel: b[0], seq: u32::from_le_bytes(b[1..5].try_into().ok()?) }, Self::LEN))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(paths: &[(&str, bool)]) -> ClipOffer {
        ClipOffer { id: 1, entries: paths.iter().map(|(p, d)| ClipOfferEntry { path: p.to_string(), is_dir: *d, size: if *d { 0 } else { 3 }, mtime: 0 }).collect() }
    }

    #[test]
    fn clip_offer_validate() {
        let ok = offer(&[("报告", true), ("报告/a.txt", false), ("报告/子", true), ("报告/子/b.bin", false), ("top.txt", false)]);
        assert_eq!(ok.validate(), Ok(()));
        assert_eq!(ok.total_bytes(), 9);
        assert_eq!(ok.top_level(), vec![0, 4]);
        // 逃出目标目录、绝对路径、空段、反斜杠
        for bad in ["../x", "a/../../x", "/etc/passwd", "a//b", "./a", "a\\..\\x", "", "a\0b"] {
            assert!(offer(&[("a", true), (bad, false)]).validate().is_err(), "{bad:?} 应该被拒");
        }
        // 上级文件夹不在前面、上级是文件、重复
        assert!(offer(&[("d/x", false), ("d", true)]).validate().is_err());
        assert!(offer(&[("d", false), ("d/x", false)]).validate().is_err());
        assert!(offer(&[("a", false), ("a", false)]).validate().is_err());
        assert!(offer(&[]).validate().is_err());
    }

    #[test]
    fn audio_header_roundtrip() {
        let h = AudioPacketHeader { channel: 9, seq: 0xdead_beef };
        let mut v = Vec::new();
        h.encode(&mut v);
        v.extend_from_slice(b"opus");
        let (d, pos) = AudioPacketHeader::decode(&v).unwrap();
        assert_eq!(d, h);
        assert_eq!(&v[pos..], b"opus");
        assert!(AudioPacketHeader::decode(&v[..4]).is_none());
    }

    #[test]
    fn virtual_display_messages() {
        // 新变体追加在最后：老变体的编码不变，新变体能来回编解码
        let old = postcard::to_stdvec(&MgmtReq::SetPrivacy { on: true }).unwrap();
        assert_eq!(old[0], 10);
        let add = MgmtReq::VirtualDisplayAdd { width: 2560, height: 1440, refresh: 120 };
        let b = postcard::to_stdvec(&add).unwrap();
        assert_eq!(b[0], 11);
        match postcard::from_bytes::<MgmtReq>(&b).unwrap() {
            MgmtReq::VirtualDisplayAdd { width, height, refresh } => assert_eq!((width, height, refresh), (2560, 1440, 120)),
            other => panic!("{other:?}"),
        }
        let b = postcard::to_stdvec(&MgmtReq::VirtualDisplayRemove { display_id: 7 }).unwrap();
        assert_eq!(b[0], 12);
        assert!(matches!(postcard::from_bytes::<MgmtReq>(&b).unwrap(), MgmtReq::VirtualDisplayRemove { display_id: 7 }));
        assert_eq!(postcard::to_stdvec(&MgmtResp::Ok).unwrap()[0], 5);
        let info = DisplayInfo { id: 7, name: "DISPLAY7".into(), width: 2560, height: 1440, scale: 1.0, primary: false, is_virtual: true };
        let b = postcard::to_stdvec(&MgmtResp::Display(info)).unwrap();
        assert_eq!(b[0], 7);
        match postcard::from_bytes::<MgmtResp>(&b).unwrap() {
            MgmtResp::Display(d) => assert!(d.is_virtual && d.id == 7 && d.width == 2560),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn capability_values() {
        let caps: Vec<String> = ["display", "virtual_display", "virtual_display_max=3"].map(String::from).to_vec();
        assert_eq!(capability_value(&caps, CAP_VIRTUAL_DISPLAY_MAX), Some("3"));
        assert_eq!(capability_value(&caps, "display"), None);
        assert!(virtual_display_mode_ok(1920, 1080, 60));
        assert!(!virtual_display_mode_ok(1920, 1080, 0));
        assert!(!virtual_display_mode_ok(16384, 1080, 60));
    }

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
