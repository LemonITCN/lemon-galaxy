//! 魔法套接字：一个 UDP 口承载 QUIC + 两端探测 + 服务器探测/中转。
//!
//! 每个对端设备分配一个虚拟地址 `[fd4c:4778::<n>]:7` 交给 QUIC。发往虚拟地址的包由这里按当前最优路径
//! 发出（局域网 / 公网直连 / UDP 中转 / TCP 中转）；从任意路径收到的该对端的 QUIC 包都还原成同一个虚拟地址。
//! 因此换路径对 QUIC 完全透明，连接不断。

use bytes::Bytes;
use galaxy_proto::wire::{self, Disco, LanReply, PacketClass, ServerPacket};
use galaxy_proto::ws::{Candidate, CandidateKind};
use galaxy_proto::{DeviceId, PathKind};
use parking_lot::{Mutex, RwLock};
use quinn::udp::{RecvMeta, Transmit};
use quinn::{AsyncUdpSocket, UdpPoller};
use std::collections::HashMap;
use std::fmt;
use std::io::{self, IoSliceMut};
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::pin::Pin;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot, watch};

const RELAY_UDP_PENALTY: u32 = 40;
const RELAY_TCP_PENALTY: u32 = 80;
const DIRECT_V4_PENALTY: u32 = 5;
const DIRECT_V6_PENALTY: u32 = 3;
const VIRTUAL_IF_PENALTY: u32 = 20;
const SWITCH_MARGIN: u32 = 10;
const PATH_TIMEOUT: Duration = Duration::from_secs(7);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathInfo {
    pub kind: PathKind,
    pub rtt_ms: u32,
    pub remote: String,
}

impl PathInfo {
    pub fn connecting() -> Self {
        Self { kind: PathKind::Connecting, rtt_ms: 0, remote: String::new() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Route {
    Direct(SocketAddr),
    RelayUdp,
    RelayTcp,
}

#[derive(Clone, Copy, Debug)]
struct PathStat {
    rtt_ms: u32,
    last_pong: Instant,
    lan: bool,
    penalty: u32,
}

struct PeerState {
    conn_tag: [u8; 8],
    candidates: Vec<(SocketAddr, CandidateKind)>,
    paths: HashMap<Route, PathStat>,
    best: Option<Route>,
    pings: HashMap<[u8; 12], (Instant, Route)>,
    relay_token: Option<[u8; 16]>,
    relay_udp_bound: bool,
    relay_tcp: Option<mpsc::Sender<Bytes>>,
    virtual_ifaces: Vec<IpAddr>,
}

pub struct Peer {
    pub device: DeviceId,
    pub virt: SocketAddr,
    state: Mutex<PeerState>,
    path_tx: watch::Sender<PathInfo>,
}

impl Peer {
    pub fn path(&self) -> watch::Receiver<PathInfo> {
        self.path_tx.subscribe()
    }
    pub fn current_path(&self) -> PathInfo {
        self.path_tx.borrow().clone()
    }
}

struct Inner {
    sock: UdpSocket,
    v6_socket: bool,
    peers_by_virt: RwLock<HashMap<SocketAddr, Arc<Peer>>>,
    peers_by_device: RwLock<HashMap<DeviceId, Arc<Peer>>>,
    by_tag: RwLock<HashMap<[u8; 8], Arc<Peer>>>,
    by_real: RwLock<HashMap<SocketAddr, Arc<Peer>>>,
    by_token: RwLock<HashMap<[u8; 16], Arc<Peer>>>,
    rx_tx: mpsc::Sender<(SocketAddr, Bytes)>,
    rx_rx: Mutex<mpsc::Receiver<(SocketAddr, Bytes)>>,
    server_udp: RwLock<Vec<SocketAddr>>,
    bind_waiters: Mutex<HashMap<[u8; 12], oneshot::Sender<SocketAddr>>>,
    lan_tx: Mutex<Option<mpsc::UnboundedSender<(LanReply, SocketAddr)>>>,
    next_virt: AtomicU16,
    /// 关闭后收包循环退出（最后一个引用放掉时 UDP 端口随之释放）
    closed: watch::Sender<bool>,
}

/// 魔法套接字。`Arc<MagicSock>` 交给 quinn 作为 UDP 套接字。
pub struct MagicSock {
    inner: Arc<Inner>,
    local_port: u16,
}

impl Drop for MagicSock {
    fn drop(&mut self) {
        // 收包循环自己也拿着 Inner，不通知它就永远不退出、端口也不释放
        let _ = self.inner.closed.send(true);
    }
}

impl fmt::Debug for MagicSock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MagicSock(:{})", self.local_port)
    }
}

/// 测试用：环境变量 LEMON_GALAXY_FORCE_RELAY=udp|tcp 时不走直连（tcp 还会跳过 UDP 中转）。
fn force_relay() -> Option<String> {
    std::env::var("LEMON_GALAXY_FORCE_RELAY").ok().filter(|s| s == "udp" || s == "tcp")
}

fn normalize(a: SocketAddr) -> SocketAddr {
    match a {
        SocketAddr::V6(v6) => match v6.ip().to_ipv4_mapped() {
            Some(v4) => SocketAddr::new(IpAddr::V4(v4), v6.port()),
            None => a,
        },
        a => a,
    }
}

fn virt_addr(n: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V6(Ipv6Addr::new(0xfd4c, 0x4778, 0, 0, 0, 0, 0, n)), 7)
}

