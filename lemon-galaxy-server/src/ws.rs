//! WebSocket 信令（/api/v1/ws）与 TCP 中转（/api/v1/relay），见 docs/protocol.md。

use crate::audit::{self, Target};
use crate::auth::Ctx;
use crate::state::{bump_stat, ConnInfo, OnlineDevice, RelayEnd, RelayPair, St};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::Response;
use futures::{SinkExt, StreamExt};
use galaxy_proto::cert::{ConnTicket, WS_AUTH_DOMAIN};
use galaxy_proto::ws::WsMsg;
use galaxy_proto::{ConnId, ConnMode, DeviceId, DeviceKind, PathKind, Platform, UserBrief};
use r2d2_sqlite::rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::json;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Deserialize, Default)]
pub struct WsQuery {
    assist: Option<u8>,
    kind: Option<String>,
    platform: Option<String>,
    hostname: Option<String>,
    os_version: Option<String>,
    arch: Option<String>,
    app_version: Option<String>,
    /// client = 主控端（带用户会话）；缺省为被控端（设备身份）
    role: Option<String>,
}

pub async fn ws_handler(State(st): State<St>, ctx: Ctx, Query(q): Query<WsQuery>, ws: WebSocketUpgrade) -> Response {
    let ip: IpAddr = ctx.ip.parse().unwrap_or(IpAddr::from([0, 0, 0, 0]));
    ws.max_message_size(256 * 1024).on_upgrade(move |sock| async move {
        if let Err(e) = run_ws(st, sock, ip, q).await {
            tracing::debug!("信令连接结束: {e}");
        }
    })
}

/// 读取某设备当前「有效」的账号。
pub async fn active_accounts(st: &St, id: DeviceId) -> Vec<u64> {
    let raw = id.0.to_vec();
    st.db
        .call(move |c| {
            let mut stmt = c.prepare(
                "SELECT da.user_id FROM device_accounts da JOIN users u ON u.id = da.user_id
                 WHERE da.device_id = ?1 AND da.status = 'active' AND u.disabled = 0",
            )?;
            let v = stmt.query_map(params![raw], |r| r.get::<_, i64>(0))?.collect::<Result<Vec<_>, _>>()?;
            Ok(v.into_iter().map(|x| x as u64).collect())
        })
        .await
        .unwrap_or_default()
}

/// 设备账号变化后刷新在线表，并通知相关账号。
pub async fn refresh_device_accounts(st: &St, id: DeviceId) {
    let new = active_accounts(st, id).await;
    let old = st.hub.devices.read().get(&id).map(|d| d.accounts.clone());
    if let Some(old) = old {
        st.hub.set_accounts(&id, new.clone());
        for u in old.iter().chain(new.iter()) {
            st.hub.notify_user(*u, WsMsg::DevicesChanged);
        }
    }
}

