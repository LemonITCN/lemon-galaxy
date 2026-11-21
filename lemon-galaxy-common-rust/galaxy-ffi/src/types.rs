//! 导出给原生界面的记录与枚举（与 docs/ffi.md 对应）。

use galaxy_proto as p;

// ---------------- 基础枚举 ----------------

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Macos,
    Windows,
    Linux,
    Android,
    Ios,
    Harmonyos,
}

impl From<p::Platform> for Platform {
    fn from(x: p::Platform) -> Self {
        match x {
            p::Platform::Macos => Self::Macos,
            p::Platform::Windows => Self::Windows,
            p::Platform::Linux => Self::Linux,
            p::Platform::Android => Self::Android,
            p::Platform::Ios => Self::Ios,
            p::Platform::Harmonyos => Self::Harmonyos,
        }
    }
}
impl From<Platform> for p::Platform {
    fn from(x: Platform) -> Self {
        match x {
            Platform::Macos => Self::Macos,
            Platform::Windows => Self::Windows,
            Platform::Linux => Self::Linux,
            Platform::Android => Self::Android,
            Platform::Ios => Self::Ios,
            Platform::Harmonyos => Self::Harmonyos,
        }
    }
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Control,
    Worker,
    Mobile,
}

impl From<p::DeviceKind> for DeviceKind {
    fn from(x: p::DeviceKind) -> Self {
        match x {
            p::DeviceKind::Control => Self::Control,
            p::DeviceKind::Worker => Self::Worker,
            p::DeviceKind::Mobile => Self::Mobile,
        }
    }
}
impl From<DeviceKind> for p::DeviceKind {
    fn from(x: DeviceKind) -> Self {
        match x {
            DeviceKind::Control => Self::Control,
            DeviceKind::Worker => Self::Worker,
            DeviceKind::Mobile => Self::Mobile,
        }
    }
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AiMode {
    Off,
    ReadOnly,
    ConfirmAll,
    ConfirmDangerous,
    FullTrust,
}

impl From<p::AiMode> for AiMode {
    fn from(x: p::AiMode) -> Self {
        match x {
            p::AiMode::Off => Self::Off,
            p::AiMode::ReadOnly => Self::ReadOnly,
            p::AiMode::ConfirmAll => Self::ConfirmAll,
            p::AiMode::ConfirmDangerous => Self::ConfirmDangerous,
            p::AiMode::FullTrust => Self::FullTrust,
        }
    }
}
impl From<AiMode> for p::AiMode {
    fn from(x: AiMode) -> Self {
        match x {
            AiMode::Off => Self::Off,
            AiMode::ReadOnly => Self::ReadOnly,
            AiMode::ConfirmAll => Self::ConfirmAll,
            AiMode::ConfirmDangerous => Self::ConfirmDangerous,
            AiMode::FullTrust => Self::FullTrust,
        }
    }
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathKind {
    Connecting,
    Lan,
    Direct,
    RelayUdp,
    RelayTcp,
    Closed,
}

impl From<p::PathKind> for PathKind {
    fn from(x: p::PathKind) -> Self {
        match x {
            p::PathKind::Connecting => Self::Connecting,
            p::PathKind::Lan => Self::Lan,
            p::PathKind::Direct => Self::Direct,
            p::PathKind::RelayUdp => Self::RelayUdp,
            p::PathKind::RelayTcp => Self::RelayTcp,
            p::PathKind::Closed => Self::Closed,
        }
    }
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientState {
    LoggedOut,
    Connecting,
    Online,
    Offline,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginResult {
    Success,
    NeedTotp,
    MustChangePassword,
}

// ---------------- 账号与设备 ----------------

#[derive(uniffi::Record, Clone, Debug)]
pub struct CoreConfig {
    pub data_dir: String,
    pub cache_dir: String,
    pub log_level: String,
    pub platform: Platform,
    pub device_kind: DeviceKind,
    pub app_version: String,
    pub os_version: String,
    pub hostname: String,
    pub locale: String,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct UserInfo {
    pub id: u64,
    pub username: String,
    pub display_name: String,
    pub ai_enabled: bool,
    pub totp_enabled: bool,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ServerInfo {
    pub address: String,
    pub name: String,
    pub version: String,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct DeviceInfo {
    pub id: String,
    pub kind: DeviceKind,
    pub platform: Platform,
    pub hostname: String,
    pub os_version: String,
    pub arch: String,
    pub app_version: String,
    pub online: bool,
    pub lan_seen: bool,
    pub last_seen_at: i64,
    pub is_self: bool,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct TotpSetup {
    pub secret: String,
    pub otpauth_uri: String,
}

// ---------------- 加密库 ----------------

#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum SshAuth {
    Password { password: Option<String> },
    PrivateKey { credential_id: String },
    Agent,
    Interactive,
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum VaultData {
    Folder { name: String, color: Option<String>, expanded: bool },
    DeviceMeta { device_id: String, alias: Option<String>, tags: Vec<String>, note: String, favorite: bool },
    SshHost { label: String, host: String, port: u16, username: String, auth: SshAuth, jump_device_id: Option<String>, jump_host_id: Option<String>, tags: Vec<String>, note: String, favorite: bool, terminal_theme: Option<String> },
    RdpHost { label: String, host: String, port: u16, username: String, password: Option<String>, domain: Option<String>, jump_device_id: Option<String>, tags: Vec<String>, note: String, favorite: bool },
    Credential { label: String, username: String, password: Option<String>, private_key: Option<String>, passphrase: Option<String> },
    Snippet { label: String, command: String, tags: Vec<String> },
    KnownHost { host: String, port: u16, key_type: String, fingerprint: String },
    Preferences { json: String },
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct VaultEntry {
    pub id: String,
    pub folder_id: Option<String>,
    pub sort: i64,
    pub data: VaultData,
    pub updated_at: i64,
}

use galaxy_client::vault as v;

impl From<v::SshAuth> for SshAuth {
    fn from(x: v::SshAuth) -> Self {
        match x {
            v::SshAuth::Password { password } => Self::Password { password },
            v::SshAuth::PrivateKey { credential_id } => Self::PrivateKey { credential_id },
            v::SshAuth::Agent => Self::Agent,
            v::SshAuth::Interactive => Self::Interactive,
        }
    }
}
impl From<SshAuth> for v::SshAuth {
    fn from(x: SshAuth) -> Self {
        match x {
            SshAuth::Password { password } => Self::Password { password },
            SshAuth::PrivateKey { credential_id } => Self::PrivateKey { credential_id },
            SshAuth::Agent => Self::Agent,
            SshAuth::Interactive => Self::Interactive,
        }
    }
}

impl From<v::VaultData> for VaultData {
    fn from(x: v::VaultData) -> Self {
        use v::VaultData as V;
        match x {
            V::Folder { name, color, expanded } => Self::Folder { name, color, expanded },
            V::DeviceMeta { device_id, alias, tags, note, favorite } => Self::DeviceMeta { device_id, alias, tags, note, favorite },
            V::SshHost { label, host, port, username, auth, jump_device_id, jump_host_id, tags, note, favorite, terminal_theme } => {
                Self::SshHost { label, host, port, username, auth: auth.into(), jump_device_id, jump_host_id, tags, note, favorite, terminal_theme }
            }
            V::RdpHost { label, host, port, username, password, domain, jump_device_id, tags, note, favorite } => Self::RdpHost { label, host, port, username, password, domain, jump_device_id, tags, note, favorite },
            V::Credential { label, username, password, private_key, passphrase } => Self::Credential { label, username, password, private_key, passphrase },
            V::Snippet { label, command, tags } => Self::Snippet { label, command, tags },
            V::KnownHost { host, port, key_type, fingerprint } => Self::KnownHost { host, port, key_type, fingerprint },
            V::Preferences { json } => Self::Preferences { json },
        }
    }
}
impl From<VaultData> for v::VaultData {
    fn from(x: VaultData) -> Self {
        use VaultData as V;
        match x {
            V::Folder { name, color, expanded } => Self::Folder { name, color, expanded },
            V::DeviceMeta { device_id, alias, tags, note, favorite } => Self::DeviceMeta { device_id, alias, tags, note, favorite },
            V::SshHost { label, host, port, username, auth, jump_device_id, jump_host_id, tags, note, favorite, terminal_theme } => {
                Self::SshHost { label, host, port, username, auth: auth.into(), jump_device_id, jump_host_id, tags, note, favorite, terminal_theme }
            }
            V::RdpHost { label, host, port, username, password, domain, jump_device_id, tags, note, favorite } => Self::RdpHost { label, host, port, username, password, domain, jump_device_id, tags, note, favorite },
            V::Credential { label, username, password, private_key, passphrase } => Self::Credential { label, username, password, private_key, passphrase },
            V::Snippet { label, command, tags } => Self::Snippet { label, command, tags },
            V::KnownHost { host, port, key_type, fingerprint } => Self::KnownHost { host, port, key_type, fingerprint },
            V::Preferences { json } => Self::Preferences { json },
        }
    }
}

impl From<v::VaultEntry> for VaultEntry {
    fn from(e: v::VaultEntry) -> Self {
        Self { id: e.id, folder_id: e.folder_id, sort: e.sort, data: e.data.into(), updated_at: e.updated_at }
    }
}

// ---------------- 会话 ----------------

#[derive(uniffi::Record, Clone, Debug)]
pub struct Permissions {
    pub view: bool,
    pub control: bool,
    pub files: bool,
    pub terminal: bool,
    pub clipboard: bool,
    pub audio: bool,
    pub ai_cap: AiMode,
}

impl From<p::Permissions> for Permissions {
    fn from(x: p::Permissions) -> Self {
        Self { view: x.view, control: x.control, files: x.files, terminal: x.terminal, clipboard: x.clipboard, audio: x.audio, ai_cap: x.ai_cap.into() }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct PathInfo {
    pub kind: PathKind,
    pub rtt_ms: u32,
    pub remote_addr: String,
}

impl From<galaxy_net::PathInfo> for PathInfo {
    fn from(x: galaxy_net::PathInfo) -> Self {
        Self { kind: x.kind.into(), rtt_ms: x.rtt_ms, remote_addr: x.remote }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct PeerInfo {
    pub device_id: String,
    pub hostname: String,
    pub platform: Platform,
    pub os_version: String,
    pub arch: String,
    pub system_user: String,
    pub shell: String,
    pub permissions: Permissions,
    pub capabilities: Vec<String>,
    pub path: PathInfo,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct TerminalOptions {
    pub attach_session: Option<String>,
    pub shell: Option<String>,
    pub cols: u16,
    pub rows: u16,
    pub env: Vec<EnvVar>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct TerminalSessionInfo {
    pub id: String,
    pub title: String,
    pub created_at: i64,
    pub attached: bool,
}

impl From<p::peer::TerminalSessionInfo> for TerminalSessionInfo {
    fn from(x: p::peer::TerminalSessionInfo) -> Self {
        Self { id: x.id, title: x.title, created_at: x.created_at, attached: x.attached }
    }
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    File,
    Dir,
    Symlink,
    Other,
}

#[derive(uniffi::Record, Clone, Debug)]
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

impl From<p::peer::FileEntry> for FileEntry {
    fn from(x: p::peer::FileEntry) -> Self {
        Self {
            name: x.name,
            path: x.path,
            kind: match x.kind {
                p::peer::FileKind::File => FileKind::File,
                p::peer::FileKind::Dir => FileKind::Dir,
                p::peer::FileKind::Symlink => FileKind::Symlink,
                p::peer::FileKind::Other => FileKind::Other,
            },
            size: x.size,
            mode: x.mode,
            modified_at: x.modified_at,
            owner: x.owner,
            group: x.group,
            link_target: x.link_target,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ExecRequest {
    pub command: String,
    pub shell: Option<String>,
    pub cwd: Option<String>,
    pub timeout_ms: u64,
    pub max_output: u64,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ExecResult {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub truncated: bool,
    pub duration_ms: u64,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    H265,
}

impl From<p::peer::VideoCodec> for VideoCodec {
    fn from(x: p::peer::VideoCodec) -> Self {
        match x {
            p::peer::VideoCodec::H264 => Self::H264,
            p::peer::VideoCodec::H265 => Self::H265,
        }
    }
}
impl From<VideoCodec> for p::peer::VideoCodec {
    fn from(x: VideoCodec) -> Self {
        match x {
            VideoCodec::H264 => Self::H264,
            VideoCodec::H265 => Self::H265,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct DisplayInfo {
    pub id: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub primary: bool,
    pub is_virtual: bool,
}

impl From<p::peer::DisplayInfo> for DisplayInfo {
    fn from(x: p::peer::DisplayInfo) -> Self {
        Self { id: x.id, name: x.name, width: x.width, height: x.height, scale: x.scale, primary: x.primary, is_virtual: x.is_virtual }
    }
}
impl From<DisplayInfo> for p::peer::DisplayInfo {
    fn from(x: DisplayInfo) -> Self {
        Self { id: x.id, name: x.name, width: x.width, height: x.height, scale: x.scale, primary: x.primary, is_virtual: x.is_virtual }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct DisplayOptions {
    pub display_id: u32,
    pub max_fps: u32,
    pub bitrate_kbps: u32,
    pub codecs: Vec<VideoCodec>,
    pub view_only: bool,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ImageData {
    pub mime: String,
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl From<p::peer::ImageData> for ImageData {
    fn from(x: p::peer::ImageData) -> Self {
        Self { mime: x.mime, data: x.data, width: x.width, height: x.height }
    }
}
impl From<ImageData> for p::peer::ImageData {
    fn from(x: ImageData) -> Self {
        Self { mime: x.mime, data: x.data, width: x.width, height: x.height }
    }
}

#[derive(uniffi::Enum, Clone, Debug)]
pub enum ClipboardData {
    Text { text: String },
    Image { image: ImageData },
    Files { paths: Vec<String> },
}

impl From<p::peer::ClipboardData> for ClipboardData {
    fn from(x: p::peer::ClipboardData) -> Self {
        match x {
            p::peer::ClipboardData::Text(t) => Self::Text { text: t },
            p::peer::ClipboardData::Image(i) => Self::Image { image: i.into() },
            p::peer::ClipboardData::Files(f) => Self::Files { paths: f },
        }
    }
}
impl From<ClipboardData> for p::peer::ClipboardData {
    fn from(x: ClipboardData) -> Self {
        match x {
            ClipboardData::Text { text } => Self::Text(text),
            ClipboardData::Image { image } => Self::Image(image.into()),
            ClipboardData::Files { paths } => Self::Files(paths),
        }
    }
}

/// 鼠标按键（避免与 InputEvent 的变体同名）。
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

/// 特殊组合键（避免与 InputEvent 的变体同名）。
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComboKey {
    CtrlAltDel,
    LockScreen,
    AltTab,
    WinKey,
    TaskManager,
    ShowDesktop,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchPhase {
    Began,
    Moved,
    Ended,
    Cancelled,
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum InputEvent {
    MouseMove { x: i32, y: i32 },
    MouseButton { button: PointerButton, down: bool, x: i32, y: i32 },
    Wheel { dx: i32, dy: i32 },
    Key { hid_usage: u32, down: bool },
    Text { text: String },
    Touch { id: u32, phase: TouchPhase, x: i32, y: i32 },
    SpecialCombo { combo: ComboKey },
}

impl From<InputEvent> for p::peer::InputEvent {
    fn from(x: InputEvent) -> Self {
        use p::peer::InputEvent as I;
        match x {
            InputEvent::MouseMove { x, y } => I::MouseMove { x, y },
            InputEvent::MouseButton { button, down, x, y } => I::MouseButton {
                button: match button {
                    PointerButton::Left => p::peer::MouseButton::Left,
                    PointerButton::Right => p::peer::MouseButton::Right,
                    PointerButton::Middle => p::peer::MouseButton::Middle,
                    PointerButton::Back => p::peer::MouseButton::Back,
                    PointerButton::Forward => p::peer::MouseButton::Forward,
                },
                down,
                x,
                y,
            },
            InputEvent::Wheel { dx, dy } => I::Wheel { dx, dy },
            InputEvent::Key { hid_usage, down } => I::Key { hid_usage, down },
            InputEvent::Text { text } => I::Text { text },
            InputEvent::Touch { id, phase, x, y } => I::Touch {
                id,
                phase: match phase {
                    TouchPhase::Began => p::peer::TouchPhase::Began,
                    TouchPhase::Moved => p::peer::TouchPhase::Moved,
                    TouchPhase::Ended => p::peer::TouchPhase::Ended,
                    TouchPhase::Cancelled => p::peer::TouchPhase::Cancelled,
                },
                x,
                y,
            },
            InputEvent::SpecialCombo { combo } => I::SpecialCombo(match combo {
                ComboKey::CtrlAltDel => p::peer::SpecialCombo::CtrlAltDel,
                ComboKey::LockScreen => p::peer::SpecialCombo::LockScreen,
                ComboKey::AltTab => p::peer::SpecialCombo::AltTab,
                ComboKey::WinKey => p::peer::SpecialCombo::WinKey,
                ComboKey::TaskManager => p::peer::SpecialCombo::TaskManager,
                ComboKey::ShowDesktop => p::peer::SpecialCombo::ShowDesktop,
            }),
        }
    }
}

impl From<p::peer::InputEvent> for InputEvent {
    fn from(x: p::peer::InputEvent) -> Self {
        use p::peer::InputEvent as I;
        match x {
            I::MouseMove { x, y } => Self::MouseMove { x, y },
            I::MouseButton { button, down, x, y } => Self::MouseButton {
                button: match button {
                    p::peer::MouseButton::Left => PointerButton::Left,
                    p::peer::MouseButton::Right => PointerButton::Right,
                    p::peer::MouseButton::Middle => PointerButton::Middle,
                    p::peer::MouseButton::Back => PointerButton::Back,
                    p::peer::MouseButton::Forward => PointerButton::Forward,
                },
                down,
                x,
                y,
            },
            I::Wheel { dx, dy } => Self::Wheel { dx, dy },
            I::Key { hid_usage, down } => Self::Key { hid_usage, down },
            I::Text { text } => Self::Text { text },
            I::Touch { id, phase, x, y } => Self::Touch {
                id,
                phase: match phase {
                    p::peer::TouchPhase::Began => TouchPhase::Began,
                    p::peer::TouchPhase::Moved => TouchPhase::Moved,
                    p::peer::TouchPhase::Ended => TouchPhase::Ended,
                    p::peer::TouchPhase::Cancelled => TouchPhase::Cancelled,
                },
                x,
                y,
            },
            I::SpecialCombo(c) => Self::SpecialCombo {
                combo: match c {
                    p::peer::SpecialCombo::CtrlAltDel => ComboKey::CtrlAltDel,
                    p::peer::SpecialCombo::LockScreen => ComboKey::LockScreen,
                    p::peer::SpecialCombo::AltTab => ComboKey::AltTab,
                    p::peer::SpecialCombo::WinKey => ComboKey::WinKey,
                    p::peer::SpecialCombo::TaskManager => ComboKey::TaskManager,
                    p::peer::SpecialCombo::ShowDesktop => ComboKey::ShowDesktop,
                },
            },
        }
    }
}

// ---------------- AI ----------------

#[derive(uniffi::Record, Clone, Debug)]
pub struct AiModel {
    pub provider_id: u64,
    pub provider_name: String,
    pub id: String,
    pub name: String,
    pub vision: bool,
    pub tools: bool,
    pub context_length: u32,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct AiModels {
    pub enabled: bool,
    pub max_mode: AiMode,
    pub items: Vec<AiModel>,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AgentTargetKind {
    Device,
    SshHost,
}

#[derive(uniffi::Record, Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AgentTarget {
    pub kind: AgentTargetKind,
    pub id: String,
}

#[derive(uniffi::Record, Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ConversationConfig {
    pub provider_id: u64,
    pub model_id: String,
    pub targets: Vec<AgentTarget>,
    pub mode: AiMode,
    pub plan_first: bool,
    /// 完全信任的时限（分钟），None 为不限
    pub trust_minutes: Option<u32>,
    pub title: Option<String>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub model_id: String,
    pub target_names: Vec<String>,
    pub updated_at: i64,
    /// 创建时的配置（模型、目标、档位）；老记录没有
    pub config: Option<ConversationConfig>,
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum ApprovalDecision {
    AllowOnce,
    AllowSimilar,
    Deny { reason: Option<String> },
    EditAndRun { command: String },
}

impl From<ApprovalDecision> for galaxy_agent::agent::ApprovalDecision {
    fn from(x: ApprovalDecision) -> Self {
        match x {
            ApprovalDecision::AllowOnce => Self::AllowOnce,
            ApprovalDecision::AllowSimilar => Self::AllowSimilar,
            ApprovalDecision::Deny { reason } => Self::Deny { reason },
            ApprovalDecision::EditAndRun { command } => Self::EditAndRun { command },
        }
    }
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiskLevel {
    Read,
    Normal,
    Dangerous,
    Unknown,
    Forbidden,
}

impl From<galaxy_agent::risk::Risk> for RiskLevel {
    fn from(x: galaxy_agent::risk::Risk) -> Self {
        use galaxy_agent::risk::Risk as R;
        match x {
            R::Read => Self::Read,
            R::Normal => Self::Normal,
            R::Dangerous => Self::Dangerous,
            R::Unknown => Self::Unknown,
            R::Forbidden => Self::Forbidden,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ToolCallView {
    pub call_id: String,
    pub tool: String,
    pub target_id: String,
    pub target_name: String,
    pub summary: String,
    pub detail: String,
    pub risk: RiskLevel,
    pub risk_reasons: Vec<String>,
    pub model_reason: String,
    pub image: Option<ImageData>,
}

impl From<galaxy_agent::agent::ToolCallView> for ToolCallView {
    fn from(x: galaxy_agent::agent::ToolCallView) -> Self {
        Self {
            call_id: x.call_id,
            tool: x.tool,
            target_id: x.target_id,
            target_name: x.target_name,
            summary: x.summary,
            detail: x.detail,
            risk: x.risk.into(),
            risk_reasons: x.risk_reasons,
            model_reason: x.model_reason,
            image: x.image.map(Into::into),
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ToolResultView {
    pub ok: bool,
    pub summary: String,
    pub output: String,
    pub image: Option<ImageData>,
    pub denied: bool,
}

impl From<galaxy_agent::agent::ToolResultView> for ToolResultView {
    fn from(x: galaxy_agent::agent::ToolResultView) -> Self {
        Self { ok: x.ok, summary: x.summary, output: x.output, image: x.image.map(Into::into), denied: x.denied }
    }
}

#[derive(uniffi::Enum, Clone, Debug)]
pub enum AgentEvent {
    AssistantDelta { message_id: String, text: String },
    AssistantDone { message_id: String },
    Thinking { message_id: String, text: String },
    ToolProposed { call: ToolCallView, needs_approval: bool },
    ToolStarted { call_id: String },
    ToolOutput { call_id: String, text: String },
    ToolFinished { call_id: String, result: ToolResultView },
    AskUser { question_id: String, question: String, options: Vec<String> },
    SecretRequested { request_id: String, prompt: String, target_id: String },
    Usage { prompt_tokens: u64, completion_tokens: u64 },
    Status { running: bool, step: u32 },
    ModeChanged { mode: AiMode, effective: AiMode, until_ms: Option<i64> },
    Error { message: String },
}

impl From<galaxy_agent::agent::AgentEvent> for AgentEvent {
    fn from(e: galaxy_agent::agent::AgentEvent) -> Self {
        use galaxy_agent::agent::AgentEvent as E;
        match e {
            E::AssistantDelta { message_id, text } => Self::AssistantDelta { message_id, text },
            E::AssistantDone { message_id } => Self::AssistantDone { message_id },
            E::Thinking { message_id, text } => Self::Thinking { message_id, text },
            E::ToolProposed { call, needs_approval } => Self::ToolProposed { call: call.into(), needs_approval },
            E::ToolStarted { call_id } => Self::ToolStarted { call_id },
            E::ToolOutput { call_id, text } => Self::ToolOutput { call_id, text },
            E::ToolFinished { call_id, result } => Self::ToolFinished { call_id, result: result.into() },
            E::AskUser { question_id, question, options } => Self::AskUser { question_id, question, options },
            E::SecretRequested { request_id, prompt, target_id } => Self::SecretRequested { request_id, prompt, target_id },
            E::Usage { prompt_tokens, completion_tokens } => Self::Usage { prompt_tokens, completion_tokens },
            E::Status { running, step } => Self::Status { running, step },
            E::ModeChanged { mode, effective, until_ms } => Self::ModeChanged { mode: mode.into(), effective: effective.into(), until_ms },
            E::Error { message } => Self::Error { message },
        }
    }
}

#[derive(uniffi::Enum, Clone, Debug)]
pub enum AgentMessage {
    User { text: String },
    Assistant { id: String, text: String, thinking: String },
    Tool { call: ToolCallView, result: Option<ToolResultView> },
}

impl From<galaxy_agent::agent::AgentMessage> for AgentMessage {
    fn from(m: galaxy_agent::agent::AgentMessage) -> Self {
        use galaxy_agent::agent::AgentMessage as M;
        match m {
            M::User { text } => Self::User { text },
            M::Assistant { id, text, thinking } => Self::Assistant { id, text, thinking },
            M::Tool { call, result } => Self::Tool { call: call.into(), result: result.map(Into::into) },
        }
    }
}

// ---------------- 本机被控 ----------------

#[derive(uniffi::Record, Clone, Debug)]
pub struct HostPolicy {
    pub system_user: String,
    pub view: bool,
    pub control: bool,
    pub files: bool,
    pub terminal: bool,
    pub clipboard: bool,
    pub audio: bool,
    pub ai_cap: AiMode,
    pub notify: bool,
}

impl From<galaxy_host::HostPolicy> for HostPolicy {
    fn from(x: galaxy_host::HostPolicy) -> Self {
        Self { system_user: x.system_user, view: x.view, control: x.control, files: x.files, terminal: x.terminal, clipboard: x.clipboard, audio: x.audio, ai_cap: x.ai_cap.into(), notify: x.notify }
    }
}
impl From<HostPolicy> for galaxy_host::HostPolicy {
    fn from(x: HostPolicy) -> Self {
        Self { system_user: x.system_user, view: x.view, control: x.control, files: x.files, terminal: x.terminal, clipboard: x.clipboard, audio: x.audio, ai_cap: x.ai_cap.into(), notify: x.notify }
    }
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountStatus {
    Active,
    NeedsRelogin,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct HostAccount {
    pub user_id: u64,
    pub username: String,
    pub display_name: String,
    pub status: AccountStatus,
    pub policy: HostPolicy,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnMode {
    Control,
    Assist,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct HostSessionInfo {
    pub id: String,
    pub from_user: String,
    pub from_device: String,
    pub mode: ConnMode,
    pub path: PathKind,
    pub started_at: i64,
    pub ai_active: bool,
}

impl From<galaxy_host::service::SessionInfo> for HostSessionInfo {
    fn from(x: galaxy_host::service::SessionInfo) -> Self {
        Self {
            id: x.id,
            from_user: x.from_user,
            from_device: x.from_device,
            mode: match x.mode {
                p::ConnMode::Control => ConnMode::Control,
                p::ConnMode::Assist => ConnMode::Assist,
            },
            path: x.path.into(),
            started_at: x.started_at,
            ai_active: x.ai_active,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct AssistInfo {
    pub device_code: String,
    pub code: String,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct HostStatus {
    pub online: bool,
    pub remote_enabled: bool,
    pub device_id: String,
    pub public_ip: String,
    pub accounts: Vec<HostAccount>,
    pub sessions: Vec<HostSessionInfo>,
    pub assist: Option<AssistInfo>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct HostEvent {
    pub at: i64,
    pub session: String,
    pub user: String,
    /// connect / disconnect / terminal / file / exec / display / forward / ai / reject / ai_denied
    pub kind: String,
    pub detail: String,
    pub ai: bool,
    pub level: EventLevel,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventLevel {
    Info,
    /// 被拒绝的连接、被拦下的 AI 操作等，界面应醒目标出
    Warning,
}

impl From<galaxy_host::service::HostLogEvent> for HostEvent {
    fn from(x: galaxy_host::service::HostLogEvent) -> Self {
        let level = if matches!(x.kind.as_str(), "reject" | "ai_denied" | "auth_failed") { EventLevel::Warning } else { EventLevel::Info };
        Self { at: x.at, session: x.session, user: x.user, kind: x.kind, detail: x.detail, ai: x.ai, level }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct CaptureOptions {
    pub max_fps: u32,
    pub bitrate_kbps: u32,
    pub codecs: Vec<VideoCodec>,
}

/// 新版本信息（已验过发布签名，文件在签名的清单里）。
#[derive(uniffi::Record, Clone, Debug)]
pub struct UpdateInfo {
    pub version: String,
    pub file_name: String,
    pub size: u64,
    pub sha256: String,
    /// 服务器上的下载路径（交给 download_update）
    pub url: String,
    pub notes: String,
}

impl From<galaxy_client::update::UpdateInfo> for UpdateInfo {
    fn from(i: galaxy_client::update::UpdateInfo) -> Self {
        Self { version: i.version, file_name: i.file_name, size: i.size, sha256: i.sha256, url: i.url, notes: i.notes }
    }
}