impl MagicSock {
    /// 绑定本机 UDP 端口（0 为随机）。优先双栈，失败退到 IPv4。
    pub fn bind(port: u16) -> io::Result<Arc<Self>> {
        use socket2::{Domain, Protocol, Socket, Type};
        let try_v6 = || -> io::Result<Socket> {
            let s = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
            s.set_only_v6(false)?;
            s.bind(&SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port).into())?;
            Ok(s)
        };
        let (s, v6) = match try_v6() {
            Ok(s) => (s, true),
            Err(_) => {
                let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
                s.bind(&SocketAddr::new(IpAddr::from([0, 0, 0, 0]), port).into())?;
                (s, false)
            }
        };
        let _ = s.set_recv_buffer_size(4 << 20);
        let _ = s.set_send_buffer_size(4 << 20);
        let _ = s.set_broadcast(true);
        s.set_nonblocking(true)?;
        let std_sock: std::net::UdpSocket = s.into();
        let sock = UdpSocket::from_std(std_sock)?;
        let local_port = sock.local_addr()?.port();
        let (rx_tx, rx_rx) = mpsc::channel(4096);
        let inner = Arc::new(Inner {
            sock,
            v6_socket: v6,
            peers_by_virt: Default::default(),
            peers_by_device: Default::default(),
            by_tag: Default::default(),
            by_real: Default::default(),
            by_token: Default::default(),
            rx_tx,
            rx_rx: Mutex::new(rx_rx),
            server_udp: Default::default(),
            bind_waiters: Default::default(),
            lan_tx: Default::default(),
            next_virt: AtomicU16::new(1),
            closed: watch::channel(false).0,
        });
        tokio::spawn(recv_loop(inner.clone()));
        Ok(Arc::new(Self { inner, local_port }))
    }

    /// 停止收包。之后这个套接字不再处理任何数据包（被控服务停止时用）。
    pub fn close(&self) {
        let _ = self.inner.closed.send(true);
    }

    pub fn port(&self) -> u16 {
        self.local_port
    }

    /// 设置服务器 UDP 地址（可多个：IPv4 与 IPv6）。
    pub fn set_server(&self, addrs: Vec<SocketAddr>) {
        *self.inner.server_udp.write() = addrs.into_iter().map(normalize).collect();
    }

    /// 接收局域网发现回应。
    pub fn set_lan_reply_sink(&self, tx: mpsc::UnboundedSender<(LanReply, SocketAddr)>) {
        *self.inner.lan_tx.lock() = Some(tx);
    }

    /// 直接从魔法套接字发包（局域网回应用）。
    pub async fn send_raw(&self, data: &[u8], to: SocketAddr) -> io::Result<usize> {
        self.inner.sock.send_to(data, self.inner.to_sock(to)).await
    }

    /// 向服务器探测本机公网地址。
    pub async fn discover_reflexive(&self, timeout: Duration) -> Vec<SocketAddr> {
        let servers = self.inner.server_udp.read().clone();
        let mut out = vec![];
        let mut rxs = vec![];
        for s in servers {
            let txn: [u8; 12] = rand::random();
            let (tx, rx) = oneshot::channel();
            self.inner.bind_waiters.lock().insert(txn, tx);
            let pkt = ServerPacket::BindRequest { txn }.encode();
            for _ in 0..2 {
                let _ = self.inner.sock.send_to(&pkt, self.inner.to_sock(s)).await;
            }
            rxs.push(rx);
        }
        for rx in rxs {
            if let Ok(Ok(a)) = tokio::time::timeout(timeout, rx).await {
                if !out.contains(&a) {
                    out.push(a);
                }
            }
        }
        out
    }

    /// 本机 host 候选（所有网卡地址 + 本口）。
    pub fn host_candidates(&self) -> Vec<Candidate> {
        crate::ifaces::list()
            .into_iter()
            .filter(|i| self.inner.v6_socket || i.ip.is_ipv4())
            .map(|i| Candidate { addr: SocketAddr::new(i.ip, self.local_port).to_string(), kind: CandidateKind::Host })
            .collect()
    }

    /// 为某对端设备建立（或复用）路径状态，返回 QUIC 用的虚拟地址。
    pub fn add_peer(&self, device: DeviceId, conn_tag: [u8; 8], relay_token: Option<[u8; 16]>) -> Arc<Peer> {
        let existing = self.inner.peers_by_device.read().get(&device).cloned();
        let peer = match existing {
            Some(p) => p,
            None => {
                let n = self.inner.next_virt.fetch_add(1, Ordering::Relaxed);
                let (path_tx, _) = watch::channel(PathInfo::connecting());
                let virtual_ifaces = crate::ifaces::list().into_iter().filter(|i| i.virtual_like).map(|i| i.ip).collect();
                let p = Arc::new(Peer {
                    device,
                    virt: virt_addr(n),
                    state: Mutex::new(PeerState {
                        conn_tag,
                        candidates: vec![],
                        paths: HashMap::new(),
                        best: None,
                        pings: HashMap::new(),
                        relay_token: None,
                        relay_udp_bound: false,
                        relay_tcp: None,
                        virtual_ifaces,
                    }),
                    path_tx,
                });
                self.inner.peers_by_virt.write().insert(p.virt, p.clone());
                self.inner.peers_by_device.write().insert(device, p.clone());
                p
            }
        };
        {
            let mut s = peer.state.lock();
            let old_tag = s.conn_tag;
            if old_tag != conn_tag {
                // 新的一次连接：旧路径（对方旧端口等）全部作废，重新探测
                s.candidates.clear();
                s.paths.clear();
                s.best = None;
                s.pings.clear();
                s.relay_udp_bound = false;
                s.relay_tcp = None;
                self.inner.by_real.write().retain(|_, v| !Arc::ptr_eq(v, &peer));
                let _ = peer.path_tx.send(PathInfo::connecting());
            }
            s.conn_tag = conn_tag;
            self.inner.by_tag.write().remove(&old_tag);
            if let Some(t) = relay_token {
                if let Some(old) = s.relay_token.replace(t) {
                    self.inner.by_token.write().remove(&old);
                }
                s.relay_udp_bound = false;
                self.inner.by_token.write().insert(t, peer.clone());
            }
        }
        self.inner.by_tag.write().insert(conn_tag, peer.clone());
        peer
    }

    pub fn peer_by_virt(&self, virt: &SocketAddr) -> Option<Arc<Peer>> {
        self.inner.peers_by_virt.read().get(virt).cloned()
    }

    pub fn peer(&self, device: &DeviceId) -> Option<Arc<Peer>> {
        self.inner.peers_by_device.read().get(device).cloned()
    }

    pub fn remove_peer(&self, device: &DeviceId) {
        if let Some(p) = self.inner.peers_by_device.write().remove(device) {
            self.inner.peers_by_virt.write().remove(&p.virt);
            let s = p.state.lock();
            self.inner.by_tag.write().remove(&s.conn_tag);
            if let Some(t) = s.relay_token {
                self.inner.by_token.write().remove(&t);
            }
            let mut br = self.inner.by_real.write();
            br.retain(|_, v| !Arc::ptr_eq(v, &p));
        }
    }

    /// 只有当前登记的还是这个对端对象时才移除（延迟清理时别误删之后新建的连接）。
    pub fn remove_peer_if_same(&self, device: &DeviceId, peer: &Arc<Peer>) {
        let same = self.inner.peers_by_device.read().get(device).map(|p| Arc::ptr_eq(p, peer)).unwrap_or(false);
        if same {
            self.remove_peer(device);
        }
    }

    /// 加入对端候选地址。
    pub fn add_candidates(&self, peer: &Arc<Peer>, items: &[Candidate]) {
        let mut s = peer.state.lock();
        for c in items {
            let Ok(a) = c.addr.parse::<SocketAddr>() else { continue };
            let a = normalize(a);
            if a.ip().is_unspecified() || a.port() == 0 {
                continue;
            }
            if !self.inner.v6_socket && a.is_ipv6() {
                continue;
            }
            if !s.candidates.iter().any(|(x, _)| *x == a) {
                s.candidates.push((a, c.kind));
                self.inner.by_real.write().insert(a, peer.clone());
            }
        }
    }

    /// 绑定 UDP 中转（向服务器发 RelayBind）。返回是否在超时内收到确认。
    pub async fn bind_relay(&self, peer: &Arc<Peer>, timeout: Duration) -> bool {
        let Some(token) = peer.state.lock().relay_token else { return false };
        if force_relay().as_deref() == Some("tcp") {
            return false;
        }
        let servers = self.inner.server_udp.read().clone();
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            for s in &servers {
                let _ = self.inner.sock.send_to(&ServerPacket::RelayBind { token }.encode(), self.inner.to_sock(*s)).await;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
            if peer.state.lock().relay_udp_bound {
                return true;
            }
        }
        false
    }

    /// 设置 TCP 中转通道（WebSocket 写端）。
    pub fn set_relay_tcp(&self, peer: &Arc<Peer>, tx: Option<mpsc::Sender<Bytes>>) {
        {
            let mut s = peer.state.lock();
            s.relay_tcp = tx;
            if s.relay_tcp.is_none() {
                s.paths.remove(&Route::RelayTcp);
            }
        }
        // 没有测出延迟前也要把「走 TCP 中转」报上去
        recompute_best(peer);
    }

    /// TCP 中转收到的负载（对端发来的 QUIC 或探测包）。
    pub fn inject_relay_tcp(&self, peer: &Arc<Peer>, payload: Bytes) {
        handle_from_peer_relay(&self.inner, peer, &payload, true);
    }

    /// 发一轮探测：所有候选地址 + 中转。
    pub async fn ping_all(&self, peer: &Arc<Peer>) {
        let (tag, mut cands, token, udp_bound, tcp) = {
            let s = peer.state.lock();
            (s.conn_tag, s.candidates.clone(), s.relay_token, s.relay_udp_bound, s.relay_tcp.clone())
        };
        if force_relay().is_some() {
            cands.clear();
        }
        for (a, _) in cands {
            let txn: [u8; 12] = rand::random();
            peer.state.lock().pings.insert(txn, (Instant::now(), Route::Direct(a)));
            let _ = self.inner.sock.send_to(&Disco::Ping { conn_tag: tag, txn }.encode(), self.inner.to_sock(a)).await;
        }
        let servers = self.inner.server_udp.read().clone();
        if let Some(token) = token {
            if udp_bound {
                let txn: [u8; 12] = rand::random();
                peer.state.lock().pings.insert(txn, (Instant::now(), Route::RelayUdp));
                let pkt = wire::relay_data(&token, &Disco::Ping { conn_tag: tag, txn }.encode());
                for s in servers.iter().copied() {
                    let _ = self.inner.sock.send_to(&pkt, self.inner.to_sock(s)).await;
                }
                // 保持中转绑定
                for s in servers.iter().copied() {
                    let _ = self.inner.sock.send_to(&ServerPacket::Keepalive { token }.encode(), self.inner.to_sock(s)).await;
                }
            }
            if let Some(tx) = tcp {
                let txn: [u8; 12] = rand::random();
                peer.state.lock().pings.insert(txn, (Instant::now(), Route::RelayTcp));
                let _ = tx.try_send(Bytes::from(Disco::Ping { conn_tag: tag, txn }.encode()));
            }
        }
        // 清理过期路径与探测
        let mut s = peer.state.lock();
        s.pings.retain(|_, (t, _)| t.elapsed() < Duration::from_secs(10));
        let before = s.best;
        s.paths.retain(|_, p| p.last_pong.elapsed() < PATH_TIMEOUT);
        if let Some(b) = before {
            if !s.paths.contains_key(&b) {
                s.best = None;
            }
        }
        drop(s);
        recompute_best(peer);
    }

    /// 后台持续探测，直到 `stop` 收到信号。前 4 秒密集，之后每 3 秒。
    pub fn spawn_prober(self: &Arc<Self>, peer: Arc<Peer>, mut stop: watch::Receiver<bool>) {
        let ms = self.clone();
        tokio::spawn(async move {
            let start = Instant::now();
            loop {
                ms.ping_all(&peer).await;
                let wait = if start.elapsed() < Duration::from_secs(4) { 200 } else if start.elapsed() < Duration::from_secs(20) { 1000 } else { 3000 };
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(wait)) => {}
                    r = stop.changed() => { if r.is_err() || *stop.borrow() { break; } }
                }
            }
        });
    }

    pub fn relay_token(&self, peer: &Arc<Peer>) -> Option<[u8; 16]> {
        peer.state.lock().relay_token
    }

    /// 是否已有可用路径。
    pub fn has_path(&self, peer: &Arc<Peer>) -> bool {
        let s = peer.state.lock();
        s.best.is_some() || s.relay_udp_bound || s.relay_tcp.is_some()
    }
}

