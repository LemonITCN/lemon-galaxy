//! 主控端发起连接：信令申请 → 收集并交换候选 → 局域网查询 → 中转兜底 → 持续探测选路 → QUIC → 端到端握手。

use crate::error::{ClientError, Result};
use crate::identity::Identity;
use crate::session::PeerSession;
use crate::signal::SignalClient;
use galaxy_net::handshake::{self, AssistSecret};
use galaxy_net::lan::LanDiscovery;
use galaxy_net::magicsock::MagicSock;
use galaxy_net::ServerAddr;
use galaxy_proto::cert::SignedBlob;
use galaxy_proto::peer::Hello;
use galaxy_proto::ws::{Candidate, CandidateKind, WsMsg};
use galaxy_proto::{ConnMode, DeviceId, PathKind};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

pub struct Connector {
    pub ms: Arc<MagicSock>,
    pub ep: quinn::Endpoint,
    pub lan: Arc<LanDiscovery>,
    pub signal: Arc<SignalClient>,
    pub identity: Identity,
    pub server: ServerAddr,
    pub account_pubkey: [u8; 32],
    pub cert: SignedBlob,
    pub lan_key: [u8; 32],
}

impl Connector {
    pub fn new(ms: Arc<MagicSock>, ep: quinn::Endpoint, signal: Arc<SignalClient>, identity: Identity, server: ServerAddr, account_pubkey: [u8; 32], cert: SignedBlob) -> Arc<Self> {
        ms.set_server(server.resolve());
        let lan = LanDiscovery::new(ms.clone());
        let lan_key = galaxy_crypto::lan_key(&account_pubkey);
        Arc::new(Self { ms, ep, lan, signal, identity, server, account_pubkey, cert, lan_key })
    }

    /// 查询局域网内同账号的设备（设备列表页定期调用）。
    pub async fn lan_scan(&self) {
        self.lan.query(self.lan_key, None).await;
    }

    pub fn lan_seen(&self, d: &DeviceId) -> bool {
        self.lan.lookup(d, Duration::from_secs(45)).is_some()
    }

    pub async fn connect(self: &Arc<Self>, device: DeviceId) -> Result<Arc<PeerSession>> {
        self.establish(Some(device), ConnMode::Control, None).await
    }

    pub async fn connect_assist(self: &Arc<Self>, device_code: &str, code: &str) -> Result<Arc<PeerSession>> {
        let dc: String = device_code.chars().filter(|c| c.is_ascii_digit()).collect();
        self.establish(None, ConnMode::Assist, Some(AssistSecret { code: code.to_string(), device_code: dc })).await
    }

