//! 远程开机：被控设备离线时，找同一网络里在线的设备代发唤醒包（被控端要在主板 / 网卡设置里开启网络唤醒）。

use crate::connector::Connector;
use crate::error::{ClientError, Result};
use galaxy_proto::api::ClientDevice;
use galaxy_proto::peer::MgmtReq;
use galaxy_proto::DeviceId;
use std::sync::Arc;

fn net_ip(d: &ClientDevice) -> &str {
    if !d.public_ip.is_empty() {
        &d.public_ip
    } else {
        &d.last_ip
    }
}

/// 能不能对这台设备发起远程开机：离线、上报过网卡地址、能被控。
pub fn can_wake(d: &ClientDevice) -> bool {
    !d.online && !d.macs.is_empty() && d.kind.can_host()
}

/// 代发唤醒包。优先用本机（与目标在同一网络时），再用同一网络里在线的能被控设备（最多两台）。
/// 返回实际代发的设备名（本机为 None 项之外的名字列表）。
pub async fn wake(connector: &Arc<Connector>, devices: &[ClientDevice], me: DeviceId, target: DeviceId) -> Result<Vec<String>> {
    let t = devices.iter().find(|d| d.id == target).ok_or_else(|| ClientError::Other("找不到这台设备".into()))?;
    if t.macs.is_empty() {
        return Err(ClientError::Other("这台设备还没有上报网卡地址（升级到新版本并上线一次后才能远程开机）".into()));
    }
    let tip = net_ip(t);
    if tip.is_empty() {
        return Err(ClientError::Other("不知道这台设备所在的网络".into()));
    }
    let mut via: Vec<String> = vec![];
    // 本机与目标在同一网络：直接从本机广播
    if devices.iter().any(|d| d.id == me && net_ip(d) == tip) {
        let mut ok = false;
        for m in &t.macs {
            ok |= galaxy_net::wol::send_magic(m).await.is_ok();
        }
        if ok {
            via.push(String::new());
        }
    }
    let relays: Vec<&ClientDevice> = devices.iter().filter(|d| d.id != target && d.id != me && d.online && d.kind.can_host() && net_ip(d) == tip).take(2).collect();
    let mut last_err = None;
    for r in relays {
        match connector.connect(r.id).await {
            Ok(s) => {
                let mut ok = false;
                for m in &t.macs {
                    match s.mgmt(MgmtReq::WakeOnLan { mac: m.clone() }).await {
                        Ok(galaxy_proto::peer::MgmtResp::Error(e)) => last_err = Some(e.message),
                        Ok(_) => ok = true,
                        Err(e) => last_err = Some(e.to_string()),
                    }
                }
                s.close();
                if ok {
                    via.push(if r.hostname.is_empty() { r.id.to_hex()[..8].to_string() } else { r.hostname.clone() });
                }
            }
            Err(e) => last_err = Some(e.to_string()),
        }
    }
    if via.is_empty() {
        return Err(ClientError::Other(match last_err {
            Some(e) => format!("代发唤醒包失败：{e}"),
            None => "同一网络里没有在线的设备可以代发唤醒包".into(),
        }));
    }
    Ok(via)
}