impl Inner {
    fn to_sock(&self, a: SocketAddr) -> SocketAddr {
        if self.v6_socket {
            if let SocketAddr::V4(v4) = a {
                return SocketAddr::new(IpAddr::V6(v4.ip().to_ipv6_mapped()), v4.port());
            }
        }
        a
    }

    fn is_server(&self, a: &SocketAddr) -> bool {
        self.server_udp.read().iter().any(|s| s == a)
    }

    fn send_via(&self, peer: &Arc<Peer>, route: Option<Route>, data: &[u8]) -> io::Result<()> {
        let (token, tcp, udp_bound) = {
            let s = peer.state.lock();
            (s.relay_token, s.relay_tcp.clone(), s.relay_udp_bound)
        };
        match route {
            Some(Route::Direct(a)) => match self.sock.try_send_to(data, self.to_sock(a)) {
                Ok(_) => Ok(()),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => Err(e),
                Err(_) => Ok(()),
            },
            Some(Route::RelayTcp) => {
                if let Some(tx) = tcp {
                    let _ = tx.try_send(Bytes::copy_from_slice(data));
                }
                Ok(())
            }
            Some(Route::RelayUdp) | None => {
                // 还没有直连路径：先走中转（UDP 已绑定或未知时都发，TCP 有就也发）
                if let Some(token) = token {
                    if udp_bound || route.is_none() {
                        let pkt = wire::relay_data(&token, data);
                        for s in self.server_udp.read().iter() {
                            match self.sock.try_send_to(&pkt, self.to_sock(*s)) {
                                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Err(e),
                                _ => {}
                            }
                        }
                    }
                    if route.is_none() && !udp_bound {
                        if let Some(tx) = tcp {
                            let _ = tx.try_send(Bytes::copy_from_slice(data));
                        }
                    }
                }
                Ok(())
            }
        }
    }
}

