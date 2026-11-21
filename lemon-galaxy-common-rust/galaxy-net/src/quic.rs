//! 基于魔法套接字的 QUIC 端点。

use crate::magicsock::MagicSock;
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use std::sync::Arc;
use std::time::Duration;

pub fn transport() -> quinn::TransportConfig {
    let mut t = quinn::TransportConfig::default();
    t.max_idle_timeout(Some(Duration::from_secs(30).try_into().expect("idle")));
    t.keep_alive_interval(Some(Duration::from_secs(5)));
    t.max_concurrent_bidi_streams(512u32.into());
    t.max_concurrent_uni_streams(256u32.into());
    t.datagram_receive_buffer_size(Some(8 << 20));
    t.datagram_send_buffer_size(4 << 20);
    t.stream_receive_window((8u32 << 20).into());
    t.receive_window((32u32 << 20).into());
    t.send_window(32 << 20);
    // 中转会多 21 字节头，初始 MTU 留余量
    t.initial_mtu(1200);
    t.min_mtu(1200);
    t
}

/// 创建端点。`accept` 为 true 时可接受入站连接（被控端）。
pub fn endpoint(sock: Arc<MagicSock>, accept: bool) -> anyhow::Result<quinn::Endpoint> {
    let transport = Arc::new(transport());
    let server_cfg = if accept {
        let tls = crate::tls::peer_server_config()?;
        let mut sc = quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls)?));
        sc.transport_config(transport.clone());
        Some(sc)
    } else {
        None
    };
    // 关闭 QUIC 固定位混淆（RFC 9287）：魔法套接字靠首字节区分 QUIC 包与自有包
    let mut ec = quinn::EndpointConfig::default();
    ec.grease_quic_bit(false);
    let mut ep = quinn::Endpoint::new_with_abstract_socket(ec, server_cfg, sock, Arc::new(quinn::TokioRuntime))?;
    let mut cc = quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(crate::tls::peer_client_config())?));
    cc.transport_config(transport);
    ep.set_default_client_config(cc);
    Ok(ep)
}

/// 从 TLS 导出 32 字节用于端到端认证绑定。
pub fn exporter(conn: &quinn::Connection) -> anyhow::Result<[u8; 32]> {
    let mut out = [0u8; 32];
    conn.export_keying_material(&mut out, galaxy_proto::cert::TLS_EXPORTER_LABEL, b"")
        .map_err(|_| anyhow::anyhow!("无法导出会话密钥材料"))?;
    Ok(out)
}