async fn run_ws(st: St, sock: WebSocket, ip: IpAddr, q: WsQuery) -> anyhow::Result<()> {
    let (mut sink, mut stream) = sock.split();
    let nonce = galaxy_crypto::random_bytes::<32>().to_vec();
    sink.send(Message::Text(WsMsg::Hello { nonce: nonce.clone(), server_time: galaxy_proto::now_ms() }.to_json().into())).await?;

    // 等待认证
    let auth = tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(m) = stream.next().await {
            if let Message::Text(t) = m? {
                return Ok::<_, anyhow::Error>(WsMsg::from_json(&t));
            }
        }
        Ok(None)
    })
    .await??;
    let Some(WsMsg::Auth { device_id, signature, device_pubkey }) = auth else {
        anyhow::bail!("未认证");
    };
    let assist = q.assist == Some(1);
    let raw = device_id.0.to_vec();
    let raw2 = raw.clone();
    let stored: Option<(Vec<u8>, String)> = st
        .db
        .call(move |c| c.query_row("SELECT pubkey, kind FROM devices WHERE id = ?1", params![raw2], |r| Ok((r.get(0)?, r.get(1)?))).optional())
        .await?;
    let (pubkey, kind) = match (stored, assist) {
        (Some((pk, kind)), _) => (pk, DeviceKind::parse(&kind).unwrap_or(DeviceKind::Worker)),
        (None, true) => {
            let pk = device_pubkey.clone().ok_or_else(|| anyhow::anyhow!("缺少设备公钥"))?;
            let kind = q.kind.as_deref().and_then(DeviceKind::parse).unwrap_or(DeviceKind::Worker);
            (pk, kind)
        }
        (None, false) => {
            let _ = sink.send(Message::Text(WsMsg::Error { code: "unknown_device".into(), message: String::new() }.to_json().into())).await;
            anyhow::bail!("未登记的设备");
        }
    };
    if let Some(p) = &device_pubkey {
        if p != &pubkey {
            anyhow::bail!("设备公钥不一致");
        }
    }
    if galaxy_crypto::verify(&pubkey, WS_AUTH_DOMAIN, &nonce, &signature).is_err() {
        let _ = sink.send(Message::Text(WsMsg::Error { code: "bad_signature".into(), message: String::new() }.to_json().into())).await;
        anyhow::bail!("设备签名无效");
    }
    if assist && !kind.can_host() {
        anyhow::bail!("该类设备不能被协助");
    }
    // 已登录账号的设备也可以同时开临时协助：账号在线状态照常保留
    let accounts = active_accounts(&st, device_id).await;
    if !assist && accounts.is_empty() {
        let _ = sink.send(Message::Text(WsMsg::Error { code: "no_account".into(), message: String::new() }.to_json().into())).await;
        anyhow::bail!("设备没有有效账号");
    }

    // 协助模式：登记设备（未登录账号的设备也要有记录，用于后台展示）
    let now = galaxy_proto::now_ms();
    if assist {
        let platform = q.platform.as_deref().and_then(Platform::parse).unwrap_or(Platform::Linux);
        let (pk2, raw3, host, os, arch, ver, ipstr) = (
            pubkey.clone(),
            raw.clone(),
            q.hostname.clone().unwrap_or_default(),
            q.os_version.clone().unwrap_or_default(),
            q.arch.clone().unwrap_or_default(),
            q.app_version.clone().unwrap_or_default(),
            ip.to_string(),
        );
        st.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO devices(id, pubkey, kind, platform, os_version, arch, hostname, app_version, last_ip, last_seen_at, created_at)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10) ON CONFLICT(id) DO NOTHING",
                    params![raw3, pk2, kind.as_str(), platform.as_str(), os, arch, host, ver, ipstr, now],
                )
            })
            .await?;
    }
    let ipstr = ip.to_string();
    let raw4 = raw.clone();
    let hostname: String = st
        .db
        .call(move |c| {
            c.execute("UPDATE devices SET last_seen_at = ?1, last_ip = ?2 WHERE id = ?3", params![now, ipstr, raw4])?;
            c.query_row("SELECT hostname FROM devices WHERE id = ?1", params![raw4], |r| r.get(0))
        })
        .await
        .unwrap_or_default();

    let client_role = q.role.as_deref() == Some("client") && !assist;
    let assist_code = if assist { Some(alloc_assist_code(&st, device_id)) } else { None };
    let (tx, mut rx) = mpsc::unbounded_channel::<WsMsg>();
    let serial = st.hub.next_serial();
    let (replaced, came_online) = {
        let mut devs = st.hub.devices.write();
        let conn = crate::state::WsConn { serial, client: client_role, tx: tx.clone() };
        match devs.get_mut(&device_id) {
            Some(d) => {
                let was = d.reachable();
                // 同一角色只保留最新的一条
                let old = d.conns.iter().position(|c| c.client == client_role).map(|i| d.conns.remove(i));
                d.conns.push(conn);
                d.kind = kind;
                d.public_ip = ip;
                d.accounts = accounts.clone();
                d.hostname = hostname.clone();
                if !client_role {
                    if let Some(old_code) = d.assist_code.take() {
                        if assist_code.as_ref() != Some(&old_code) {
                            st.hub.assist_codes.write().remove(&old_code);
                        }
                    }
                    d.assist_code = assist_code.clone();
                }
                (old, !was && d.reachable())
            }
            None => {
                let d = OnlineDevice {
                    conns: vec![conn],
                    kind,
                    public_ip: ip,
                    accounts: accounts.clone(),
                    assist_code: assist_code.clone(),
                    hostname: hostname.clone(),
                    connected_at: now,
                };
                let up = d.reachable();
                devs.insert(device_id, d);
                (None, up)
            }
        }
    };
    if let Some(old) = replaced {
        let _ = old.tx.send(WsMsg::Error { code: "replaced".into(), message: String::new() });
    }
    if came_online {
        for u in &accounts {
            st.hub.notify_user(*u, WsMsg::Presence { device_id, online: true });
        }
    }
    let _ = tx.send(WsMsg::Ready { device_id, public_ip: ip.to_string(), udp_port: st.port, assist_code });

    // 写出任务
    let writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(Duration::from_secs(20));
        ping.tick().await;
        loop {
            tokio::select! {
                m = rx.recv() => {
                    let Some(m) = m else { break };
                    let replaced = matches!(&m, WsMsg::Error { code, .. } if code == "replaced");
                    if sink.send(Message::Text(m.to_json().into())).await.is_err() || replaced {
                        break;
                    }
                }
                _ = ping.tick() => {
                    if sink.send(Message::Ping(Vec::new().into())).await.is_err() { break; }
                }
            }
        }
        let _ = sink.close().await;
    });

    // 读取循环
    let mut last_seen = tokio::time::Instant::now();
    loop {
        let m = tokio::time::timeout(Duration::from_secs(70), stream.next()).await;
        let m = match m {
            Ok(Some(Ok(m))) => m,
            _ => break,
        };
        if last_seen.elapsed() > Duration::from_secs(60) {
            last_seen = tokio::time::Instant::now();
            let (raw5, n) = (raw.clone(), galaxy_proto::now_ms());
            let _ = st.db.call(move |c| c.execute("UPDATE devices SET last_seen_at = ?1 WHERE id = ?2", params![n, raw5])).await;
        }
        match m {
            Message::Text(t) => {
                let Some(msg) = WsMsg::from_json(&t) else { continue };
                if !st.rate.check(&format!("ws:{device_id}"), 200, 10) {
                    continue;
                }
                handle_msg(&st, device_id, kind, ip, msg).await;
            }
            Message::Close(_) => break,
            _ => {}
        }
        if writer.is_finished() {
            break;
        }
    }

    // 下线：去掉这条连接；设备没有连接了才算离线
    let (gone, went_down, accts_now) = {
        let mut devs = st.hub.devices.write();
        let mut gone = None;
        let mut dropped_code = None;
        let mut went_down = false;
        let mut accts_now = vec![];
        if let Some(d) = devs.get_mut(&device_id) {
            let was = d.reachable();
            if let Some(i) = d.conns.iter().position(|c| c.serial == serial) {
                d.conns.remove(i);
                if !client_role {
                    dropped_code = d.assist_code.take();
                }
            }
            // 还剩主控连接但被控连接没了：对别人来说已经连不上
            went_down = was && !d.conns.is_empty() && !d.reachable();
            accts_now = d.accounts.clone();
            if d.conns.is_empty() {
                gone = devs.remove(&device_id);
            }
        }
        if let Some(code) = dropped_code {
            st.hub.assist_codes.write().remove(&code);
        }
        (gone, went_down, accts_now)
    };
    if went_down {
        for u in &accts_now {
            st.hub.notify_user(*u, WsMsg::Presence { device_id, online: false });
        }
    }
    writer.abort();
    if let Some(d) = gone {
        for u in &d.accounts {
            st.hub.notify_user(*u, WsMsg::Presence { device_id, online: false });
        }
        let n = galaxy_proto::now_ms();
        let _ = st.db.call(move |c| c.execute("UPDATE devices SET last_seen_at = ?1 WHERE id = ?2", params![n, raw])).await;
    }
    Ok(())
}