fn path_penalty(s: &PeerState, a: &SocketAddr) -> (u32, bool) {
    let kind = s.candidates.iter().find(|(x, _)| x == a).map(|(_, k)| *k);
    let lan = matches!(kind, Some(CandidateKind::Lan)) || crate::ifaces::is_private(&a.ip());
    let mut p = if lan { 0 } else if a.is_ipv6() { DIRECT_V6_PENALTY } else { DIRECT_V4_PENALTY };
    // 对端地址落在本机虚拟网卡同网段的情况很难判断，按候选来源保守处理：VPN/容器地址统一加惩罚
    if s.virtual_ifaces.iter().any(|v| v.is_ipv4() == a.ip().is_ipv4() && same_prefix(v, &a.ip())) {
        p += VIRTUAL_IF_PENALTY;
    }
    (p, lan)
}

fn same_prefix(a: &IpAddr, b: &IpAddr) -> bool {
    match (a, b) {
        (IpAddr::V4(x), IpAddr::V4(y)) => x.octets()[..3] == y.octets()[..3],
        (IpAddr::V6(x), IpAddr::V6(y)) => x.segments()[..4] == y.segments()[..4],
        _ => false,
    }
}

fn recompute_best(peer: &Arc<Peer>) {
    let info = {
        let mut s = peer.state.lock();
        let score = |r: &Route, p: &PathStat| -> u32 { p.rtt_ms + p.penalty + if matches!(r, Route::Direct(_)) { 0 } else { 0 } };
        let mut best: Option<(Route, u32)> = None;
        for (r, p) in &s.paths {
            let sc = score(r, p);
            if best.map(|(_, b)| sc < b).unwrap_or(true) {
                best = Some((*r, sc));
            }
        }
        let cur = s.best.and_then(|r| s.paths.get(&r).map(|p| (r, score(&r, p))));
        let chosen = match (cur, best) {
            (Some((cr, cs)), Some((_, bs))) if cs <= bs + SWITCH_MARGIN => Some(cr),
            (_, Some((br, _))) => Some(br),
            (_, None) => None,
        };
        s.best = chosen;
        match chosen {
            Some(Route::Direct(a)) => {
                let p = s.paths[&Route::Direct(a)];
                PathInfo { kind: if p.lan { PathKind::Lan } else { PathKind::Direct }, rtt_ms: p.rtt_ms, remote: a.to_string() }
            }
            Some(Route::RelayUdp) => PathInfo { kind: PathKind::RelayUdp, rtt_ms: s.paths[&Route::RelayUdp].rtt_ms, remote: String::new() },
            Some(Route::RelayTcp) => PathInfo { kind: PathKind::RelayTcp, rtt_ms: s.paths[&Route::RelayTcp].rtt_ms, remote: String::new() },
            None => {
                if s.relay_udp_bound {
                    PathInfo { kind: PathKind::RelayUdp, rtt_ms: 0, remote: String::new() }
                } else if s.relay_tcp.is_some() {
                    PathInfo { kind: PathKind::RelayTcp, rtt_ms: 0, remote: String::new() }
                } else {
                    PathInfo::connecting()
                }
            }
        }
    };
    peer.path_tx.send_if_modified(|cur| {
        let changed = cur.kind != info.kind || cur.remote != info.remote || cur.rtt_ms.abs_diff(info.rtt_ms) > 5;
        if changed {
            *cur = info;
        }
        changed
    });
}

