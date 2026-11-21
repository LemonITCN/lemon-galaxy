//! 服务器 UDP：地址探测（STUN 类）与 UDP 中转。见 docs/protocol.md 第 3 节。

use crate::state::{RelayEnd, St};
use galaxy_proto::wire::{self, ServerPacket};
use std::net::SocketAddr;

pub fn normalize(a: SocketAddr) -> SocketAddr {
    match a {
        SocketAddr::V6(v6) => match v6.ip().to_ipv4_mapped() {
            Some(v4) => SocketAddr::new(v4.into(), v6.port()),
            None => a,
        },
        a => a,
    }
}

pub async fn run(st: St) {
    let sock = st.udp.clone();
    let mut buf = vec![0u8; 65536];
    loop {
        let (n, from) = match sock.recv_from(&mut buf).await {
            Ok(x) => x,
            Err(e) => {
                tracing::debug!("UDP 接收错误: {e}");
                continue;
            }
        };
        let pkt = &buf[..n];
        // 热路径：中转数据不复制负载
        if let Some((t, token, pos)) = wire::peek_relay(pkt) {
            if t == 0x12 {
                if let Some((pair, side)) = st.relay.lookup(&token) {
                    {
                        let mut e = pair.ends[side].lock();
                        match &*e {
                            RelayEnd::Udp(a) if *a == from => {}
                            RelayEnd::Tcp(_) => {}
                            _ => *e = RelayEnd::Udp(from),
                        }
                    }
                    crate::ws::relay_forward(&st, &pair, side, &pkt[pos..]).await;
                }
            }
            continue;
        }
        match ServerPacket::decode(pkt) {
            Some(ServerPacket::BindRequest { txn }) => {
                let resp = ServerPacket::BindResponse { txn, observed: normalize(from) }.encode();
                let _ = sock.send_to(&resp, from).await;
            }
            Some(ServerPacket::RelayBind { token }) | Some(ServerPacket::Keepalive { token }) => {
                if let Some((pair, side)) = st.relay.lookup(&token) {
                    *pair.ends[side].lock() = RelayEnd::Udp(from);
                    pair.last_activity.store(galaxy_proto::now_ms(), std::sync::atomic::Ordering::Relaxed);
                    let _ = sock.send_to(&ServerPacket::RelayBound { token }.encode(), from).await;
                }
            }
            _ => {}
        }
    }
}
