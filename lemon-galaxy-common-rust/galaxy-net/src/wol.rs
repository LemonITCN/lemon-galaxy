//! 远程开机（Wake-on-LAN）：向本机所有局域网广播魔术包。

/// 向本机各网卡的广播地址发魔术包（UDP 9 / 7 端口）。
pub async fn send_magic(mac: &str) -> Result<(), String> {
    let norm = galaxy_proto::normalize_mac(mac).ok_or("MAC 地址格式错误")?;
    let bytes: Vec<u8> = norm.split(':').map(|h| u8::from_str_radix(h, 16).unwrap_or(0)).collect();
    let mut pkt = vec![0xffu8; 6];
    for _ in 0..16 {
        pkt.extend_from_slice(&bytes);
    }
    let s = tokio::net::UdpSocket::bind("0.0.0.0:0").await.map_err(|e| e.to_string())?;
    s.set_broadcast(true).map_err(|e| e.to_string())?;
    let mut sent = 0;
    for port in [9u16, 7] {
        if s.send_to(&pkt, ("255.255.255.255", port)).await.is_ok() {
            sent += 1;
        }
    }
    for i in crate::ifaces::list() {
        if let Some(b) = i.broadcast {
            if s.send_to(&pkt, (b, 9)).await.is_ok() {
                sent += 1;
            }
        }
    }
    if sent == 0 {
        return Err("没有可用的局域网".into());
    }
    Ok(())
}