fn on_pong(peer: &Arc<Peer>, txn: &[u8; 12]) {
    {
        let mut s = peer.state.lock();
        let Some((sent, route)) = s.pings.remove(txn) else { return };
        let rtt = sent.elapsed().as_millis().min(60_000) as u32;
        let (penalty, lan) = match route {
            Route::Direct(a) => path_penalty(&s, &a),
            Route::RelayUdp => (RELAY_UDP_PENALTY, false),
            Route::RelayTcp => (RELAY_TCP_PENALTY, false),
        };
        let e = s.paths.entry(route).or_insert(PathStat { rtt_ms: rtt, last_pong: Instant::now(), lan, penalty });
        // 平滑 RTT
        e.rtt_ms = (e.rtt_ms * 3 + rtt) / 4;
        e.last_pong = Instant::now();
        e.lan = lan;
        e.penalty = penalty;
    }
    recompute_best(peer);
}

/// 处理经中转到达的、来自某对端的负载。
fn handle_from_peer_relay(inner: &Arc<Inner>, peer: &Arc<Peer>, payload: &[u8], tcp: bool) {
    match wire::classify(payload) {
        PacketClass::Quic => {
            let _ = inner.rx_tx.try_send((peer.virt, Bytes::copy_from_slice(payload)));
        }
        PacketClass::Disco => match Disco::decode(payload) {
            Some(Disco::Ping { conn_tag, txn }) => {
                let (tag, token, tx) = {
                    let s = peer.state.lock();
                    (s.conn_tag, s.relay_token, s.relay_tcp.clone())
                };
                if conn_tag != tag {
                    return;
                }
                let pong = Disco::Pong { conn_tag, txn, observed: SocketAddr::from(([0, 0, 0, 0], 0)) }.encode();
                if tcp {
                    if let Some(tx) = tx {
                        let _ = tx.try_send(Bytes::from(pong));
                    }
                } else if let Some(token) = token {
                    let pkt = wire::relay_data(&token, &pong);
                    for s in inner.server_udp.read().iter() {
                        let _ = inner.sock.try_send_to(&pkt, inner.to_sock(*s));
                    }
                }
            }
            Some(Disco::Pong { conn_tag, txn, .. }) => {
                if conn_tag == peer.state.lock().conn_tag {
                    on_pong(peer, &txn);
                }
            }
            None => {}
        },
        _ => {}
    }
}

