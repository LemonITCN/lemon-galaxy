//! 本机网卡地址：用于 host 候选与局域网广播。

use std::net::{IpAddr, Ipv4Addr};

#[derive(Clone, Debug)]
pub struct Iface {
    pub name: String,
    pub ip: IpAddr,
    pub broadcast: Option<Ipv4Addr>,
    pub index: Option<u32>,
    pub virtual_like: bool,
}

const VIRTUAL_PREFIXES: &[&str] = &[
    "docker", "br-", "veth", "vmnet", "vboxnet", "virbr", "utun", "tun", "tap", "zt", "tailscale", "wg", "ham", "llw", "awdl", "anpi", "bridge", "cni", "flannel", "kube", "lxc", "lxd",
];

pub fn is_virtual_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    VIRTUAL_PREFIXES.iter().any(|p| n.starts_with(p)) || n.contains("virtual") || n.contains("vethernet") || n.contains("vpn")
}

/// 本机可用于连接的地址（排除回环、IPv4 链路本地、IPv6 链路本地）。
pub fn list() -> Vec<Iface> {
    let mut out = vec![];
    let Ok(all) = if_addrs::get_if_addrs() else { return out };
    for i in all {
        if i.is_loopback() {
            continue;
        }
        let ip = i.ip();
        match ip {
            IpAddr::V4(v4) => {
                if v4.is_link_local() || v4.is_unspecified() || v4.is_multicast() {
                    continue;
                }
            }
            IpAddr::V6(v6) => {
                let seg0 = v6.segments()[0];
                if v6.is_unspecified() || v6.is_multicast() || (seg0 & 0xffc0) == 0xfe80 {
                    continue;
                }
            }
        }
        let broadcast = match &i.addr {
            if_addrs::IfAddr::V4(a) => a.broadcast.or_else(|| {
                let ipn = u32::from(a.ip);
                let mask = u32::from(a.netmask);
                if mask == 0 || mask == u32::MAX {
                    None
                } else {
                    Some(Ipv4Addr::from(ipn | !mask))
                }
            }),
            _ => None,
        };
        out.push(Iface { virtual_like: is_virtual_name(&i.name), name: i.name.clone(), ip, broadcast, index: i.index });
    }
    out
}

/// 用于远程开机的网卡 MAC（有私网 IPv4、不像虚拟网卡的），格式 `AA:BB:CC:DD:EE:FF`。手机端返回空。
#[cfg(any(target_os = "ios", target_os = "android"))]
pub fn wake_macs() -> Vec<String> {
    vec![]
}

/// 用于远程开机的网卡 MAC（有私网 IPv4、不像虚拟网卡的），格式 `AA:BB:CC:DD:EE:FF`。
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub fn wake_macs() -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for i in list() {
        let IpAddr::V4(v4) = i.ip else { continue };
        if i.virtual_like || !v4.is_private() {
            continue;
        }
        if let Ok(Some(m)) = mac_address::mac_address_by_name(&i.name) {
            let b = m.bytes();
            // 全零与本地管理地址（随机化 MAC、虚拟网卡）唤醒不了
            if b == [0; 6] || b[0] & 0x02 != 0 {
                continue;
            }
            let s = m.to_string().to_ascii_uppercase();
            if !out.contains(&s) {
                out.push(s);
            }
        }
        if out.len() >= 8 {
            break;
        }
    }
    out
}

/// 是否私网 / 局域网地址。
pub fn is_private(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local() || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 64),
        IpAddr::V6(v6) => {
            let s0 = v6.segments()[0];
            (s0 & 0xfe00) == 0xfc00 || (s0 & 0xffc0) == 0xfe80
        }
    }
}
