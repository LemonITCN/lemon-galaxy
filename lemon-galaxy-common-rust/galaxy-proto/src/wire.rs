//! UDP 包格式：服务器探测与中转、两端探测（disco）、局域网发现。
//! 所有自有包首字节都是 0x00（合法 QUIC 包固定位必为 1，不会冲突）。

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

pub const SERVER_MAGIC: [u8; 4] = [0, b'L', b'G', b'S'];
pub const DISCO_MAGIC: [u8; 4] = [0, b'L', b'G', b'D'];
pub const LAN_QUERY_MAGIC: [u8; 4] = [0, b'L', b'G', b'Q'];
pub const LAN_REPLY_MAGIC: [u8; 4] = [0, b'L', b'G', b'R'];
/// 离线局域网连接请求（服务器不可达时主控端直接请求被控端）
pub const LAN_CONNECT_MAGIC: [u8; 4] = [0, b'L', b'G', b'C'];

pub const LAN_MULTICAST_V4: Ipv4Addr = Ipv4Addr::new(239, 255, 71, 71);
pub const LAN_MULTICAST_V6: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 0x4c47);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketClass {
    Server,
    Disco,
    LanQuery,
    LanReply,
    LanConnect,
    /// 不是自有包，交给 QUIC
    Quic,
    Junk,
}

pub fn classify(b: &[u8]) -> PacketClass {
    if b.is_empty() {
        return PacketClass::Junk;
    }
    if b[0] != 0 {
        // 端点已关闭固定位混淆，合法 QUIC 包首字节不会是 0；其余一律交给 QUIC 判断
        return PacketClass::Quic;
    }
    if b.len() < 5 {
        return PacketClass::Junk;
    }
    match [b[0], b[1], b[2], b[3]] {
        SERVER_MAGIC => PacketClass::Server,
        DISCO_MAGIC => PacketClass::Disco,
        LAN_QUERY_MAGIC => PacketClass::LanQuery,
        LAN_REPLY_MAGIC => PacketClass::LanReply,
        LAN_CONNECT_MAGIC => PacketClass::LanConnect,
        _ => PacketClass::Junk,
    }
}

// ---------------- 地址编码 ----------------

pub fn put_addr(out: &mut Vec<u8>, a: &SocketAddr) {
    match a.ip() {
        IpAddr::V4(v4) => {
            out.push(4);
            out.extend_from_slice(&v4.octets());
        }
        IpAddr::V6(v6) => {
            out.push(6);
            out.extend_from_slice(&v6.octets());
        }
    }
    out.extend_from_slice(&a.port().to_be_bytes());
}

pub fn get_addr(b: &[u8]) -> Option<(SocketAddr, usize)> {
    let fam = *b.first()?;
    match fam {
        4 => {
            if b.len() < 1 + 4 + 2 {
                return None;
            }
            let ip = Ipv4Addr::new(b[1], b[2], b[3], b[4]);
            let port = u16::from_be_bytes([b[5], b[6]]);
            Some((SocketAddr::new(IpAddr::V4(ip), port), 7))
        }
        6 => {
            if b.len() < 1 + 16 + 2 {
                return None;
            }
            let oct: [u8; 16] = b[1..17].try_into().ok()?;
            let port = u16::from_be_bytes([b[17], b[18]]);
            Some((SocketAddr::new(IpAddr::V6(Ipv6Addr::from(oct)), port), 19))
        }
        _ => None,
    }
}

// ---------------- 服务器包 ----------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerPacket {
    BindRequest { txn: [u8; 12] },
    BindResponse { txn: [u8; 12], observed: SocketAddr },
    RelayBind { token: [u8; 16] },
    RelayBound { token: [u8; 16] },
    RelayData { token: [u8; 16], payload: Vec<u8> },
    RelayDeliver { token: [u8; 16], payload: Vec<u8> },
    Keepalive { token: [u8; 16] },
}

