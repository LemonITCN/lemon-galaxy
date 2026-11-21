//! 信令长连接：断线自动重连，收到的消息广播给订阅者。

use crate::error::{ClientError, Result};
use crate::identity::Identity;
use futures::{SinkExt, StreamExt};
use galaxy_net::ServerAddr;
use galaxy_proto::cert::WS_AUTH_DOMAIN;
use galaxy_proto::ws::WsMsg;
use galaxy_proto::{ConnMode, DeviceId};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, watch};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::Connector;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignalState {
    Connecting,
    Connected { public_ip: String, assist_code: Option<String> },
    Disconnected { reason: String },
}

/// 未登录（临时协助）模式下附带的设备信息。
#[derive(Clone, Debug, Default)]
pub struct AssistParams {
    pub kind: String,
    pub platform: String,
    pub hostname: String,
    pub os_version: String,
    pub arch: String,
    pub app_version: String,
}

pub struct SignalClient {
    tx: mpsc::UnboundedSender<WsMsg>,
    events: broadcast::Sender<WsMsg>,
    state: watch::Receiver<SignalState>,
    stop: watch::Sender<bool>,
}

impl Drop for SignalClient {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

fn enc(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                vec![c.to_string()]
            } else {
                c.to_string().bytes().map(|b| format!("%{b:02X}")).collect()
            }
        })
        .collect()
}

impl SignalClient {
    pub fn start(server: ServerAddr, identity: Identity, token: Option<String>, assist: Option<AssistParams>) -> Arc<Self> {
        let (tx, rx) = mpsc::unbounded_channel();
        let (events, _) = broadcast::channel(1024);
        let (state_tx, state) = watch::channel(SignalState::Connecting);
        let (stop, stop_rx) = watch::channel(false);
        let ev = events.clone();
        tokio::spawn(run(server, identity, token, assist, rx, ev, state_tx, stop_rx));
        Arc::new(Self { tx, events, state, stop })
    }