async fn recv_loop(inner: Arc<Inner>) {
    let mut buf = vec![0u8; 65536];
    let mut closed = inner.closed.subscribe();
    loop {
        if *closed.borrow() {
            return;
        }
        let r = tokio::select! {
            r = inner.sock.recv_from(&mut buf) => r,
            _ = closed.changed() => continue,
        };
        let (n, from) = match r {
            Ok(x) => x,
            Err(e) => {
                // Windows 上收到 ICMP 端口不可达会报 ConnectionReset，忽略即可
                tracing::trace!("magicsock recv: {e}");
                continue;
            }
        };
        let from = normalize(from);
        let pkt = &buf[..n];
        match wire::classify(pkt) {
            PacketClass::Quic => {
                let peer = inner.by_real.read().get(&from).cloned();
                if let Some(p) = peer {
                    let _ = inner.rx_tx.try_send((p.virt, Bytes::copy_from_slice(pkt)));
                }
            }
            PacketClass::Disco => handle_disco_direct(&inner, from, pkt).await,
            PacketClass::Server => {
                if !inner.is_server(&from) {
                    continue;
                }
                if let Some((t, token, pos)) = wire::peek_relay(pkt) {
                    if t == 0x13 {
                        let peer = inner.by_token.read().get(&token).cloned();
                        if let Some(p) = peer {
                            handle_from_peer_relay(&inner, &p, &pkt[pos..], false);
                        }
                    }
                    continue;
                }
                match ServerPacket::decode(pkt) {
                    Some(ServerPacket::BindResponse { txn, observed }) => {
                        if let Some(tx) = inner.bind_waiters.lock().remove(&txn) {
                            let _ = tx.send(observed);
                        }
                    }
                    Some(ServerPacket::RelayBound { token }) => {
                        let peer = inner.by_token.read().get(&token).cloned();
                        if let Some(p) = peer {
                            let changed = {
                                let mut s = p.state.lock();
                                let c = !s.relay_udp_bound;
                                s.relay_udp_bound = true;
                                c
                            };
                            if changed {
                                recompute_best(&p);
                            }
                        }
                    }
                    _ => {}
                }
            }
            PacketClass::LanReply => {
                if let Some(r) = LanReply::decode(pkt) {
                    if let Some(tx) = inner.lan_tx.lock().as_ref() {
                        let _ = tx.send((r, from));
                    }
                }
            }
            _ => {}
        }
    }
}

