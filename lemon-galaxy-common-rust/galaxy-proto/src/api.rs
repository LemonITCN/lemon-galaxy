//! 客户端 HTTP 接口（docs/api.md 第二部分）的请求与响应。

use crate::{AiMode, DeviceId, DeviceKind, Platform};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub code: String,
    #[serde(default)]
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: ErrorInfo,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OkBody {
    pub ok: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerInfoResp {
    pub server_name: String,
    pub version: String,
    #[serde(with = "crate::b64")]
    pub server_sign_pubkey: Vec<u8>,
    pub min_client_version: String,
    pub features: Vec<String>,
    pub udp_port: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceRegistration {
    pub device_id: DeviceId,
    #[serde(with = "crate::b64")]
    pub device_pubkey: Vec<u8>,
    pub kind: DeviceKind,
    pub platform: Platform,
    #[serde(default)]
    pub os_version: String,
    #[serde(default)]
    pub arch: String,
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub app_version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoginStartReq {
    pub username: String,
    #[serde(with = "crate::b64")]
    pub opaque_request: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoginStartResp {
    pub login_id: String,
    #[serde(with = "crate::b64")]
    pub opaque_response: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoginFinishReq {
    pub login_id: String,
    #[serde(with = "crate::b64")]
    pub opaque_finalization: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub totp_code: Option<String>,
    pub device: DeviceRegistration,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserSelf {
    pub id: u64,
    pub username: String,
    pub display_name: String,
    pub ai_enabled: bool,
    #[serde(default)]
    pub totp_enabled: bool,
}

/// 账号机密（加密后）及公钥。
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AccountMaterial {
    pub epoch: u32,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::b64::opt")]
    pub pubkey: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::b64::opt")]
    pub wrapped_secrets: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::b64::opt")]
    pub nonce: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoginFinishResp {
    pub session_token: String,
    pub user: UserSelf,
    pub must_change_password: bool,
    pub account: AccountMaterial,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PasswordStartReq {
    #[serde(with = "crate::b64")]
    pub registration_request: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PasswordStartResp {
    #[serde(with = "crate::b64")]
    pub registration_response: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PasswordFinishReq {
    #[serde(with = "crate::b64")]
    pub registration_upload: Vec<u8>,
    pub account: AccountMaterial,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CertUploadReq {
    #[serde(with = "crate::b64")]
    pub cert: Vec<u8>,
    #[serde(with = "crate::b64")]
    pub signature: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClientDevice {
    pub id: DeviceId,
    pub kind: DeviceKind,
    pub platform: Platform,
    #[serde(default)]
    pub os_version: String,
    #[serde(default)]
    pub arch: String,
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub app_version: String,
    pub online: bool,
    pub last_seen_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::b64::opt")]
    pub cert: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::b64::opt")]
    pub cert_signature: Option<Vec<u8>>,
    #[serde(default)]
    pub public_ip: String,
    #[serde(with = "crate::b64")]
    pub device_pubkey: Vec<u8>,
    /// 最近一次连上服务器时的公网 IP（离线设备也有），用来找同一网络里的设备
    #[serde(default)]
    pub last_ip: String,
    /// 网卡 MAC（远程开机用）
    #[serde(default)]
    pub macs: Vec<String>,
    /// 更新通道「组件/平台/架构」（如 worker/windowsxp/x86、control/macos/universal）：被控端连上信令时报上来，
    /// 老版本没报时服务器按系统推断
    #[serde(default)]
    pub update_channel: String,
    /// 服务器上有比 `app_version` 新的版本时是那个版本号（主控端在设备列表里标「可升级」），否则为空
    #[serde(default)]
    pub update_version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListResp<T> {
    pub items: Vec<T>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RevocationsWire {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::b64::opt")]
    pub list: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::b64::opt")]
    pub signature: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultItemWire {
    pub id: String,
    pub kind: String,
    #[serde(with = "crate::b64")]
    pub ciphertext: Vec<u8>,
    #[serde(with = "crate::b64")]
    pub nonce: Vec<u8>,
    pub version: i64,
    pub deleted: bool,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultListResp {
    pub items: Vec<VaultItemWire>,
    pub version: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultPutReq {
    pub kind: String,
    #[serde(with = "crate::b64")]
    pub ciphertext: Vec<u8>,
    #[serde(with = "crate::b64")]
    pub nonce: Vec<u8>,
    pub base_version: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VersionResp {
    pub version: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiModelInfo {
    pub provider_id: u64,
    pub provider_name: String,
    pub id: String,
    pub name: String,
    pub vision: bool,
    pub tools: bool,
    pub context_length: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiModelsResp {
    pub enabled: bool,
    pub max_mode: AiMode,
    pub items: Vec<AiModelInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TotpSetupResp {
    pub secret: String,
    pub otpauth_uri: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TotpCodeReq {
    pub code: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateLatestResp {
    pub available: bool,
    #[serde(default)]
    pub version: String,
    /// 服务器上的下载地址（相对路径，带登录令牌访问）
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub sha256: String,
    /// 这次发布的 SHA256SUMS.txt 全文与签名（base64），客户端自己验
    #[serde(default)]
    pub sums: String,
    #[serde(default)]
    pub signature: String,
    #[serde(default)]
    pub notes: String,
}

/// 账号最近的活动（登录、连接等），`GET /account/activity?limit=`。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActivityItem {
    pub at: i64,
    /// 如 user.login、connection.start、connection.end、device.register
    pub action: String,
    pub actor_name: String,
    pub target_type: String,
    pub target_id: String,
    pub target_name: String,
    pub ip: String,
    /// 原始详情（JSON 文本）
    pub detail: String,
}
