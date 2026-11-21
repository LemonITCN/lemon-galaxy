//! LemonGalaxy 线上消息与数据结构。
//!
//! - [`ws`]：客户端与服务器之间的 WebSocket 信令（JSON）
//! - [`api`]：客户端 HTTP 接口的请求与响应（JSON）
//! - [`cert`]：设备证书、连接票据、吊销列表（postcard 编码后签名）
//! - [`peer`]：两端之间 QUIC 会话里的消息（postcard 帧）
//! - [`wire`]：UDP 包格式（服务器探测/中转、两端探测、局域网发现）
//!
//! 协议细节见仓库 docs/protocol.md。

pub mod api;
pub mod b64;
pub mod cert;
pub mod frame;
pub mod ids;
pub mod model;
pub mod peer;
pub mod update;
pub mod wire;
pub mod ws;

pub use ids::{ConnId, DeviceId};
pub use model::*;

/// 两端会话协议版本。
/// 完整版本号（CI 构建带构建序号，见 build.rs）。
pub const APP_VERSION: &str = env!("LEMONGALAXY_APP_VERSION");

pub const PROTOCOL_VERSION: u32 = 1;
/// QUIC ALPN。
pub const ALPN: &[u8] = b"lemon-galaxy/1";
/// 服务器默认端口（TCP 与 UDP 同号）。
pub const DEFAULT_PORT: u16 = 7443;
/// 局域网发现端口。
pub const LAN_PORT: u16 = 7444;
/// 单帧最大字节数。
pub const MAX_FRAME: usize = 4 * 1024 * 1024;

/// 当前 Unix 毫秒。
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 当前 Unix 秒。
pub fn now_secs() -> i64 {
    now_ms() / 1000
}