impl ServerPacket {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(64);
        o.extend_from_slice(&SERVER_MAGIC);
        match self {
            Self::BindRequest { txn } => {
                o.push(0x01);
                o.extend_from_slice(txn);
            }
            Self::BindResponse { txn, observed } => {
                o.push(0x02);
                o.extend_from_slice(txn);
                put_addr(&mut o, observed);
            }
            Self::RelayBind { token } => {
                o.push(0x10);
                o.extend_from_slice(token);
            }
            Self::RelayBound { token } => {
                o.push(0x11);
                o.extend_from_slice(token);
            }
            Self::RelayData { token, payload } => {
                o.reserve(payload.len());
                o.push(0x12);
                o.extend_from_slice(token);
                o.extend_from_slice(payload);
            }
            Self::RelayDeliver { token, payload } => {
                o.reserve(payload.len());
                o.push(0x13);
                o.extend_from_slice(token);
                o.extend_from_slice(payload);
            }
            Self::Keepalive { token } => {
                o.push(0x20);
                o.extend_from_slice(token);
            }
        }
        o
    }

    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < 5 || b[..4] != SERVER_MAGIC {
            return None;
        }
        let t = b[4];
        let r = &b[5..];
        let tok = |r: &[u8]| -> Option<[u8; 16]> { r.get(..16)?.try_into().ok() };
        Some(match t {
            0x01 => Self::BindRequest { txn: r.get(..12)?.try_into().ok()? },
            0x02 => {
                let txn: [u8; 12] = r.get(..12)?.try_into().ok()?;
                let (observed, _) = get_addr(&r[12..])?;
                Self::BindResponse { txn, observed }
            }
            0x10 => Self::RelayBind { token: tok(r)? },
            0x11 => Self::RelayBound { token: tok(r)? },
            0x12 => Self::RelayData { token: tok(r)?, payload: r[16..].to_vec() },
            0x13 => Self::RelayDeliver { token: tok(r)?, payload: r[16..].to_vec() },
            0x20 => Self::Keepalive { token: tok(r)? },
            _ => return None,
        })
    }
}

/// 只解析中转数据的头，避免复制负载。返回 (类型, 令牌, 负载起始)。
pub fn peek_relay(b: &[u8]) -> Option<(u8, [u8; 16], usize)> {
    if b.len() < 21 || b[..4] != SERVER_MAGIC {
        return None;
    }
    let t = b[4];
    if t != 0x12 && t != 0x13 {
        return None;
    }
    Some((t, b[5..21].try_into().ok()?, 21))
}

/// 直接拼一个 RelayDeliver 包。
pub fn relay_deliver(token: &[u8; 16], payload: &[u8]) -> Vec<u8> {
    let mut o = Vec::with_capacity(21 + payload.len());
    o.extend_from_slice(&SERVER_MAGIC);
    o.push(0x13);
    o.extend_from_slice(token);
    o.extend_from_slice(payload);
    o
}

/// 直接拼一个 RelayData 包。
pub fn relay_data(token: &[u8; 16], payload: &[u8]) -> Vec<u8> {
    let mut o = Vec::with_capacity(21 + payload.len());
    o.extend_from_slice(&SERVER_MAGIC);
    o.push(0x12);
    o.extend_from_slice(token);
    o.extend_from_slice(payload);
    o
}

// ---------------- 两端探测 ----------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Disco {
    Ping { conn_tag: [u8; 8], txn: [u8; 12] },
    Pong { conn_tag: [u8; 8], txn: [u8; 12], observed: SocketAddr },
}