async fn handle_disco_direct(inner: &Arc<Inner>, from: SocketAddr, pkt: &[u8]) {
    if force_relay().is_some() {
        return;
    }
    match Disco::decode(pkt) {
        Some(Disco::Ping { conn_tag, txn }) => {
            let peer = inner.by_tag.read().get(&conn_tag).cloned();
            let Some(peer) = peer else { return };
            // 对端的真实来源地址（可能是 NAT 映射后的地址）：记为候选，并允许其 QUIC 包进入
            let is_new = {
                let mut s = peer.state.lock();
                if !s.candidates.iter().any(|(a, _)| *a == from) {
                    let kind = if crate::ifaces::is_private(&from.ip()) { CandidateKind::Lan } else { CandidateKind::Srflx };
                    s.candidates.push((from, kind));
                    true
                } else {
                    false
                }
            };
            inner.by_real.write().insert(from, peer.clone());
            let pong = Disco::Pong { conn_tag, txn, observed: from }.encode();
            let _ = inner.sock.send_to(&pong, inner.to_sock(from)).await;
            if is_new {
                // 反向探测一次，尽快确认这条路径
                let txn2: [u8; 12] = rand::random();
                peer.state.lock().pings.insert(txn2, (Instant::now(), Route::Direct(from)));
                let _ = inner.sock.send_to(&Disco::Ping { conn_tag, txn: txn2 }.encode(), inner.to_sock(from)).await;
            }
        }
        Some(Disco::Pong { conn_tag, txn, .. }) => {
            let peer = inner.by_tag.read().get(&conn_tag).cloned();
            if let Some(p) = peer {
                inner.by_real.write().insert(from, p.clone());
                on_pong(&p, &txn);
            }
        }
        None => {}
    }
}