fn alloc_assist_code(st: &St, id: DeviceId) -> String {
    let mut codes = st.hub.assist_codes.write();
    codes.retain(|_, d| *d != id);
    loop {
        let n: u32 = rand::random::<u32>() % 900_000_000 + 100_000_000;
        let code = n.to_string();
        if !codes.contains_key(&code) {
            codes.insert(code.clone(), id);
            return code;
        }
    }
}

async fn handle_msg(st: &St, me: DeviceId, kind: DeviceKind, my_ip: IpAddr, msg: WsMsg) {
    match msg {
        WsMsg::Connect { req_id, target, mode, assist_code } => {
            if let Err((code, message)) = handle_connect(st, me, kind, my_ip, &req_id, target, mode, assist_code).await {
                st.hub.send(&me, WsMsg::ConnectError { req_id, code: code.into(), message });
            }
        }
        WsMsg::Candidates { conn_id, items } => {
            let other = st.conns.read().get(&conn_id).and_then(|c| {
                if c.from_device_id == me {
                    Some(c.to_device_id)
                } else if c.to_device_id == me {
                    Some(c.from_device_id)
                } else {
                    None
                }
            });
            if let Some(o) = other {
                let items: Vec<_> = items.into_iter().take(32).collect();
                st.hub.send(&o, WsMsg::Candidates { conn_id, items });
            }
        }
        WsMsg::ConnState { conn_id, path, rtt_ms: _ } => {
            if let Some(c) = st.conns.write().get_mut(&conn_id) {
                if c.from_device_id == me || c.to_device_id == me {
                    c.path = path;
                }
            }
        }
        WsMsg::ConnClose { conn_id } => {
            let info = {
                let mut conns = st.conns.write();
                match conns.get(&conn_id) {
                    Some(c) if c.from_device_id == me || c.to_device_id == me => conns.remove(&conn_id),
                    _ => None,
                }
            };
            if let Some(c) = info {
                let bytes = st.relay.pairs.read().get(&conn_id).map(|p| p.bytes.load(Ordering::Relaxed)).unwrap_or(0);
                st.relay.remove(&conn_id);
                let other = if c.from_device_id == me { c.to_device_id } else { c.from_device_id };
                st.hub.send(&other, WsMsg::ConnClose { conn_id });
                let actor = audit::Actor { kind: "user", id: c.user_id.to_string(), name: c.username.clone(), ip: String::new() };
                audit::log(st, &actor, "connection.end", Target::new("connection", conn_id.to_hex(), &c.to_device_name),
                    json!({ "relay_bytes": bytes, "duration_ms": galaxy_proto::now_ms() - c.started_at, "path": c.path.as_str() })).await;
            }
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_connect(
    st: &St,
    me: DeviceId,
    kind: DeviceKind,
    my_ip: IpAddr,
    req_id: &str,
    target: Option<DeviceId>,
    mode: ConnMode,
    assist_code: Option<String>,
) -> Result<(), (&'static str, String)> {
    if !kind.can_control() {
        return Err(("forbidden", "该设备不能发起控制".into()));
    }
    if !st.rate.check(&format!("connect:{me}"), 30, 60) {
        return Err(("rate_limited", String::new()));
    }
    let my_accounts = st.hub.devices.read().get(&me).map(|d| d.accounts.clone()).unwrap_or_default();
    let Some(&user_id) = my_accounts.first() else { return Err(("forbidden", String::new())) };

    let target = match mode {
        ConnMode::Control => {
            let t = target.ok_or(("bad_request", String::new()))?;
            let ok = st.hub.devices.read().get(&t).map(|d| (d.accounts.contains(&user_id) && d.kind.can_host(), d.reachable()));
            match ok {
                None | Some((true, false)) => return Err(("offline", String::new())),
                Some((false, _)) => return Err(("forbidden", String::new())),
                Some((true, true)) => t,
            }
        }
        ConnMode::Assist => {
            let code: String = assist_code.unwrap_or_default().chars().filter(|c| c.is_ascii_digit()).collect();
            if !st.rate.check(&format!("assist:{me}"), 10, 60) {
                return Err(("rate_limited", String::new()));
            }
            let t = st.hub.assist_codes.read().get(&code).copied();
            t.ok_or(("not_found", String::new()))?
        }
    };
    if target == me {
        return Err(("bad_request", String::new()));
    }

    let user = {
        let uid = user_id as i64;
        st.db
            .call(move |c| c.query_row("SELECT username, display_name FROM users WHERE id = ?1", params![uid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))))
            .await
            .map_err(|_| ("forbidden", String::new()))?
    };
    let (my_name, target_ip, target_name) = {
        let devs = st.hub.devices.read();
        let m = devs.get(&me).map(|d| d.hostname.clone()).unwrap_or_default();
        let t = devs.get(&target).ok_or(("offline", String::new()))?;
        (m, t.public_ip, t.hostname.clone())
    };

    let conn_id = ConnId::random();
    let pair: Arc<RelayPair> = st.relay.create(conn_id);
    let now = galaxy_proto::now_ms();
    let ticket = ConnTicket {
        v: 1,
        conn_id,
        controller_device: me,
        target_device: target,
        account_id: user_id,
        mode,
        issued_at: now,
        expires_at: now + 120_000,
    };
    let signed = galaxy_crypto::sign_ticket(&st.keys.sign, &ticket);
    st.conns.write().insert(
        conn_id,
        ConnInfo {
            id: conn_id,
            kind: mode,
            user_id,
            username: user.0.clone(),
            from_device_id: me,
            from_device_name: my_name,
            to_device_id: target,
            to_device_name: target_name.clone(),
            path: PathKind::Connecting,
            started_at: now,
        },
    );
    let same = my_ip == target_ip;
    st.hub.send(
        &target,
        WsMsg::Incoming {
            conn_id,
            mode,
            from_device: me,
            from_user: Some(UserBrief { id: user_id, username: user.0.clone(), display_name: user.1.clone() }),
            ticket: signed.data.clone(),
            ticket_sig: signed.sig.clone(),
            relay_token: pair.tokens[1].to_vec(),
            peer_public_ip: my_ip.to_string(),
            same_public_ip: same,
        },
    );
    st.hub.send(
        &me,
        WsMsg::ConnectOk {
            req_id: req_id.to_string(),
            conn_id,
            target,
            ticket: signed.data,
            ticket_sig: signed.sig,
            relay_token: pair.tokens[0].to_vec(),
            peer_public_ip: target_ip.to_string(),
            same_public_ip: same,
        },
    );
    bump_stat(st, "connections", 1).await;
    let actor = audit::Actor { kind: "user", id: user_id.to_string(), name: user.0, ip: my_ip.to_string() };
    audit::log(st, &actor, "connection.start", Target::new("device", target.to_hex(), target_name), json!({ "mode": format!("{mode:?}").to_lowercase(), "conn_id": conn_id.to_hex() })).await;
    Ok(())
}

// ---------------- 中转转发（UDP 与 TCP 共用） ----------------

/// 把一端发来的负载转给另一端。返回是否转发成功。
pub async fn relay_forward(st: &St, pair: &Arc<RelayPair>, from_side: usize, payload: &[u8]) -> bool {
    let to = 1 - from_side;
    let now_ms = galaxy_proto::now_ms();
    pair.last_activity.store(now_ms, Ordering::Relaxed);
    let (kbps, relay_enabled) = {
        let s = st.settings.read();
        (s.relay_session_kbps, s.relay_enabled)
    };
    if !relay_enabled {
        return false;
    }
    if kbps > 0 {
        let sec = now_ms / 1000;
        let mut w = pair.window.lock();
        if w.0 != sec {
            *w = (sec, 0);
        }
        if w.1 + payload.len() as u64 > kbps * 1000 / 8 {
            return false;
        }
        w.1 += payload.len() as u64;
    }
    pair.bytes.fetch_add(payload.len() as u64, Ordering::Relaxed);
    st.relay.total_bytes.fetch_add(payload.len() as u64, Ordering::Relaxed);
    enum Dest {
        Udp(SocketAddr),
        Tcp(mpsc::Sender<bytes::Bytes>),
        None,
    }
    let dest = match &*pair.ends[to].lock() {
        RelayEnd::Udp(a) => Dest::Udp(*a),
        RelayEnd::Tcp(tx) => Dest::Tcp(tx.clone()),
        RelayEnd::None => Dest::None,
    };
    match dest {
        Dest::Udp(a) => {
            let pkt = galaxy_proto::wire::relay_deliver(&pair.tokens[to], payload);
            st.udp.send_to(&pkt, a).await.is_ok()
        }
        Dest::Tcp(tx) => tx.try_send(bytes::Bytes::copy_from_slice(payload)).is_ok(),
        Dest::None => false,
    }
}

#[derive(Deserialize)]
pub struct RelayQuery {
    token: String,
}

pub async fn relay_handler(State(st): State<St>, Query(q): Query<RelayQuery>, ws: WebSocketUpgrade) -> Response {
    ws.max_message_size(64 * 1024).on_upgrade(move |sock| async move {
        let Some(tok) = galaxy_proto::b64::decode(&q.token).ok().and_then(|v| <[u8; 16]>::try_from(v.as_slice()).ok()) else { return };
        let Some((pair, side)) = st.relay.lookup(&tok) else { return };
        let (tx, mut rx) = mpsc::channel::<bytes::Bytes>(1024);
        *pair.ends[side].lock() = RelayEnd::Tcp(tx);
        let (mut sink, mut stream) = sock.split();
        let writer = tokio::spawn(async move {
            while let Some(b) = rx.recv().await {
                if sink.send(Message::Binary(b)).await.is_err() {
                    break;
                }
            }
        });
        while let Ok(Some(Ok(m))) = tokio::time::timeout(Duration::from_secs(600), stream.next()).await {
            match m {
                Message::Binary(b) => {
                    relay_forward(&st, &pair, side, &b).await;
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        writer.abort();
        let mut e = pair.ends[side].lock();
        if matches!(*e, RelayEnd::Tcp(_)) {
            *e = RelayEnd::None;
        }
    })
}
