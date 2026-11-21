//! 局域网发现（docs/protocol.md 第 5 节）。
//!
//! 被控端在 7444 端口听组播 / 广播查询；主控端从任意口发查询，回应单播到主控端的魔法套接字端口。
//! 查询与回应都用 `lan_key = HKDF(账号公钥)` 做 HMAC，同网络的陌生设备构造不出查询，也认不出回应。

use crate::magicsock::MagicSock;
use galaxy_proto::wire::{LanConnect, LanQuery, LanReply, LAN_MULTICAST_V4, LAN_MULTICAST_V6};
use galaxy_proto::{DeviceId, LAN_PORT};
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn mac_ok(key: &[u8; 32], input: &[u8], mac: &[u8; 16]) -> bool {
    galaxy_crypto::ct_eq(&galaxy_crypto::lan_mac(key, input), mac)
}

fn reuse_udp(addr: SocketAddr) -> std::io::Result<tokio::net::UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let domain = if addr.is_ipv6() { Domain::IPV6 } else { Domain::IPV4 };
    let s = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_reuse_address(true)?;
    #[cfg(all(unix, not(any(target_os = "solaris", target_os = "illumos"))))]
    {
        let _ = s.set_reuse_port(true);
    }
    if addr.is_ipv6() {
        let _ = s.set_only_v6(true);
    }
    let _ = s.set_broadcast(true);
    s.bind(&addr.into())?;
    s.set_nonblocking(true)?;
    tokio::net::UdpSocket::from_std(s.into())
}

/// 离线连接请求的回调：(主控端设备, 连接 ID, 主控端魔法套接字地址)。
pub type OnLanConnect = Arc<dyn Fn(DeviceId, [u8; 16], SocketAddr) + Send + Sync>;

/// 被控端：监听局域网查询并回应。`keys` 为本机登录的各账号的 lan_key，可随登录变化更新。
pub fn spawn_responder(ms: Arc<MagicSock>, device_id: DeviceId, keys: Arc<RwLock<Vec<[u8; 32]>>>) {
    spawn_responder_with(ms, device_id, keys, None)
}

pub fn spawn_responder_with(ms: Arc<MagicSock>, device_id: DeviceId, keys: Arc<RwLock<Vec<[u8; 32]>>>, on_connect: Option<OnLanConnect>) {
    spawn_responder_until(ms, device_id, keys, on_connect, None)
}

/// 同 [`spawn_responder_with`]，`stop` 变成 true（或发送端丢弃）时两个监听任务退出、释放端口。
pub fn spawn_responder_until(ms: Arc<MagicSock>, device_id: DeviceId, keys: Arc<RwLock<Vec<[u8; 32]>>>, on_connect: Option<OnLanConnect>, stop: Option<tokio::sync::watch::Receiver<bool>>) {
    let stop4 = stop.clone();
    let ms4 = ms.clone();
    let keys4 = keys.clone();
    let oc4 = on_connect.clone();
    tokio::spawn(async move {
        let sock = match reuse_udp(SocketAddr::from(([0, 0, 0, 0], LAN_PORT))) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("局域网发现监听 IPv4 失败: {e}");
                return;
            }
        };
        let mut joined: Vec<Ipv4Addr> = vec![];
        let mut last_join = Instant::now() - Duration::from_secs(3600);
        responder_loop(&sock, &ms4, device_id, &keys4, oc4.as_ref(), stop4, |s| {
            if last_join.elapsed() > Duration::from_secs(30) {
                last_join = Instant::now();
                for i in crate::ifaces::list() {
                    if let IpAddr::V4(v4) = i.ip {
                        if !joined.contains(&v4) && s.join_multicast_v4(LAN_MULTICAST_V4, v4).is_ok() {
                            joined.push(v4);
                        }
                    }
                }
            }
        })
        .await;
    });
    tokio::spawn(async move {
        let sock = match reuse_udp(SocketAddr::from((Ipv6Addr::UNSPECIFIED, LAN_PORT))) {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!("局域网发现监听 IPv6 失败: {e}");
                return;
            }
        };
        let mut joined: Vec<u32> = vec![];
        let mut last_join = Instant::now() - Duration::from_secs(3600);
        responder_loop(&sock, &ms, device_id, &keys, on_connect.as_ref(), stop, |s| {
            if last_join.elapsed() > Duration::from_secs(30) {
                last_join = Instant::now();
                for i in crate::ifaces::list() {
                    if let Some(idx) = i.index {
                        if !joined.contains(&idx) && s.join_multicast_v6(&LAN_MULTICAST_V6, idx).is_ok() {
                            joined.push(idx);
                        }
                    }
                }
            }
        })
        .await;
    });
}

