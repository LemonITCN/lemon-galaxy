//! 设备证书、连接票据、吊销列表。签名与校验在 galaxy-crypto。

use crate::{ConnId, ConnMode, DeviceId, DeviceKind};
use serde::{Deserialize, Serialize};

pub const DEVICE_CERT_DOMAIN: &[u8] = b"lemon-galaxy/device-cert/v1";
pub const CONN_TICKET_DOMAIN: &[u8] = b"lemon-galaxy/conn-ticket/v1";
pub const REVOCATION_DOMAIN: &[u8] = b"lemon-galaxy/revocations/v1";
pub const SESSION_AUTH_DOMAIN: &[u8] = b"lemon-galaxy/session-auth/v1";
pub const WS_AUTH_DOMAIN: &[u8] = b"lemon-galaxy/ws-auth/v1";
pub const TLS_EXPORTER_LABEL: &[u8] = b"lemon-galaxy/session/v1";

/// 账号签名密钥签发给设备的证书。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCert {
    pub v: u8,
    pub account_id: u64,
    pub account_epoch: u32,
    pub device_id: DeviceId,
    pub device_pubkey: [u8; 32],
    pub kind: DeviceKind,
    pub issued_at: i64,
}

impl DeviceCert {
    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_stdvec(self).expect("postcard")
    }
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        postcard::from_bytes(b).ok()
    }
}

/// 证书原文 + 签名，原文保持签名时的字节不变。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedBlob {
    #[serde(with = "crate::b64")]
    pub data: Vec<u8>,
    #[serde(with = "crate::b64")]
    pub sig: Vec<u8>,
}

/// 服务器签发的连接票据：证明这次连接经过了服务器授权（用于即时吊销）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnTicket {
    pub v: u8,
    pub conn_id: ConnId,
    pub controller_device: DeviceId,
    pub target_device: DeviceId,
    /// control 模式下为发起者账号 ID；assist 模式下为发起者用户 ID
    pub account_id: u64,
    pub mode: ConnMode,
    pub issued_at: i64,
    pub expires_at: i64,
}

impl ConnTicket {
    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_stdvec(self).expect("postcard")
    }
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        postcard::from_bytes(b).ok()
    }
}

/// 账号吊销列表，由账号签名密钥签名。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RevocationList {
    pub account_id: u64,
    pub epoch: u32,
    pub version: u64,
    pub devices: Vec<DeviceId>,
    pub issued_at: i64,
}

impl RevocationList {
    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_stdvec(self).expect("postcard")
    }
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        postcard::from_bytes(b).ok()
    }
}
