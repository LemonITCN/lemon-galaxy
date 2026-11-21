//! WebSocket 信令消息（JSON 文本帧），见 docs/protocol.md 第 2 节。

use crate::{ConnId, ConnMode, DeviceId, PathKind, UserBrief};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    Host,
    Srflx,
    Lan,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub addr: String,
    pub kind: CandidateKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum WsMsg {
    // ---- 认证 ----
    Hello {
        #[serde(with = "crate::b64")]
        nonce: Vec<u8>,
        server_time: i64,
    },
    Auth {
        device_id: DeviceId,
        #[serde(with = "crate::b64")]
        signature: Vec<u8>,
        /// 临时协助模式（未登录）时提供设备公钥
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::b64::opt")]
        device_pubkey: Option<Vec<u8>>,
    },
    Ready {
        device_id: DeviceId,
        public_ip: String,
        udp_port: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        assist_code: Option<String>,
    },

    // ---- 推送 ----
    Presence {
        device_id: DeviceId,
        online: bool,
    },
    DevicesChanged,
    VaultChanged {
        version: i64,
    },
    RevocationsChanged,
    SessionRevoked,
    AccountReset {
        user_id: u64,
    },

    // ---- 连接 ----
    Connect {
        req_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<DeviceId>,
        mode: ConnMode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        assist_code: Option<String>,
    },
    Incoming {
        conn_id: ConnId,
        mode: ConnMode,
        from_device: DeviceId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_user: Option<UserBrief>,
        #[serde(with = "crate::b64")]
        ticket: Vec<u8>,
        #[serde(with = "crate::b64")]
        ticket_sig: Vec<u8>,
        #[serde(with = "crate::b64")]
        relay_token: Vec<u8>,
        peer_public_ip: String,
        same_public_ip: bool,
    },
    ConnectOk {
        req_id: String,
        conn_id: ConnId,
        target: DeviceId,
        #[serde(with = "crate::b64")]
        ticket: Vec<u8>,
        #[serde(with = "crate::b64")]
        ticket_sig: Vec<u8>,
        #[serde(with = "crate::b64")]
        relay_token: Vec<u8>,
        peer_public_ip: String,
        same_public_ip: bool,
    },
    ConnectError {
        req_id: String,
        code: String,
        #[serde(default)]
        message: String,
    },
    Candidates {
        conn_id: ConnId,
        items: Vec<Candidate>,
    },
    ConnState {
        conn_id: ConnId,
        path: PathKind,
        rtt_ms: u32,
    },
    ConnClose {
        conn_id: ConnId,
    },

    Error {
        code: String,
        #[serde(default)]
        message: String,
    },
}

impl WsMsg {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("json")
    }
    pub fn from_json(s: &str) -> Option<Self> {
        serde_json::from_str(s).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn json_shape() {
        let m = WsMsg::Presence { device_id: DeviceId([1; 16]), online: true };
        let j = m.to_json();
        assert!(j.contains("\"t\":\"presence\""));
        assert_eq!(WsMsg::from_json(&j), Some(m));
        let m = WsMsg::DevicesChanged;
        assert_eq!(m.to_json(), "{\"t\":\"devices_changed\"}");
    }
}