async fn responder_loop<F: FnMut(&tokio::net::UdpSocket)>(
    sock: &tokio::net::UdpSocket,
    ms: &Arc<MagicSock>,
    device_id: DeviceId,
    keys: &Arc<RwLock<Vec<[u8; 32]>>>,
    on_connect: Option<&OnLanConnect>,
    mut stop: Option<tokio::sync::watch::Receiver<bool>>,
    mut rejoin: F,
) {
    let mut buf = vec![0u8; 2048];
    let mut recent: HashMap<IpAddr, (Instant, u32)> = HashMap::new();
    loop {
        if stop.as_ref().is_some_and(|s| *s.borrow()) {
            return;
        }
        rejoin(sock);
        let recv = tokio::time::timeout(Duration::from_secs(30), sock.recv_from(&mut buf));
        let r = match stop.as_mut() {
            Some(s) => tokio::select! {
                r = recv => r,
                c = s.changed() => {
                    if c.is_err() || *s.borrow() {
                        return;
                    }
                    continue;
                }
            },
            None => recv.await,
        };
        let Ok(Ok((n, from))) = r else { continue };
        // 离线连接请求：校验后通知被控端建立路径，并像查询一样回应地址
        let q = if let Some(c) = LanConnect::decode(&buf[..n]) {
            if c.target != device_id.0 {
                continue;
            }
            let input = LanConnect::mac_input(&c.conn_id, &c.target, &c.controller, c.reply_port);
            if !keys.read().iter().any(|k| mac_ok(k, &input, &c.mac)) {
                continue;
            }
            let ctl_addr = match from {
                SocketAddr::V6(v6) => match v6.ip().to_ipv4_mapped() {
                    Some(v4) => SocketAddr::new(IpAddr::V4(v4), c.reply_port),
                    None => SocketAddr::V6(SocketAddrV6::new(*v6.ip(), c.reply_port, 0, v6.scope_id())),
                },
                SocketAddr::V4(v4) => SocketAddr::new(IpAddr::V4(*v4.ip()), c.reply_port),
            };
            if let Some(cb) = on_connect {
                cb(DeviceId(c.controller), c.conn_id, ctl_addr);
            }
            // 用 conn_id 当 nonce 回应，主控端据此拿到被控端地址
            let key = keys.read().iter().copied().find(|k| mac_ok(k, &input, &c.mac)).unwrap_or([0; 32]);
            let mac = galaxy_crypto::lan_mac(&key, &LanQuery::mac_input(&c.conn_id, &c.target, c.reply_port));
            LanQuery { nonce: c.conn_id, target: c.target, reply_port: c.reply_port, mac }
        } else {
            let Some(q) = LanQuery::decode(&buf[..n]) else { continue };
            q
        };
        if q.target != [0u8; 16] && q.target != device_id.0 {
            continue;
        }
        // 每个来源每秒最多回应 10 次
        let e = recent.entry(from.ip()).or_insert((Instant::now(), 0));
        if e.0.elapsed() > Duration::from_secs(1) {
            *e = (Instant::now(), 0);
        }
        e.1 += 1;
        if e.1 > 10 {
            continue;
        }
        if recent.len() > 1024 {
            recent.clear();
        }
        let input = LanQuery::mac_input(&q.nonce, &q.target, q.reply_port);
        let key = keys.read().iter().copied().find(|k| mac_ok(k, &input, &q.mac));
        let Some(key) = key else { continue };
        let port = ms.port();
        let addrs: Vec<SocketAddr> = crate::ifaces::list().into_iter().map(|i| SocketAddr::new(i.ip, port)).collect();
        let mac = galaxy_crypto::lan_mac(&key, &LanReply::mac_input(&q.nonce, &device_id.0, port, &addrs));
        let reply = LanReply { nonce: q.nonce, device_id: device_id.0, port, addrs, mac }.encode();
        let to = match from {
            SocketAddr::V6(v6) => match v6.ip().to_ipv4_mapped() {
                Some(v4) => SocketAddr::new(IpAddr::V4(v4), q.reply_port),
                None => SocketAddr::V6(SocketAddrV6::new(*v6.ip(), q.reply_port, 0, v6.scope_id())),
            },
            SocketAddr::V4(v4) => SocketAddr::V4(SocketAddrV4::new(*v4.ip(), q.reply_port)),
        };
        let _ = ms.send_raw(&reply, to).await;
    }
}

/// 发现到的设备。
#[derive(Clone, Debug)]
pub struct LanSeen {
    pub device: DeviceId,
    pub addrs: Vec<SocketAddr>,
    pub at: Instant,
}

/// 主控端：发查询、收回应。
pub struct LanDiscovery {
    ms: Arc<MagicSock>,
    pending: Mutex<HashMap<[u8; 16], ([u8; 32], Instant)>>,
    pub seen: RwLock<HashMap<DeviceId, LanSeen>>,
    notify: tokio::sync::broadcast::Sender<LanSeen>,
}