    pub fn send(&self, m: WsMsg) -> bool {
        self.tx.send(m).is_ok()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WsMsg> {
        self.events.subscribe()
    }

    pub fn state(&self) -> watch::Receiver<SignalState> {
        self.state.clone()
    }

    pub fn is_connected(&self) -> bool {
        matches!(*self.state.borrow(), SignalState::Connected { .. })
    }

    pub fn stop(&self) {
        let _ = self.stop.send(true);
    }

    /// 等待连上（或超时）。
    pub async fn wait_connected(&self, timeout: Duration) -> bool {
        let mut st = self.state.clone();
        let fut = async {
            loop {
                if matches!(*st.borrow(), SignalState::Connected { .. }) {
                    return true;
                }
                if st.changed().await.is_err() {
                    return false;
                }
            }
        };
        tokio::time::timeout(timeout, fut).await.unwrap_or(false)
    }

    /// 发起连接请求，等待服务器回复 connect_ok / connect_error。
    pub async fn request_connect(&self, target: Option<DeviceId>, mode: ConnMode, assist_code: Option<String>) -> Result<WsMsg> {
        if !self.is_connected() && !self.wait_connected(Duration::from_secs(5)).await {
            return Err(ClientError::Network("连不上服务器".into()));
        }
        let req_id = uuid::Uuid::new_v4().simple().to_string();
        let mut sub = self.subscribe();
        self.send(WsMsg::Connect { req_id: req_id.clone(), target, mode, assist_code });
        let fut = async {
            loop {
                match sub.recv().await {
                    Ok(m @ WsMsg::ConnectOk { .. }) => {
                        if let WsMsg::ConnectOk { req_id: r, .. } = &m {
                            if *r == req_id {
                                return Ok(m);
                            }
                        }
                    }
                    Ok(WsMsg::ConnectError { req_id: r, code, message }) if r == req_id => {
                        let msg = match code.as_str() {
                            "offline" => "设备不在线".to_string(),
                            "forbidden" => "没有权限控制该设备".to_string(),
                            "not_found" => "设备码不存在或已失效".to_string(),
                            "rate_limited" => "操作太频繁，请稍后再试".to_string(),
                            _ => if message.is_empty() { code.clone() } else { message },
                        };
                        return Err(if code == "forbidden" { ClientError::Permission(msg) } else if code == "not_found" { ClientError::NotFound(msg) } else { ClientError::Network(msg) });
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => return Err(ClientError::Network("信令已断开".into())),
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(12), fut).await.map_err(|_| ClientError::Network("服务器响应超时".into()))?
    }
}

#[allow(clippy::too_many_arguments)]
async fn run(
    server: ServerAddr,
    identity: Identity,
    token: Option<String>,
    assist: Option<AssistParams>,
    mut rx: mpsc::UnboundedReceiver<WsMsg>,
    events: broadcast::Sender<WsMsg>,
    state: watch::Sender<SignalState>,
    mut stop: watch::Receiver<bool>,
) {
    let mut backoff = 1u64;
    loop {
        if *stop.borrow() {
            break;
        }
        let _ = state.send(SignalState::Connecting);
        let r = session(&server, &identity, token.as_deref(), assist.as_ref(), &mut rx, &events, &state, &mut stop).await;
        if *stop.borrow() {
            break;
        }
        let (reason, fatal) = match r {
            Ok(()) => ("连接已断开".to_string(), false),
            Err(e) => {
                let s = e.to_string();
                let fatal = s.contains("unknown_device") || s.contains("no_account") || s.contains("bad_signature");
                (s, fatal)
            }
        };
        tracing::debug!("信令断开: {reason}");
        let _ = state.send(SignalState::Disconnected { reason });
        let wait = if fatal { 60 } else { backoff };
        backoff = (backoff * 2).min(30);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(wait)) => {}
            _ = stop.changed() => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn session(
    server: &ServerAddr,
    identity: &Identity,
    token: Option<&str>,
    assist: Option<&AssistParams>,
    rx: &mut mpsc::UnboundedReceiver<WsMsg>,
    events: &broadcast::Sender<WsMsg>,
    state: &watch::Sender<SignalState>,
    stop: &mut watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let mut url = server.ws_url("/api/v1/ws");
    if let Some(a) = assist {
        url.push_str(&format!(
            "?assist=1&kind={}&platform={}&hostname={}&os_version={}&arch={}&app_version={}",
            enc(&a.kind), enc(&a.platform), enc(&a.hostname), enc(&a.os_version), enc(&a.arch), enc(&a.app_version)
        ));
    } else if token.is_some() {
        // 主控端连接：与同一设备上的被控服务各占一条，互不挤掉
        url.push_str("?role=client");
    }
    let mut req = url.into_client_request()?;
    if let Some(t) = token {
        req.headers_mut().insert("authorization", format!("Bearer {t}").parse()?);
    }
    let cfg = galaxy_net::tls::server_client_config(server.fingerprint, &[b"http/1.1"]);
    let connector = if server.fingerprint.is_some() { Some(Connector::Rustls(Arc::new(cfg))) } else { None };
    let (ws, _) = tokio::time::timeout(Duration::from_secs(15), tokio_tungstenite::connect_async_tls_with_config(req, None, true, connector)).await??;
    let (mut sink, mut stream) = ws.split();

    // 认证
    let hello = loop {
        match tokio::time::timeout(Duration::from_secs(15), stream.next()).await? {
            Some(Ok(Message::Text(t))) => break WsMsg::from_json(&t),
            Some(Ok(_)) => continue,
            _ => anyhow::bail!("服务器关闭了连接"),
        }
    };
    let Some(WsMsg::Hello { nonce, .. }) = hello else { anyhow::bail!("握手消息错误") };
    let sig = identity.key.sign(WS_AUTH_DOMAIN, &nonce);
    let auth = WsMsg::Auth { device_id: identity.device_id, signature: sig, device_pubkey: Some(identity.key.public().to_vec()) };
    sink.send(Message::Text(auth.to_json().into())).await?;

    let mut ping = tokio::time::interval(Duration::from_secs(25));
    ping.tick().await;
    loop {
        tokio::select! {
            m = stream.next() => {
                let Some(m) = m else { return Ok(()) };
                match m? {
                    Message::Text(t) => {
                        let Some(msg) = WsMsg::from_json(&t) else { continue };
                        match &msg {
                            WsMsg::Ready { public_ip, assist_code, .. } => {
                                let _ = state.send(SignalState::Connected { public_ip: public_ip.clone(), assist_code: assist_code.clone() });
                            }
                            WsMsg::Error { code, .. } => {
                                let _ = events.send(msg.clone());
                                anyhow::bail!("{code}");
                            }
                            _ => {}
                        }
                        let _ = events.send(msg);
                    }
                    Message::Close(_) => return Ok(()),
                    _ => {}
                }
            }
            out = rx.recv() => {
                let Some(out) = out else { return Ok(()) };
                sink.send(Message::Text(out.to_json().into())).await?;
            }
            _ = ping.tick() => {
                sink.send(Message::Ping(Vec::new().into())).await?;
            }
            _ = stop.changed() => {
                let _ = sink.close().await;
                return Ok(());
            }
        }
    }
}
