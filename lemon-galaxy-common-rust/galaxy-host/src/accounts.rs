//! 被控端本地状态：登录的账号、每个账号的本机权限（只能在本机修改）。

use galaxy_proto::cert::SignedBlob;
use galaxy_proto::{AiMode, DeviceId, Permissions};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HostPolicy {
    /// 终端、文件、命令以哪个系统用户运行
    pub system_user: String,
    pub view: bool,
    pub control: bool,
    pub files: bool,
    pub terminal: bool,
    pub clipboard: bool,
    pub audio: bool,
    /// 本机给这个账号的 AI 上限
    pub ai_cap: AiMode,
    /// 被控时显示提示
    #[serde(default = "yes")]
    pub notify: bool,
}

fn yes() -> bool {
    true
}

impl HostPolicy {
    pub fn default_for(system_user: &str) -> Self {
        Self { system_user: system_user.into(), view: true, control: true, files: true, terminal: true, clipboard: true, audio: true, ai_cap: AiMode::FullTrust, notify: true }
    }
    pub fn permissions(&self) -> Permissions {
        Permissions { view: self.view, control: self.control, files: self.files, terminal: self.terminal, clipboard: self.clipboard, audio: self.audio, ai_cap: self.ai_cap }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccountStatus {
    Active,
    NeedsRelogin,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostAccount {
    pub user_id: u64,
    pub username: String,
    pub display_name: String,
    #[serde(with = "hex32")]
    pub account_pubkey: [u8; 32],
    pub epoch: u32,
    pub cert: SignedBlob,
    pub token: String,
    pub status: AccountStatus,
    pub policy: HostPolicy,
    #[serde(default)]
    pub revoked: Vec<DeviceId>,
    pub added_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct HostState {
    /// 服务器完整地址（含指纹）
    pub server: String,
    #[serde(default)]
    pub server_sign_pubkey: Vec<u8>,
    pub accounts: Vec<HostAccount>,
    #[serde(default = "yes")]
    pub remote_enabled: bool,
    /// 自定义的临时协助验证码（为空则每次随机）
    #[serde(default)]
    pub assist_code: Option<String>,
    /// 服务模式下自动安装新版本（签名校验通过才装）
    #[serde(default = "yes")]
    pub auto_update: bool,
}

impl HostState {
    pub fn load(path: &Path) -> Self {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| Self { remote_enabled: true, auto_update: true, ..Default::default() })
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(tmp, path)
    }

    pub fn active(&self) -> impl Iterator<Item = &HostAccount> {
        self.accounts.iter().filter(|a| a.status == AccountStatus::Active)
    }
}

mod hex32 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(v))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let s = String::deserialize(d)?;
        let v = hex::decode(s).map_err(serde::de::Error::custom)?;
        v.try_into().map_err(|_| serde::de::Error::custom("len"))
    }
}
