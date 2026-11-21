//! LemonGalaxy 连接层。
//!
//! - [`magicsock::MagicSock`]：一个 UDP 口同时承载 QUIC、两端探测、服务器探测与中转；
//!   QUIC 只看到每个对端的固定虚拟地址，真实走哪条路径（局域网 / 直连 / 中转）由这里决定，可随时切换。
//! - [`lan`]：局域网发现（组播 / 广播查询，单播回应）。
//! - [`relay_tcp`]：UDP 不通时经 WebSocket 走 TCP 中转。
//! - [`quic`]：基于魔法套接字的 QUIC 端点。
//! - [`handshake`]：端到端认证握手（docs/security.md 第 5 节）。
//! - [`tls`]：服务器地址解析与证书指纹固定。

pub mod addr;
pub mod frame;
pub mod handshake;
pub mod ifaces;
pub mod lan;
pub mod magicsock;
pub mod quic;
pub mod relay_tcp;
pub mod tls;

pub use addr::ServerAddr;
pub use magicsock::{MagicSock, PathInfo};

/// 安装 rustls 默认加密后端（ring）。重复调用无害。
pub fn init_crypto() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