// ---------------- quinn 接口 ----------------

#[derive(Debug)]
struct Poller {
    inner: Arc<Inner>,
}

impl UdpPoller for Poller {
    fn poll_writable(self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        self.inner.sock.poll_send_ready(cx)
    }
}

impl fmt::Debug for Inner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MagicSockInner")
    }
}

impl AsyncUdpSocket for MagicSock {
    fn create_io_poller(self: Arc<Self>) -> Pin<Box<dyn UdpPoller>> {
        Box::pin(Poller { inner: self.inner.clone() })
    }

    fn try_send(&self, t: &Transmit) -> io::Result<()> {
        let peer = self.inner.peers_by_virt.read().get(&t.destination).cloned();
        let Some(peer) = peer else { return Ok(()) };
        let route = peer.state.lock().best;
        match t.segment_size {
            Some(seg) if seg < t.contents.len() => {
                for chunk in t.contents.chunks(seg) {
                    self.inner.send_via(&peer, route, chunk)?;
                }
                Ok(())
            }
            _ => self.inner.send_via(&peer, route, t.contents),
        }
    }

    fn poll_recv(&self, cx: &mut Context, bufs: &mut [IoSliceMut<'_>], meta: &mut [RecvMeta]) -> Poll<io::Result<usize>> {
        let mut rx = self.inner.rx_rx.lock();
        let mut count = 0;
        while count < bufs.len().min(meta.len()) {
            match rx.poll_recv(cx) {
                Poll::Ready(Some((from, data))) => {
                    let n = data.len().min(bufs[count].len());
                    bufs[count][..n].copy_from_slice(&data[..n]);
                    meta[count] = RecvMeta { addr: from, len: n, stride: n, ecn: None, dst_ip: None };
                    count += 1;
                }
                Poll::Ready(None) => {
                    if count > 0 {
                        break;
                    }
                    return Poll::Ready(Err(io::Error::new(io::ErrorKind::BrokenPipe, "magicsock closed")));
                }
                Poll::Pending => break,
            }
        }
        if count > 0 {
            Poll::Ready(Ok(count))
        } else {
            Poll::Pending
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(SocketAddr::new(IpAddr::V6(Ipv6Addr::new(0xfd4c, 0x4778, 0, 0, 0, 0, 0, 0)), 7))
    }

    fn max_receive_segments(&self) -> usize {
        1
    }
}