    async fn establish(self: &Arc<Self>, target: Option<DeviceId>, mode: ConnMode, assist: Option<AssistSecret>) -> Result<Arc<PeerSession>> {
        // 先查一轮局域网，和信令并行
        if let Some(t) = target {
            let me = self.clone();
            tokio::spawn(async move {
                for _ in 0..3 {
                    me.lan.query(me.lan_key, Some(t)).await;
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            });
        }
        let t0 = std::time::Instant::now();
        let ok = self.signal.request_connect(target, mode, assist.as_ref().map(|a| a.device_code.clone())).await?;
        let WsMsg::ConnectOk { conn_id, target: tdev, ticket, ticket_sig, relay_token, same_public_ip, .. } = ok else {
            return Err(ClientError::Protocol("信令应答错误".into()));
        };
        let token: [u8; 16] = relay_token.as_slice().try_into().map_err(|_| ClientError::Protocol("中转令牌错误".into()))?;
        let tag = galaxy_crypto::conn_tag(&conn_id.0);
        let peer = self.ms.add_peer(tdev, tag, Some(token));
        let (stop_tx, stop_rx) = watch::channel(false);

        // 对方的候选
        {
            let mut sub = self.signal.subscribe();
            let ms = self.ms.clone();
            let peer = peer.clone();
            let mut stop = stop_rx.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        m = sub.recv() => match m {
                            Ok(WsMsg::Candidates { conn_id: c, items }) if c == conn_id => ms.add_candidates(&peer, &items),
                            Ok(WsMsg::ConnClose { conn_id: c }) if c == conn_id => break,
                            Ok(_) => {}
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                            Err(_) => break,
                        },
                        r = stop.changed() => if r.is_err() || *stop.borrow() { break },
                    }
                }
            });
        }
        // 局域网发现到的地址
        if let Some(seen) = self.lan.lookup(&tdev, Duration::from_secs(60)) {
            let c: Vec<Candidate> = seen.addrs.iter().map(|a| Candidate { addr: a.to_string(), kind: CandidateKind::Lan }).collect();
            self.ms.add_candidates(&peer, &c);
        }
        {
            let mut lan_rx = self.lan.subscribe();
            let ms = self.ms.clone();
            let peer = peer.clone();
            let mut stop = stop_rx.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        m = lan_rx.recv() => match m {
                            Ok(s) if s.device == tdev => {
                                let c: Vec<Candidate> = s.addrs.iter().map(|a| Candidate { addr: a.to_string(), kind: CandidateKind::Lan }).collect();
                                ms.add_candidates(&peer, &c);
                                ms.ping_all(&peer).await;
                            }
                            Ok(_) => {}
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                            Err(_) => break,
                        },
                        r = stop.changed() => if r.is_err() || *stop.borrow() { break },
                    }
                }
            });
        }
        // 自己的候选：网卡地址 + 服务器看到的公网地址
        let mut mine = self.ms.host_candidates();
        self.signal.send(WsMsg::Candidates { conn_id, items: mine.clone() });
        for a in self.ms.discover_reflexive(Duration::from_millis(800)).await {
            mine.push(Candidate { addr: a.to_string(), kind: CandidateKind::Srflx });
        }
        self.signal.send(WsMsg::Candidates { conn_id, items: mine });
        tracing::debug!("候选已发出 {:?}", t0.elapsed());
        if same_public_ip {
            tracing::debug!("两端公网 IP 相同，优先局域网");
        }

        // 中转兜底：UDP 绑定不上就走 TCP
        {
            let ms = self.ms.clone();
            let peer = peer.clone();
            let server = self.server.clone();
            let stop = stop_rx.clone();
            tokio::spawn(async move {
                if ms.bind_relay(&peer, Duration::from_millis(2500)).await {
                    return;
                }
                if *stop.borrow() {
                    return;
                }
                match galaxy_net::relay_tcp::connect(&server, token).await {
                    Ok((tx, mut rx)) => {
                        ms.set_relay_tcp(&peer, Some(tx));
                        while let Some(b) = rx.recv().await {
                            ms.inject_relay_tcp(&peer, b);
                        }
                        ms.set_relay_tcp(&peer, None);
                    }
                    Err(e) => tracing::debug!("TCP 中转失败: {e}"),
                }
            });
        }
        self.ms.spawn_prober(peer.clone(), stop_rx.clone());

        // 等到有任何可用路径（最多 8 秒），再发起 QUIC
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        while !self.ms.has_path(&peer) && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        tracing::debug!("路径就绪={} {:?} {:?}", self.ms.has_path(&peer), t0.elapsed(), peer.current_path());
        let connecting = self.ep.connect(peer.virt, "lemon-galaxy").map_err(|e| ClientError::Network(e.to_string()))?;
        let conn = match tokio::time::timeout(Duration::from_secs(20), connecting).await {
            Ok(Ok(c)) => c,
            Ok(Err(e)) => {
                let _ = stop_tx.send(true);
                return Err(ClientError::Network(format!("无法建立连接：{e}")));
            }
            Err(_) => {
                let _ = stop_tx.send(true);
                return Err(ClientError::Network("连接超时：直连和中转都不通".into()));
            }
        };

        tracing::debug!("QUIC 已建立 {:?}", t0.elapsed());
        let account_pk = self.account_pubkey;
        let verify = move |h: &Hello| -> std::result::Result<Option<galaxy_proto::cert::DeviceCert>, String> {
            if mode == ConnMode::Assist {
                return Ok(None);
            }
            let blob = h.cert.as_ref().ok_or("对方没有提供设备证书")?;
            let cert = galaxy_crypto::verify_device_cert(&account_pk, blob).map_err(|_| "对方证书不属于本账号".to_string())?;
            if cert.device_id != tdev || cert.device_pubkey != h.device_pubkey || !cert.kind.can_host() {
                return Err("对方证书与设备不符".into());
            }
            Ok(Some(cert))
        };
        let ticket_blob = SignedBlob { data: ticket, sig: ticket_sig };
        let est = match handshake::controller(&conn, &self.identity.key, Some(self.cert.clone()), Some(ticket_blob), assist, verify).await {
            Ok(e) => e,
            Err(e) => {
                conn.close(1u32.into(), b"auth");
                let _ = stop_tx.send(true);
                self.signal.send(WsMsg::ConnClose { conn_id });
                return Err(e.into());
            }
        };
        let session = PeerSession::new(conn, est.ack, tdev, Some(conn_id), (est.send, est.recv), self.ms.clone(), peer.clone(), stop_tx, Some(self.signal.clone()));
        tracing::info!("已连接 {}，路径 {:?}", tdev.short(), session.path().kind);
        let _ = PathKind::Connecting;
        Ok(session)
    }
}
