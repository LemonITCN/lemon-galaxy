//! 端到端加密库：文件夹、设备别名、SSH/RDP 主机、凭据、片段、偏好。明文结构见 docs/ffi.md 2.1。

use crate::api::Api;
use crate::error::{ClientError, Result};
use galaxy_crypto::AccountSecrets;
use galaxy_proto::api::{VaultItemWire, VaultListResp, VaultPutReq, VersionResp};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SshAuth {
    Password { password: Option<String> },
    PrivateKey { credential_id: String },
    Agent,
    Interactive,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VaultData {
    Folder { name: String, color: Option<String>, expanded: bool },
    DeviceMeta { device_id: String, alias: Option<String>, tags: Vec<String>, note: String, favorite: bool },
    SshHost {
        label: String,
        host: String,
        port: u16,
        username: String,
        auth: SshAuth,
        jump_device_id: Option<String>,
        jump_host_id: Option<String>,
        tags: Vec<String>,
        note: String,
        favorite: bool,
        terminal_theme: Option<String>,
    },
    RdpHost { label: String, host: String, port: u16, username: String, password: Option<String>, domain: Option<String>, jump_device_id: Option<String>, tags: Vec<String>, note: String, favorite: bool },
    Credential { label: String, username: String, password: Option<String>, private_key: Option<String>, passphrase: Option<String> },
    Snippet { label: String, command: String, tags: Vec<String> },
    KnownHost { host: String, port: u16, key_type: String, fingerprint: String },
    Preferences { json: String },
    /// 端口映射规则：本机 127.0.0.1:local_port 经某台设备或某台 SSH 主机连到 remote_host:remote_port。
    /// 两个「经由」恰好一个有值；local_port 为 0 时每次自动选。
    PortForward {
        label: String,
        via_device_id: Option<String>,
        via_ssh_host_id: Option<String>,
        local_port: u16,
        remote_host: String,
        remote_port: u16,
        /// 主控端登录后自动开启
        auto_start: bool,
        note: String,
    },
}

impl VaultData {
    pub fn kind(&self) -> &'static str {
        match self {
            VaultData::Folder { .. } => "folder",
            VaultData::DeviceMeta { .. } => "device_meta",
            VaultData::SshHost { .. } => "ssh_host",
            VaultData::RdpHost { .. } => "rdp_host",
            VaultData::Credential { .. } => "credential",
            VaultData::Snippet { .. } => "snippet",
            VaultData::KnownHost { .. } => "known_host",
            VaultData::Preferences { .. } => "preferences",
            VaultData::PortForward { .. } => "port_forward",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VaultEntry {
    pub id: String,
    pub folder_id: Option<String>,
    pub sort: i64,
    pub data: VaultData,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub version: i64,
}

#[derive(Serialize, Deserialize)]
struct Plain {
    folder_id: Option<String>,
    sort: i64,
    data: VaultData,
}

/// 内存里的已解密加密库，按版本号增量同步。
pub struct Vault {
    user_id: u64,
    key: [u8; 32],
    pub entries: BTreeMap<String, VaultEntry>,
    pub version: i64,
}

impl Vault {
    pub fn new(user_id: u64, secrets: &AccountSecrets) -> Self {
        Self { user_id, key: secrets.vault_key, entries: BTreeMap::new(), version: 0 }
    }

    fn decrypt(&self, w: &VaultItemWire) -> Option<VaultEntry> {
        if w.deleted {
            return None;
        }
        let aad = galaxy_crypto::vault_aad(self.user_id, &w.id, &w.kind);
        let plain = galaxy_crypto::open(&self.key, &w.nonce, &w.ciphertext, &aad).ok()?;
        let p: Plain = serde_json::from_slice(&plain).ok()?;
        if p.data.kind() != w.kind {
            return None;
        }
        Some(VaultEntry { id: w.id.clone(), folder_id: p.folder_id, sort: p.sort, data: p.data, updated_at: w.updated_at, version: w.version })
    }

    /// 从服务器拉取增量。返回是否有变化。
    pub async fn sync(&mut self, api: &Api) -> Result<bool> {
        let resp: VaultListResp = api.get(&format!("/vault?since={}", self.version)).await?;
        let changed = !resp.items.is_empty();
        for w in resp.items {
            match self.decrypt(&w) {
                Some(e) => {
                    self.entries.insert(w.id.clone(), e);
                }
                None => {
                    self.entries.remove(&w.id);
                }
            }
        }
        self.version = self.version.max(resp.version);
        Ok(changed)
    }

    /// 保存条目（id 为空则新建）。版本冲突时先同步再重试一次。
    pub async fn save(&mut self, api: &Api, mut e: VaultEntry) -> Result<VaultEntry> {
        if e.id.is_empty() {
            e.id = uuid::Uuid::new_v4().simple().to_string();
            e.version = 0;
        }
        for attempt in 0..2 {
            let base = self.entries.get(&e.id).map(|x| x.version).unwrap_or(0);
            let kind = e.data.kind();
            let plain = serde_json::to_vec(&Plain { folder_id: e.folder_id.clone(), sort: e.sort, data: e.data.clone() }).map_err(|x| ClientError::Other(x.to_string()))?;
            let (nonce, ct) = galaxy_crypto::seal(&self.key, &plain, &galaxy_crypto::vault_aad(self.user_id, &e.id, kind));
            let r: Result<VersionResp> = api.put(&format!("/vault/{}", e.id), &VaultPutReq { kind: kind.into(), ciphertext: ct, nonce, base_version: base }).await;
            match r {
                Ok(v) => {
                    e.version = v.version;
                    e.updated_at = galaxy_proto::now_ms();
                    self.entries.insert(e.id.clone(), e.clone());
                    return Ok(e);
                }
                Err(ClientError::Auth { code, .. }) if code == "conflict" && attempt == 0 => {
                    self.sync(api).await?;
                }
                Err(x) => return Err(x),
            }
        }
        Err(ClientError::Other("保存冲突".into()))
    }

    pub async fn delete(&mut self, api: &Api, id: &str) -> Result<()> {
        let base = self.entries.get(id).map(|x| x.version).unwrap_or(0);
        let _: VersionResp = api.delete(&format!("/vault/{id}?base_version={base}")).await?;
        self.entries.remove(id);
        // 删除文件夹时把子项移到上一级
        Ok(())
    }

    /// 本地缓存（用设备包裹密钥加密），离线时也能看到设备列表和主机。
    pub fn to_cache(&self, wrap_key: &[u8; 32]) -> Vec<u8> {
        let plain = serde_json::to_vec(&(self.version, &self.entries)).unwrap_or_default();
        let (mut n, c) = galaxy_crypto::seal(wrap_key, &plain, b"lemon-galaxy/vault-cache/v1");
        n.extend_from_slice(&c);
        n
    }

    pub fn load_cache(&mut self, wrap_key: &[u8; 32], blob: &[u8]) {
        if blob.len() < 24 {
            return;
        }
        if let Ok(plain) = galaxy_crypto::open(wrap_key, &blob[..24], &blob[24..], b"lemon-galaxy/vault-cache/v1") {
            if let Ok((v, e)) = serde_json::from_slice::<(i64, BTreeMap<String, VaultEntry>)>(&plain) {
                self.version = v;
                self.entries = e;
            }
        }
    }
}
