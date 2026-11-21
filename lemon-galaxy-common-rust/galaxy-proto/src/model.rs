use crate::DeviceId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    /// Windows / macOS 主控 + 被控，只能登录一个账号
    Control,
    /// 纯被控，可登录多个账号
    Worker,
    /// 移动端纯主控
    Mobile,
}

impl DeviceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Worker => "worker",
            Self::Mobile => "mobile",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "control" => Some(Self::Control),
            "worker" => Some(Self::Worker),
            "mobile" => Some(Self::Mobile),
            _ => None,
        }
    }
    /// 能否被控。
    pub fn can_host(&self) -> bool {
        !matches!(self, Self::Mobile)
    }
    /// 能否发起控制。
    pub fn can_control(&self) -> bool {
        !matches!(self, Self::Worker)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Linux,
    Macos,
    Windows,
    Android,
    Ios,
    Harmonyos,
}

impl Platform {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Windows => "windows",
            Self::Android => "android",
            Self::Ios => "ios",
            Self::Harmonyos => "harmonyos",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "linux" => Self::Linux,
            "macos" => Self::Macos,
            "windows" => Self::Windows,
            "android" => Self::Android,
            "ios" => Self::Ios,
            "harmonyos" => Self::Harmonyos,
            _ => return None,
        })
    }
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "android") {
            Self::Android
        } else if cfg!(target_os = "ios") {
            Self::Ios
        } else {
            Self::Linux
        }
    }
    pub fn has_gui(&self) -> bool {
        matches!(self, Self::Macos | Self::Windows)
    }
}

/// AI 权限档位，按从严到宽排序。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiMode {
    Off,
    ReadOnly,
    ConfirmAll,
    ConfirmDangerous,
    FullTrust,
}

impl AiMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::ReadOnly => "read_only",
            Self::ConfirmAll => "confirm_all",
            Self::ConfirmDangerous => "confirm_dangerous",
            Self::FullTrust => "full_trust",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "off" => Self::Off,
            "read_only" => Self::ReadOnly,
            "confirm_all" => Self::ConfirmAll,
            "confirm_dangerous" => Self::ConfirmDangerous,
            "full_trust" => Self::FullTrust,
            _ => return None,
        })
    }
    /// 多个上限里取最严的。
    pub fn strictest(modes: &[AiMode]) -> AiMode {
        modes.iter().copied().min().unwrap_or(AiMode::Off)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathKind {
    Connecting,
    Lan,
    Direct,
    RelayUdp,
    RelayTcp,
    Closed,
}

impl PathKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Lan => "lan",
            Self::Direct => "direct",
            Self::RelayUdp => "relay_udp",
            Self::RelayTcp => "relay_tcp",
            Self::Closed => "closed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnMode {
    Control,
    Assist,
}

/// 被控端基本信息。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostInfo {
    pub device_id: DeviceId,
    pub hostname: String,
    pub platform: Platform,
    pub os_version: String,
    pub arch: String,
    pub app_version: String,
    /// 远端操作所用的系统用户名（终端、文件、命令）
    pub system_user: String,
    /// 系统用户的默认 shell
    pub shell: String,
}

/// 被控端给某个账号的权限。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permissions {
    pub view: bool,
    pub control: bool,
    pub files: bool,
    pub terminal: bool,
    pub clipboard: bool,
    pub audio: bool,
    pub ai_cap: AiMode,
}

impl Permissions {
    pub fn full() -> Self {
        Self {
            view: true,
            control: true,
            files: true,
            terminal: true,
            clipboard: true,
            audio: true,
            ai_cap: AiMode::ConfirmDangerous,
        }
    }
    pub fn none() -> Self {
        Self {
            view: false,
            control: false,
            files: false,
            terminal: false,
            clipboard: false,
            audio: false,
            ai_cap: AiMode::Off,
        }
    }
}

impl Default for Permissions {
    fn default() -> Self {
        Self::full()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserBrief {
    pub id: u64,
    pub username: String,
    pub display_name: String,
}