impl LanDiscovery {
    pub fn new(ms: Arc<MagicSock>) -> Arc<Self> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        ms.set_lan_reply_sink(tx);
        let (notify, _) = tokio::sync::broadcast::channel(64);
        let me = Arc::new(Self { ms, pending: Default::default(), seen: Default::default(), notify });
        let weak = Arc::downgrade(&me);
        tokio::spawn(async move {
            while let Some((reply, from)) = rx.recv().await {
                let Some(me) = weak.upgrade() else { break };
                me.on_reply(reply, from);
            }
        });
        me
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<LanSeen> {
        self.notify.subscribe()
    }

    fn on_reply(&self, r: LanReply, from: SocketAddr) {
        let key = {
            let p = self.pending.lock();
            p.get(&r.nonce).map(|(k, _)| *k)
        };
        let Some(key) = key else { return };
        if !mac_ok(&key, &LanReply::mac_input(&r.nonce, &r.device_id, r.port, &r.addrs), &r.mac) {
            return;
        }
        let dev = DeviceId(r.device_id);
        let mut addrs = vec![SocketAddr::new(from.ip(), r.port)];
        for a in r.addrs {
            if !addrs.contains(&a) {
                addrs.push(a);
            }
        }
        let seen = LanSeen { device: dev, addrs, at: Instant::now() };
        self.seen.write().insert(dev, seen.clone());
        let _ = self.notify.send(seen);
    }

    /// 发一轮查询。`target` 为空表示查同账号的所有设备。
    pub async fn query(&self, lan_key: [u8; 32], target: Option<DeviceId>) {
        let nonce: [u8; 16] = rand::random();
        let tgt = target.map(|d| d.0).unwrap_or([0u8; 16]);
        let reply_port = self.ms.port();
        let mac = galaxy_crypto::lan_mac(&lan_key, &LanQuery::mac_input(&nonce, &tgt, reply_port));
        {
            let mut p = self.pending.lock();
            p.retain(|_, (_, t)| t.elapsed() < Duration::from_secs(30));
            p.insert(nonce, (lan_key, Instant::now()));
        }
        let pkt = LanQuery { nonce, target: tgt, reply_port, mac }.encode();
        send_query(&pkt).await;
    }

    /// 离线局域网连接请求（服务器不可达时用）。回应会像查询一样进入 `seen` 并触发通知。
    pub async fn connect_request(&self, lan_key: [u8; 32], target: DeviceId, controller: DeviceId, conn_id: [u8; 16]) {
        let reply_port = self.ms.port();
        let mac = galaxy_crypto::lan_mac(&lan_key, &LanConnect::mac_input(&conn_id, &target.0, &controller.0, reply_port));
        {
            let mut p = self.pending.lock();
            p.retain(|_, (_, t)| t.elapsed() < Duration::from_secs(30));
            p.insert(conn_id, (lan_key, Instant::now()));
        }
        let pkt = LanConnect { conn_id, target: target.0, controller: controller.0, reply_port, mac }.encode();
        send_query(&pkt).await;
        // 之前见过的地址直接单播一份
        let known: Vec<SocketAddr> = self.seen.read().get(&target).map(|s| s.addrs.clone()).unwrap_or_default();
        if let Ok(s) = reuse_udp(SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0))).or_else(|_| reuse_udp(SocketAddr::from(([0, 0, 0, 0], 0)))) {
            for a in known {
                let _ = s.send_to(&pkt, SocketAddr::new(a.ip(), LAN_PORT)).await;
            }
        }
    }

    pub fn lookup(&self, d: &DeviceId, max_age: Duration) -> Option<LanSeen> {
        self.seen.read().get(d).filter(|s| s.at.elapsed() < max_age).cloned()
    }
}

async fn send_query(pkt: &[u8]) {
    let ifaces = crate::ifaces::list();
    if let Ok(s) = reuse_udp(SocketAddr::from(([0, 0, 0, 0], 0))) {
        let _ = s.send_to(pkt, (Ipv4Addr::BROADCAST, LAN_PORT)).await;
        for i in &ifaces {
            if let Some(b) = i.broadcast {
                let _ = s.send_to(pkt, (b, LAN_PORT)).await;
            }
        }
    }
    // 组播：逐个网卡发
    for i in &ifaces {
        if let IpAddr::V4(v4) = i.ip {
            if let Ok(s) = reuse_udp(SocketAddr::from((v4, 0))) {
                let sref = socket2::SockRef::from(&s);
                let _ = sref.set_multicast_if_v4(&v4);
                let _ = sref.set_multicast_ttl_v4(1);
                let _ = s.send_to(pkt, (LAN_MULTICAST_V4, LAN_PORT)).await;
            }
        }
    }
    if let Ok(s) = reuse_udp(SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0))) {
        let mut done = vec![];
        for i in &ifaces {
            if let (IpAddr::V6(_), Some(idx)) = (i.ip, i.index) {
                if done.contains(&idx) {
                    continue;
                }
                done.push(idx);
                let _ = socket2::SockRef::from(&s).set_multicast_if_v6(idx);
                let _ = s.send_to(pkt, SocketAddrV6::new(LAN_MULTICAST_V6, LAN_PORT, 0, idx)).await;
            }
        }
    }
}