impl Disco {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(48);
        o.extend_from_slice(&DISCO_MAGIC);
        match self {
            Self::Ping { conn_tag, txn } => {
                o.push(0x01);
                o.extend_from_slice(conn_tag);
                o.extend_from_slice(txn);
            }
            Self::Pong { conn_tag, txn, observed } => {
                o.push(0x02);
                o.extend_from_slice(conn_tag);
                o.extend_from_slice(txn);
                put_addr(&mut o, observed);
            }
        }
        o
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < 5 + 8 + 12 || b[..4] != DISCO_MAGIC {
            return None;
        }
        let conn_tag: [u8; 8] = b[5..13].try_into().ok()?;
        let txn: [u8; 12] = b[13..25].try_into().ok()?;
        match b[4] {
            0x01 => Some(Self::Ping { conn_tag, txn }),
            0x02 => {
                let (observed, _) = get_addr(&b[25..])?;
                Some(Self::Pong { conn_tag, txn, observed })
            }
            _ => None,
        }
    }
}

// ---------------- 局域网发现 ----------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanQuery {
    pub nonce: [u8; 16],
    /// 全 0 表示同账号任意设备
    pub target: [u8; 16],
    pub reply_port: u16,
    pub mac: [u8; 16],
}

impl LanQuery {
    pub fn mac_input(nonce: &[u8; 16], target: &[u8; 16], reply_port: u16) -> Vec<u8> {
        let mut v = Vec::with_capacity(1 + 16 + 16 + 2);
        v.push(b'q');
        v.extend_from_slice(nonce);
        v.extend_from_slice(target);
        v.extend_from_slice(&reply_port.to_be_bytes());
        v
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(60);
        o.extend_from_slice(&LAN_QUERY_MAGIC);
        o.push(1);
        o.extend_from_slice(&self.nonce);
        o.extend_from_slice(&self.target);
        o.extend_from_slice(&self.reply_port.to_be_bytes());
        o.extend_from_slice(&self.mac);
        o
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < 5 + 16 + 16 + 2 + 16 || b[..4] != LAN_QUERY_MAGIC || b[4] != 1 {
            return None;
        }
        Some(Self {
            nonce: b[5..21].try_into().ok()?,
            target: b[21..37].try_into().ok()?,
            reply_port: u16::from_be_bytes([b[37], b[38]]),
            mac: b[39..55].try_into().ok()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanReply {
    pub nonce: [u8; 16],
    pub device_id: [u8; 16],
    pub port: u16,
    pub addrs: Vec<SocketAddr>,
    pub mac: [u8; 16],
}

impl LanReply {
    fn body(nonce: &[u8; 16], device_id: &[u8; 16], port: u16, addrs: &[SocketAddr]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(nonce);
        v.extend_from_slice(device_id);
        v.extend_from_slice(&port.to_be_bytes());
        v.push(addrs.len().min(32) as u8);
        for a in addrs.iter().take(32) {
            put_addr(&mut v, a);
        }
        v
    }
    pub fn mac_input(nonce: &[u8; 16], device_id: &[u8; 16], port: u16, addrs: &[SocketAddr]) -> Vec<u8> {
        let mut v = vec![b'r'];
        v.extend_from_slice(&Self::body(nonce, device_id, port, addrs));
        v
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::new();
        o.extend_from_slice(&LAN_REPLY_MAGIC);
        o.push(1);
        o.extend_from_slice(&Self::body(&self.nonce, &self.device_id, self.port, &self.addrs));
        o.extend_from_slice(&self.mac);
        o
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < 5 + 16 + 16 + 2 + 1 + 16 || b[..4] != LAN_REPLY_MAGIC || b[4] != 1 {
            return None;
        }
        let nonce: [u8; 16] = b[5..21].try_into().ok()?;
        let device_id: [u8; 16] = b[21..37].try_into().ok()?;
        let port = u16::from_be_bytes([b[37], b[38]]);
        let n = b[39] as usize;
        let mut pos = 40;
        let mut addrs = Vec::with_capacity(n);
        for _ in 0..n {
            let (a, used) = get_addr(&b[pos..])?;
            addrs.push(a);
            pos += used;
        }
        let mac: [u8; 16] = b.get(pos..pos + 16)?.try_into().ok()?;
        Some(Self { nonce, device_id, port, addrs, mac })
    }
}

/// 离线局域网连接请求。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanConnect {
    pub conn_id: [u8; 16],
    pub target: [u8; 16],
    pub controller: [u8; 16],
    /// 主控端魔法套接字端口
    pub reply_port: u16,
    pub mac: [u8; 16],
}

impl LanConnect {
    pub fn mac_input(conn_id: &[u8; 16], target: &[u8; 16], controller: &[u8; 16], reply_port: u16) -> Vec<u8> {
        let mut v = vec![b'c'];
        v.extend_from_slice(conn_id);
        v.extend_from_slice(target);
        v.extend_from_slice(controller);
        v.extend_from_slice(&reply_port.to_be_bytes());
        v
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(72);
        o.extend_from_slice(&LAN_CONNECT_MAGIC);
        o.push(1);
        o.extend_from_slice(&self.conn_id);
        o.extend_from_slice(&self.target);
        o.extend_from_slice(&self.controller);
        o.extend_from_slice(&self.reply_port.to_be_bytes());
        o.extend_from_slice(&self.mac);
        o
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < 5 + 48 + 2 + 16 || b[..4] != LAN_CONNECT_MAGIC || b[4] != 1 {
            return None;
        }
        Some(Self {
            conn_id: b[5..21].try_into().ok()?,
            target: b[21..37].try_into().ok()?,
            controller: b[37..53].try_into().ok()?,
            reply_port: u16::from_be_bytes([b[53], b[54]]),
            mac: b[55..71].try_into().ok()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lan_connect() {
        let c = LanConnect { conn_id: [1; 16], target: [2; 16], controller: [3; 16], reply_port: 4321, mac: [9; 16] };
        let e = c.encode();
        assert_eq!(classify(&e), PacketClass::LanConnect);
        assert_eq!(LanConnect::decode(&e), Some(c));
    }

    #[test]
    fn server_packets() {
        let a: SocketAddr = "[240e::1]:5000".parse().unwrap();
        for p in [
            ServerPacket::BindRequest { txn: [7; 12] },
            ServerPacket::BindResponse { txn: [7; 12], observed: a },
            ServerPacket::RelayData { token: [3; 16], payload: b"hello".to_vec() },
            ServerPacket::RelayDeliver { token: [3; 16], payload: vec![] },
            ServerPacket::Keepalive { token: [9; 16] },
        ] {
            let e = p.encode();
            assert_eq!(classify(&e), PacketClass::Server);
            assert_eq!(ServerPacket::decode(&e), Some(p));
        }
    }

    #[test]
    fn disco_and_lan() {
        let a: SocketAddr = "192.168.1.9:7000".parse().unwrap();
        let p = Disco::Pong { conn_tag: [1; 8], txn: [2; 12], observed: a };
        let e = p.encode();
        assert_eq!(classify(&e), PacketClass::Disco);
        assert_eq!(Disco::decode(&e), Some(p));

        let q = LanQuery { nonce: [5; 16], target: [0; 16], reply_port: 5555, mac: [6; 16] };
        assert_eq!(LanQuery::decode(&q.encode()), Some(q));
        let r = LanReply { nonce: [5; 16], device_id: [8; 16], port: 6000, addrs: vec![a, "[fe80::1]:6000".parse().unwrap()], mac: [4; 16] };
        assert_eq!(LanReply::decode(&r.encode()), Some(r));
    }

    #[test]
    fn quic_is_not_ours() {
        assert_eq!(classify(&[0xc3, 0, 0, 0, 1, 2]), PacketClass::Quic);
        assert_eq!(classify(&[0x41, 1, 2, 3, 4, 5]), PacketClass::Quic);
        assert_eq!(classify(&[0x01, 1, 2, 3, 4, 5]), PacketClass::Quic);
    }
}
