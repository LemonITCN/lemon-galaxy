//! 被控端服务：保持信令在线、响应连接请求（候选交换、中转、探测）、接受 QUIC、端到端认证、分派会话。

use crate::accounts::{AccountStatus, HostAccount, HostPolicy, HostState};
use crate::platform::{HostPlatform, NoPlatform};
use crate::term::TermManager;
use futures::future::BoxFuture;
use galaxy_client::account::{DeviceMeta, LoggedIn};
use galaxy_client::identity::Identity;
use galaxy_client::signal::{AssistParams, SignalClient, SignalState};
use galaxy_net::handshake::{self, AssistSecret, HostVerdict};
use galaxy_net::magicsock::MagicSock;
use galaxy_net::ServerAddr;
use galaxy_proto::cert::{DeviceCert, SignedBlob};
use galaxy_proto::peer::{Hello, HelloAck};
use galaxy_proto::ws::{Candidate, CandidateKind, WsMsg};
use galaxy_proto::{ConnId, ConnMode, DeviceId, HostInfo, PathKind, Permissions, UserBrief};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::watch;

pub struct HostConfig {
    pub server: ServerAddr,
    pub data_dir: PathBuf,
    pub meta: DeviceMeta,
    /// worker 可登录多个账号；control 只能一个
    pub multi_account: bool,
    /// root 运行时替普通用户做文件操作的降权子进程（通常是当前可执行文件）
    pub file_helper: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub account: Option<u64>,
    pub from_user: String,
    pub from_device: String,
    pub mode: ConnMode,
    pub path: PathKind,
    pub started_at: i64,
    pub ai_active: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostStatus {
    pub online: bool,
    pub server: String,
    pub device_id: String,
    pub accounts: Vec<(u64, String, AccountStatus)>,
    pub sessions: Vec<SessionInfo>,
    pub assist: Option<(String, String)>,
    pub remote_enabled: bool,
    pub public_ip: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostLogEvent {
    pub at: i64,
    pub session: String,
    pub user: String,
    pub kind: String,
    pub detail: String,
    pub ai: bool,
}

pub trait HostEvents: Send + Sync {
    fn on_status(&self, _s: &HostStatus) {}
    /// 临时协助请求：返回是否允许。默认拒绝。
    fn ask_assist(&self, _from_user: String, _from_device: String) -> BoxFuture<'static, bool> {
        Box::pin(async { false })
    }
    fn on_session_started(&self, _s: &SessionInfo) {}
    fn on_session_ended(&self, _id: &str) {}
    fn on_ai_activity(&self, _session: &str, _desc: &str) {}
    fn on_event(&self, _e: &HostLogEvent) {}
}

pub struct NoEvents;
impl HostEvents for NoEvents {}

struct Pending {
    conn_id: ConnId,
    mode: ConnMode,
    from_user: Option<UserBrief>,
    /// 离线局域网连接没有票据
    ticket: Option<SignedBlob>,
    at: Instant,
    stop: watch::Sender<bool>,
}

pub struct ActiveSession {
    pub info: RwLock<SessionInfo>,
    pub account: Option<u64>,
    pub policy: HostPolicy,
    pub conn: quinn::Connection,
    pub user: crate::sysuser::SysUser,
    pub from_user_name: String,
    /// 看过远程桌面（结束时决定要不要自动锁屏）
    pub used_display: std::sync::atomic::AtomicBool,
}

pub struct HostService {
    pub cfg: HostConfig,
    pub identity: Identity,
    pub ms: Arc<MagicSock>,
    pub ep: quinn::Endpoint,
    state: RwLock<HostState>,
    signal: RwLock<Option<Arc<SignalClient>>>,
    lan_keys: Arc<RwLock<Vec<[u8; 32]>>>,
    pending: Mutex<HashMap<DeviceId, Pending>>,
    pub(crate) sessions: Mutex<HashMap<String, Arc<ActiveSession>>>,
    pub terms: Arc<TermManager>,
    pub platform: RwLock<Arc<dyn HostPlatform>>,
    /// 远端声音分发
    pub audio: Arc<crate::audio::AudioHub>,
    /// 画面：同一块屏只采集一路，分给所有在看的会话
    pub captures: Arc<crate::capture::CaptureHub>,
    /// 开着隐私屏的会话
    pub privacy_owner: parking_lot::Mutex<Option<String>>,
    /// 虚拟屏归属（见 vdisplay.rs）
    pub(crate) vdisplays: crate::vdisplay::VirtualDisplays,
    /// 已停止（换服务器等）：不再重连信令
    stopped: std::sync::atomic::AtomicBool,
    /// 通知局域网发现监听退出
    lan_stop: watch::Sender<bool>,
    pub events: RwLock<Arc<dyn HostEvents>>,
    assist_active: Mutex<Option<String>>,
    public_ip: RwLock<String>,
    ai_stop: watch::Sender<u64>,
}

fn state_path(dir: &std::path::Path) -> PathBuf {
    dir.join("host.json")
}

impl HostService {
    pub async fn start(cfg: HostConfig, identity: Identity) -> anyhow::Result<Arc<Self>> {
        galaxy_net::init_crypto();
        std::fs::create_dir_all(&cfg.data_dir)?;
        let mut state = HostState::load(&state_path(&cfg.data_dir));
        let full = cfg.server.to_string_full();
        if !state.server.is_empty() && state.server != full {
            // 换了服务器：旧账号全部作废
            state.accounts.clear();
            state.server_sign_pubkey.clear();
        }
        state.server = full;
        let ms = MagicSock::bind(0)?;
        ms.set_server(cfg.server.resolve());
        let ep = galaxy_net::quic::endpoint(ms.clone(), true)?;
        let lan_keys = Arc::new(RwLock::new(state.active().map(|a| galaxy_crypto::lan_key(&a.account_pubkey)).collect()));
        let (ai_stop, _) = watch::channel(0u64);
        let (lan_stop, lan_stop_rx) = watch::channel(false);
        let me = Arc::new(Self {
            cfg,
            identity,
            ms,
            ep,
            state: RwLock::new(state),
            signal: RwLock::new(None),
            lan_keys,
            pending: Default::default(),
            sessions: Default::default(),
            terms: Arc::new(TermManager::default()),
            platform: RwLock::new(Arc::new(NoPlatform)),
            audio: crate::audio::AudioHub::new(),
            captures: crate::capture::CaptureHub::new(),
            privacy_owner: parking_lot::Mutex::new(None),
            vdisplays: Default::default(),
            stopped: std::sync::atomic::AtomicBool::new(false),
            lan_stop,
            events: RwLock::new(Arc::new(NoEvents)),
            assist_active: Mutex::new(None),
            public_ip: RwLock::new(String::new()),
            ai_stop,
        });
        // 局域网发现与离线连接请求
        let weak = Arc::downgrade(&me);
        let on_connect: galaxy_net::lan::OnLanConnect = Arc::new(move |ctl, conn_id, addr| {
            if let Some(h) = weak.upgrade() {
                h.offline_incoming(ctl, conn_id, addr);
            }
        });
        galaxy_net::lan::spawn_responder_until(me.ms.clone(), me.identity.device_id, me.lan_keys.clone(), Some(on_connect), Some(lan_stop_rx));
        me.save();
        me.refresh_server_key().await;
        me.restart_signal();
        let me2 = me.clone();
        tokio::spawn(async move { me2.accept_loop().await });
        let me3 = me.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(20)).await;
                if me3.stopped.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                me3.pending.lock().retain(|_, p| {
                    let keep = p.at.elapsed() < Duration::from_secs(60);
                    if !keep {
                        let _ = p.stop.send(true);
                    }
                    keep
                });
            }
        });
        Ok(me)
    }

    pub fn set_platform(&self, p: Arc<dyn HostPlatform>) {
        *self.platform.write() = p;
    }

    pub fn set_events(&self, e: Arc<dyn HostEvents>) {
        *self.events.write() = e;
        self.emit_status();
    }

    fn save(&self) {
        let st = self.state.read().clone();
        if let Err(e) = st.save(&state_path(&self.cfg.data_dir)) {
            tracing::warn!("保存被控端状态失败: {e}");
        }
    }

    async fn refresh_server_key(&self) {
        if !self.state.read().server_sign_pubkey.is_empty() {
            return;
        }
        if let Ok(api) = galaxy_client::api::Api::new(self.cfg.server.clone(), "zh-CN") {
            if let Ok(info) = api.get::<galaxy_proto::api::ServerInfoResp>("/info").await {
                self.state.write().server_sign_pubkey = info.server_sign_pubkey;
                self.save();
            }
        }
    }

    pub fn state(&self) -> HostState {
        self.state.read().clone()
    }

    pub fn status(&self) -> HostStatus {
        let st = self.state.read();
        let online = self.signal.read().as_ref().map(|s| s.is_connected()).unwrap_or(false);
        let assist = self.assist_active.lock().clone().and_then(|code| {
            let dc = self.signal.read().as_ref().and_then(|s| match &*s.state().borrow() {
                SignalState::Connected { assist_code: Some(c), .. } => Some(c.clone()),
                _ => None,
            });
            dc.map(|d| (d, code))
        });
        HostStatus {
            online,
            server: st.server.clone(),
            device_id: self.identity.device_id.to_hex(),
            accounts: st.accounts.iter().map(|a| (a.user_id, a.username.clone(), a.status)).collect(),
            sessions: self.sessions.lock().values().map(|s| s.info.read().clone()).collect(),
            assist,
            remote_enabled: st.remote_enabled,
            public_ip: self.public_ip.read().clone(),
        }
    }

    pub(crate) fn emit_status(&self) {
        let s = self.status();
        self.events.read().on_status(&s);
    }

    pub fn log(&self, session: &str, user: &str, kind: &str, detail: &str, ai: bool) {
        let e = HostLogEvent { at: galaxy_proto::now_ms(), session: session.into(), user: user.into(), kind: kind.into(), detail: detail.chars().take(2000).collect(), ai };
        if let Ok(line) = serde_json::to_string(&e) {
            use std::io::Write;
            let p = self.cfg.data_dir.join("events.log");
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
                let _ = writeln!(f, "{line}");
            }
            if std::fs::metadata(&p).map(|m| m.len() > 20 * 1024 * 1024).unwrap_or(false) {
                let _ = std::fs::rename(&p, self.cfg.data_dir.join("events.log.1"));
            }
        }
        self.events.read().on_event(&e);
        if ai {
            self.events.read().on_ai_activity(session, detail);
        }
    }

    pub fn recent_events(&self, n: usize) -> Vec<HostLogEvent> {
        let p = self.cfg.data_dir.join("events.log");
        let Ok(s) = std::fs::read_to_string(p) else { return vec![] };
        let mut v: Vec<HostLogEvent> = s.lines().rev().take(n).filter_map(|l| serde_json::from_str(l).ok()).collect();
        v.reverse();
        v
    }

    // ---------------- 账号 ----------------

    /// 登录完成后加入账号（被控端随即丢弃账号私钥，只留公钥与证书）。
    pub fn add_account(self: &Arc<Self>, l: &LoggedIn, policy: HostPolicy) -> anyhow::Result<()> {
        {
            let mut st = self.state.write();
            if !self.cfg.multi_account {
                st.accounts.retain(|a| a.user_id == l.user.id);
            }
            st.accounts.retain(|a| a.user_id != l.user.id);
            st.accounts.push(HostAccount {
                user_id: l.user.id,
                username: l.user.username.clone(),
                display_name: l.user.display_name.clone(),
                account_pubkey: l.account_pubkey,
                epoch: l.epoch,
                cert: l.cert.clone(),
                token: l.token.clone(),
                status: AccountStatus::Active,
                policy,
                revoked: vec![],
                added_at: galaxy_proto::now_ms(),
            });
        }
        self.refresh_lan_keys();
        self.save();
        if !self.signal.read().as_ref().map(|s| s.is_connected()).unwrap_or(false) {
            self.restart_signal();
        }
        self.emit_status();
        Ok(())
    }

    pub async fn remove_account(self: &Arc<Self>, user_id: u64) {
        let acct = self.state.read().accounts.iter().find(|a| a.user_id == user_id).cloned();
        if let Some(a) = acct {
            if let Ok(api) = galaxy_client::api::Api::new(self.cfg.server.clone(), "zh-CN") {
                api.set_token(Some(a.token.clone()));
                let _ = api.post::<_, serde_json::Value>("/auth/logout", &serde_json::json!({})).await;
            }
        }
        self.state.write().accounts.retain(|a| a.user_id != user_id);
        for s in self.sessions.lock().values() {
            if s.account == Some(user_id) {
                s.conn.close(2u32.into(), b"account removed");
            }
        }
        self.refresh_lan_keys();
        self.save();
        if self.state.read().active().count() == 0 && self.assist_active.lock().is_none() {
            if let Some(s) = self.signal.write().take() {
                s.stop();
            }
        }
        self.emit_status();
    }

    pub fn update_policy(&self, user_id: u64, policy: HostPolicy) {
        if let Some(a) = self.state.write().accounts.iter_mut().find(|a| a.user_id == user_id) {
            a.policy = policy;
        }
        self.save();
        self.emit_status();
    }

    fn refresh_lan_keys(&self) {
        *self.lan_keys.write() = self.state.read().active().map(|a| galaxy_crypto::lan_key(&a.account_pubkey)).collect();
    }

    /// 重新读取 host.json（命令行登录 / 改权限后由守护进程调用）。返回是否有变化。
    pub fn reload_from_disk(self: &Arc<Self>) -> bool {
        let disk = HostState::load(&state_path(&self.cfg.data_dir));
        let changed = {
            let cur = self.state.read();
            serde_json::to_string(&cur.accounts).ok() != serde_json::to_string(&disk.accounts).ok()
                || cur.remote_enabled != disk.remote_enabled
                || cur.assist_code != disk.assist_code
                || cur.lock_on_end != disk.lock_on_end
                || cur.headless_display != disk.headless_display
                || cur.auto_update != disk.auto_update
                || cur.unlock_on_connect != disk.unlock_on_connect
        };
        if !changed {
            return false;
        }
        let had_active = self.state.read().active().count() > 0;
        {
            let mut st = self.state.write();
            st.accounts = disk.accounts;
            st.remote_enabled = disk.remote_enabled;
            st.assist_code = disk.assist_code;
            st.lock_on_end = disk.lock_on_end;
            st.headless_display = disk.headless_display;
            // 不读进来的话，服务下次保存会把命令行 / 界面改的自动更新开关写回旧值
            st.auto_update = disk.auto_update;
            st.unlock_on_connect = disk.unlock_on_connect;
        }
        self.refresh_lan_keys();
        let has_active = self.state.read().active().count() > 0;
        if !self.state.read().remote_enabled {
            self.disconnect_all();
        }
        if has_active != had_active || !self.online() {
            self.restart_signal();
        }
        self.emit_status();
        true
    }

    /// 把状态写到 status.json，供命令行 status 读取。
    pub fn write_status_file(&self) {
        let s = self.status();
        if let Ok(b) = serde_json::to_vec_pretty(&s) {
            let p = self.cfg.data_dir.join("status.json");
            let tmp = p.with_extension("tmp");
            if std::fs::write(&tmp, b).is_ok() {
                let _ = std::fs::rename(tmp, p);
            }
        }
    }

    /// 远程看过桌面的会话都结束后自动锁屏。
    pub fn set_lock_on_end(&self, on: bool) {
        self.state.write().lock_on_end = on;
        self.save();
    }

    pub fn lock_on_end(&self) -> bool {
        self.state.read().lock_on_end
    }

    /// 无头兜底屏的模式 (宽, 高, 刷新率)；关闭时为 None。
    pub fn headless_display(&self) -> Option<(u32, u32, u32)> {
        crate::accounts::parse_headless_display(&self.state.read().headless_display)
    }

    /// 修改无头兜底屏设置（空 = 默认 1920x1080@60，`off` = 关闭）。
    pub fn set_headless_display(&self, v: &str) {
        self.state.write().headless_display = v.trim().to_string();
        self.save();
    }

    pub(crate) fn session_count(&self) -> usize {
        self.sessions.lock().len()
    }

    /// 会话结束后：开着自动锁屏、这个会话看过桌面、10 秒内没有别的会话在看桌面（断线重连不算结束），就锁屏。
    fn maybe_lock_after(self: &Arc<Self>, ended: &ActiveSession) {
        if !ended.used_display.load(std::sync::atomic::Ordering::Relaxed) || !self.state.read().lock_on_end {
            return;
        }
        let me = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            let watching = me.sessions.lock().values().any(|s| s.used_display.load(std::sync::atomic::Ordering::Relaxed));
            if watching || !me.state.read().lock_on_end {
                return;
            }
            let platform = me.platform.read().clone();
            let _ = tokio::task::spawn_blocking(move || {
                let display = platform.list_displays().first().map(|d| d.id).unwrap_or(0);
                platform.inject(display, galaxy_proto::peer::InputEvent::SpecialCombo(galaxy_proto::peer::SpecialCombo::LockScreen));
            })
            .await;
            tracing::info!("远程会话结束，已自动锁屏");
        });
    }

    pub fn set_remote_enabled(&self, on: bool) {
        self.state.write().remote_enabled = on;
        self.save();
        if !on {
            self.disconnect_all();
        }
        self.emit_status();
    }

    /// 断开一个会话。返回是否找到。
    pub fn disconnect_session(&self, id: &str) -> bool {
        let s = self.sessions.lock().get(id).cloned();
        match s {
            Some(s) => {
                s.conn.close(3u32.into(), b"disconnected by host");
                true
            }
            None => false,
        }
    }

    pub fn disconnect_all(&self) {
        let all: Vec<_> = self.sessions.lock().values().cloned().collect();
        for s in all {
            s.conn.close(3u32.into(), b"disconnected by host");
        }
    }

    /// 一键停止所有 AI 操作：断开所有有 AI 活动的会话。
    pub fn stop_ai(&self) {
        self.ai_stop.send_modify(|v| *v += 1);
        let all: Vec<_> = self.sessions.lock().values().filter(|s| s.info.read().ai_active).cloned().collect();
        for s in all {
            s.conn.close(4u32.into(), b"ai stopped by host");
        }
    }

    pub fn ai_stop_watch(&self) -> watch::Receiver<u64> {
        self.ai_stop.subscribe()
    }

    // ---------------- 临时协助 ----------------

    /// 开启临时协助，返回验证码；设备码在信令连上后出现在 status().assist 里。
    pub fn start_assist(self: &Arc<Self>) -> String {
        let code = self.state.read().assist_code.clone().filter(|c| c.len() >= 6).unwrap_or_else(random_code);
        *self.assist_active.lock() = Some(code.clone());
        self.restart_signal();
        code
    }

    pub fn stop_assist(self: &Arc<Self>) {
        *self.assist_active.lock() = None;
        self.restart_signal();
        self.emit_status();
    }

    pub fn set_assist_code(&self, code: Option<String>) {
        let c = code.map(|s| s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>()).filter(|s| s.len() >= 8 && s.len() <= 16);
        self.state.write().assist_code = c.clone();
        self.save();
        if self.assist_active.lock().is_some() {
            *self.assist_active.lock() = Some(c.unwrap_or_else(random_code));
        }
        self.emit_status();
    }

    // ---------------- 信令 ----------------

    /// 停止服务：断开信令和所有会话、关闭端点（换服务器前用）。停止后不会再自动重连。
    pub fn shutdown(&self) {
        self.stopped.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = self.lan_stop.send(true);
        self.lan_keys.write().clear();
        if let Some(s) = self.signal.write().take() {
            s.stop();
        }
        self.disconnect_all();
        self.terms.kill_all();
        self.ep.close(0u32.into(), b"shutdown");
        self.ms.close();
    }

    fn restart_signal(self: &Arc<Self>) {
        if self.stopped.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let assist = self.assist_active.lock().is_some();
        let has_account = self.state.read().active().count() > 0;
        if let Some(old) = self.signal.write().take() {
            old.stop();
        }
        if !assist && !has_account {
            return;
        }
        let params = if assist {
            Some(AssistParams {
                kind: self.cfg.meta.kind.as_str().into(),
                platform: self.cfg.meta.platform.as_str().into(),
                hostname: self.cfg.meta.hostname.clone(),
                os_version: self.cfg.meta.os_version.clone(),
                arch: self.cfg.meta.arch.clone(),
                app_version: self.cfg.meta.app_version.clone(),
            })
        } else {
            None
        };
        let sig = SignalClient::start(self.cfg.server.clone(), self.identity.clone(), None, params);
        *self.signal.write() = Some(sig.clone());
        // 事件循环（持有弱引用，服务释放后自动结束）
        let weak = Arc::downgrade(self);
        let mut sub = sig.subscribe();
        let mut state = sig.state();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    m = sub.recv() => match m {
                        Ok(m) => {
                            let Some(me) = weak.upgrade() else { break };
                            me.on_signal(m).await;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(_) => break,
                    },
                    r = state.changed() => {
                        if r.is_err() { break; }
                        let Some(me) = weak.upgrade() else { break };
                        if let SignalState::Connected { public_ip, .. } = &*state.borrow() {
                            *me.public_ip.write() = public_ip.clone();
                        }
                        me.emit_status();
                    }
                }
            }
        });
    }

    async fn on_signal(self: Arc<Self>, m: WsMsg) {
        match m {
            WsMsg::Incoming { conn_id, mode, from_device, from_user, ticket, ticket_sig, relay_token, .. } => {
                if !self.state.read().remote_enabled {
                    self.send(WsMsg::ConnClose { conn_id });
                    return;
                }
                if mode == ConnMode::Assist && self.assist_active.lock().is_none() {
                    self.send(WsMsg::ConnClose { conn_id });
                    return;
                }
                let Ok(token) = <[u8; 16]>::try_from(relay_token.as_slice()) else { return };
                let peer = self.ms.add_peer(from_device, galaxy_crypto::conn_tag(&conn_id.0), Some(token));
                let (stop, stop_rx) = watch::channel(false);
                if let Some(old) = self.pending.lock().insert(from_device, Pending { conn_id, mode, from_user, ticket: Some(SignedBlob { data: ticket, sig: ticket_sig }), at: Instant::now(), stop }) {
                    let _ = old.stop.send(true);
                }
                // 候选
                let mut mine = self.ms.host_candidates();
                self.send(WsMsg::Candidates { conn_id, items: mine.clone() });
                let me = self.clone();
                let peer2 = peer.clone();
                tokio::spawn(async move {
                    for a in me.ms.discover_reflexive(Duration::from_millis(800)).await {
                        mine.push(Candidate { addr: a.to_string(), kind: CandidateKind::Srflx });
                    }
                    me.send(WsMsg::Candidates { conn_id, items: mine });
                    if !me.ms.bind_relay(&peer2, Duration::from_millis(2500)).await {
                        if let Ok((tx, mut rx)) = galaxy_net::relay_tcp::connect(&me.cfg.server, token).await {
                            me.ms.set_relay_tcp(&peer2, Some(tx));
                            while let Some(b) = rx.recv().await {
                                me.ms.inject_relay_tcp(&peer2, b);
                            }
                            me.ms.set_relay_tcp(&peer2, None);
                        }
                    }
                });
                self.ms.spawn_prober(peer, stop_rx);
            }
            WsMsg::Candidates { conn_id, items } => {
                let dev = self.pending.lock().iter().find(|(_, p)| p.conn_id == conn_id).map(|(d, _)| *d);
                let dev = dev.or_else(|| {
                    // 已建立的会话路径变化
                    None
                });
                if let Some(d) = dev {
                    if let Some(peer) = self.ms.peer(&d) {
                        self.ms.add_candidates(&peer, &items);
                    }
                }
            }
            WsMsg::ConnClose { conn_id } => {
                let dev = self.pending.lock().iter().find(|(_, p)| p.conn_id == conn_id).map(|(d, _)| *d);
                if let Some(d) = dev {
                    if let Some(p) = self.pending.lock().remove(&d) {
                        let _ = p.stop.send(true);
                    }
                }
            }
            WsMsg::AccountReset { user_id } => {
                if let Some(a) = self.state.write().accounts.iter_mut().find(|a| a.user_id == user_id) {
                    a.status = AccountStatus::NeedsRelogin;
                }
                for s in self.sessions.lock().values() {
                    if s.account == Some(user_id) {
                        s.conn.close(2u32.into(), b"account reset");
                    }
                }
                self.refresh_lan_keys();
                self.save();
                self.emit_status();
            }
            WsMsg::SessionRevoked => {
                // 某个账号的会话被注销：逐个检查令牌是否还有效
                let accts = self.state.read().accounts.clone();
                for a in accts {
                    if let Ok(api) = galaxy_client::api::Api::new(self.cfg.server.clone(), "zh-CN") {
                        api.set_token(Some(a.token.clone()));
                        if let Err(galaxy_client::ClientError::Auth { .. }) = api.get::<serde_json::Value>("/account").await {
                            if let Some(x) = self.state.write().accounts.iter_mut().find(|x| x.user_id == a.user_id) {
                                x.status = AccountStatus::NeedsRelogin;
                            }
                        }
                    }
                }
                self.refresh_lan_keys();
                self.save();
                self.emit_status();
            }
            _ => {}
        }
    }

    /// 服务器不可达时，主控端经局域网直接发来的连接请求。被控端在线时不接受（必须走服务器授权）。
    fn offline_incoming(self: &Arc<Self>, controller: DeviceId, conn_id: [u8; 16], addr: std::net::SocketAddr) {
        if self.online() || !self.state.read().remote_enabled {
            return;
        }
        let cid = ConnId(conn_id);
        if self.pending.lock().get(&controller).map(|p| p.conn_id == cid).unwrap_or(false) {
            return;
        }
        let peer = self.ms.add_peer(controller, galaxy_crypto::conn_tag(&conn_id), None);
        self.ms.add_candidates(&peer, &[Candidate { addr: addr.to_string(), kind: CandidateKind::Lan }]);
        let (stop, stop_rx) = watch::channel(false);
        if let Some(old) = self.pending.lock().insert(controller, Pending { conn_id: cid, mode: ConnMode::Control, from_user: None, ticket: None, at: Instant::now(), stop }) {
            let _ = old.stop.send(true);
        }
        tracing::info!("局域网离线连接请求：{}", controller.short());
        self.ms.spawn_prober(peer, stop_rx);
    }

    pub fn send(&self, m: WsMsg) {
        if let Some(s) = self.signal.read().as_ref() {
            s.send(m);
        }
    }

    fn online(&self) -> bool {
        self.signal.read().as_ref().map(|s| s.is_connected()).unwrap_or(false)
    }

    // ---------------- 接受连接 ----------------

    async fn accept_loop(self: Arc<Self>) {
        while let Some(inc) = self.ep.accept().await {
            let me = self.clone();
            tokio::spawn(async move {
                let conn = match inc.await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::debug!("QUIC 接入失败: {e}");
                        return;
                    }
                };
                if let Err(e) = me.clone().handle_conn(conn).await {
                    tracing::info!("会话结束: {e}");
                }
            });
        }
    }

    /// 临时协助的权限。普通权限运行时按当前用户；以 SYSTEM / root 运行（系统服务）时按屏幕前登录的用户，
    /// 取不到登录用户就只给画面、键鼠、剪贴板与声音，不给终端与文件——否则对方会拿到最高权限。
    fn assist_policy(&self) -> HostPolicy {
        use crate::sysuser::{is_root, SysUser};
        if !is_root() {
            return HostPolicy::default_for(&SysUser::current().name);
        }
        // 最高权限账号（root、SYSTEM）不算登录用户
        let privileged = |s: &SysUser| if cfg!(windows) { s.sid == "S-1-5-18" || s.name.eq_ignore_ascii_case("SYSTEM") } else { s.uid == 0 };
        let user = self.platform.read().interactive_user().filter(|u| SysUser::lookup(u).is_some_and(|s| !privileged(&s)));
        match user {
            Some(u) => HostPolicy::default_for(&u),
            None => {
                let mut p = HostPolicy::default_for("");
                p.terminal = false;
                p.files = false;
                p
            }
        }
    }

    fn host_info(&self, policy: Option<&HostPolicy>) -> (HostInfo, crate::sysuser::SysUser) {
        let user = policy
            .and_then(|p| crate::sysuser::SysUser::lookup(&p.system_user))
            .unwrap_or_else(crate::sysuser::SysUser::current);
        (
            HostInfo {
                device_id: self.identity.device_id,
                hostname: self.cfg.meta.hostname.clone(),
                platform: self.cfg.meta.platform,
                os_version: self.cfg.meta.os_version.clone(),
                arch: self.cfg.meta.arch.clone(),
                app_version: self.cfg.meta.app_version.clone(),
                system_user: user.name.clone(),
                shell: user.shell.clone(),
            },
            user,
        )
    }

    fn capabilities(&self, perms: &Permissions) -> Vec<String> {
        let mut c = vec![];
        if perms.terminal {
            c.extend(["terminal", "exec", "port_forward"].map(String::from));
        }
        if perms.files {
            c.push("files".into());
        }
        let pc = self.platform.read().capabilities();
        if perms.view {
            for x in &pc {
                if matches!(x.as_str(), "display" | "screenshot" | "ui_tree") {
                    c.push(x.clone());
                }
            }
        }
        if perms.audio && pc.iter().any(|x| x == "audio") {
            c.push("audio".into());
        }
        if perms.control && pc.iter().any(|x| x == "input") {
            c.push("input".into());
        }
        if perms.view && pc.iter().any(|x| x == "client_cursor") {
            c.push("client_cursor".into());
        }
        // 隐私屏要有控制权限（开着时被控端本地键鼠失效）
        if perms.control && pc.iter().any(|x| x == "privacy") {
            c.push("privacy".into());
        }
        // 虚拟屏：要有控制权限，平台层有驱动才有
        if perms.control {
            let max = self.platform.read().virtual_display_max();
            if max > 0 {
                c.push(galaxy_proto::peer::CAP_VIRTUAL_DISPLAY.into());
                c.push(format!("{}={max}", galaxy_proto::peer::CAP_VIRTUAL_DISPLAY_MAX));
            }
        }
        if perms.clipboard && pc.iter().any(|x| x == "clipboard") {
            c.push("clipboard".into());
        }
        c
    }

    /// 正常模式：校验主控端证书与票据，返回所属账号。
    fn verify_controller(&self, hello: &Hello, pending: Option<&Pending>) -> Result<HostAccount, String> {
        let blob = hello.cert.as_ref().ok_or("缺少设备证书")?;
        let unverified = DeviceCert::from_bytes(&blob.data).ok_or("证书格式错误")?;
        let st = self.state.read();
        let acct = st
            .accounts
            .iter()
            .find(|a| a.user_id == unverified.account_id && a.status == AccountStatus::Active)
            .cloned()
            .ok_or("本机未登录该账号")?;
        let cert = galaxy_crypto::verify_device_cert(&acct.account_pubkey, blob).map_err(|_| "证书签名无效")?;
        if cert.account_epoch != acct.epoch {
            return Err("账号密钥已更换，请重新登录".into());
        }
        if cert.device_pubkey != hello.device_pubkey {
            return Err("证书与设备不符".into());
        }
        if !cert.kind.can_control() {
            return Err("该设备不能发起控制".into());
        }
        if acct.revoked.contains(&cert.device_id) {
            return Err("该设备已被吊销".into());
        }
        let ticket = hello.ticket.as_ref().or(pending.and_then(|p| p.ticket.as_ref()));
        let online = self.online();
        match ticket {
            Some(t) if !st.server_sign_pubkey.is_empty() => {
                let tk = galaxy_crypto::verify_ticket(&st.server_sign_pubkey, t).map_err(|_| "连接票据无效")?;
                let now = galaxy_proto::now_ms();
                if tk.controller_device != cert.device_id || tk.target_device != self.identity.device_id || tk.account_id != cert.account_id || tk.expires_at + 300_000 < now {
                    return Err("连接票据不匹配或已过期".into());
                }
            }
            _ if online => return Err("缺少服务器授权".into()),
            _ => {} // 服务器不可达：局域网离线模式，只凭证书
        }
        Ok(acct)
    }

    async fn handle_conn(self: Arc<Self>, conn: quinn::Connection) -> anyhow::Result<()> {
        let virt = conn.remote_address();
        let device = self.ms.peer_by_virt(&virt).map(|p| p.device);
        let pending = device.and_then(|d| self.pending.lock().remove(&d));
        let hs = handshake::host_accept(&conn).await?;
        let from_user = pending.as_ref().and_then(|p| p.from_user.clone());
        let assist_mode = hs.hello.assist;

        let (verdict, account, policy, session_from) = if assist_mode {
            let code = self.assist_active.lock().clone();
            let dc = self.status().assist.map(|a| a.0);
            match (code, dc, pending.as_ref().map(|p| p.mode)) {
                (Some(code), Some(dc), Some(ConnMode::Assist)) => {
                    let policy = self.assist_policy();
                    (HostVerdict::Assist { secret: AssistSecret { code, device_code: dc } }, None, policy, from_user.clone())
                }
                _ => (HostVerdict::Reject { code: "assist_off".into(), message: "对方未开启临时协助".into() }, None, HostPolicy::default_for("nobody"), None),
            }
        } else if !self.state.read().remote_enabled {
            (HostVerdict::Reject { code: "disabled".into(), message: "对方已禁止远程".into() }, None, HostPolicy::default_for("nobody"), None)
        } else {
            match self.verify_controller(&hs.hello, pending.as_ref()) {
                Ok(acct) => {
                    let (host, _) = self.host_info(Some(&acct.policy));
                    let perms = acct.policy.permissions();
                    let ack = HelloAck { host, account: Some(acct.user_id), permissions: perms.clone(), capabilities: self.capabilities(&perms) };
                    let name = from_user.clone().unwrap_or(UserBrief { id: acct.user_id, username: acct.username.clone(), display_name: acct.display_name.clone() });
                    (HostVerdict::Accept { my_cert: acct.cert.clone(), ack }, Some(acct.user_id), acct.policy.clone(), Some(name))
                }
                Err(msg) => {
                    tracing::warn!("拒绝连接：{msg}");
                    self.log("", "", "reject", &msg, false);
                    (HostVerdict::Reject { code: "denied".into(), message: msg }, None, HostPolicy::default_for("nobody"), None)
                }
            }
        };

        let me = self.clone();
        let from_name = session_from.as_ref().map(|u| if u.display_name.is_empty() { u.username.clone() } else { format!("{}（{}）", u.display_name, u.username) }).unwrap_or_default();
        let fn2 = from_name.clone();
        let dev_hex = device.map(|d| d.short()).unwrap_or_default();
        let policy2 = policy.clone();
        let decide = move || async move {
            let ask = me.events.read().ask_assist(fn2, dev_hex);
            let allowed = tokio::time::timeout(Duration::from_secs(120), ask).await.unwrap_or(false);
            if allowed {
                let (host, _) = me.host_info(Some(&policy2));
                let perms = policy2.permissions();
                Ok(HelloAck { host, account: None, permissions: perms.clone(), capabilities: me.capabilities(&perms) })
            } else {
                Err("对方拒绝了协助请求".to_string())
            }
        };
        let r = handshake::host_finish(hs, &self.identity.key, verdict, decide).await;
        if let Some(p) = &pending {
            if r.is_err() {
                let _ = p.stop.send(true);
            }
        }
        let (_send, _recv, _hello) = r?;

        let (_, user) = self.host_info(Some(&policy));
        let id = format!("{:08x}", rand::random::<u32>());
        let path = self.ms.peer_by_virt(&virt).map(|p| p.current_path().kind).unwrap_or(PathKind::Connecting);
        let info = SessionInfo {
            id: id.clone(),
            account,
            from_user: from_name.clone(),
            from_device: device.map(|d| d.to_hex()).unwrap_or_default(),
            mode: if assist_mode { ConnMode::Assist } else { ConnMode::Control },
            path,
            started_at: galaxy_proto::now_ms(),
            ai_active: false,
        };
        let sess = Arc::new(ActiveSession {
            info: RwLock::new(info.clone()),
            account,
            policy,
            conn: conn.clone(),
            user,
            from_user_name: from_name.clone(),
            used_display: std::sync::atomic::AtomicBool::new(false),
        });
        self.sessions.lock().insert(id.clone(), sess.clone());
        self.log(&id, &from_name, "connect", &format!("{} 连接（{:?}）", from_name, info.mode), false);
        self.events.read().on_session_started(&info);
        self.emit_status();

        // 路径变化更新到会话信息
        if let Some(peer) = self.ms.peer_by_virt(&virt) {
            let mut pw = peer.path();
            let s2 = sess.clone();
            let me = self.clone();
            let c2 = conn.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        r = pw.changed() => {
                            if r.is_err() { break; }
                            s2.info.write().path = pw.borrow().kind;
                            me.emit_status();
                        }
                        _ = c2.closed() => break,
                    }
                }
            });
        }

        crate::serve::run(self.clone(), conn.clone(), sess.clone()).await;

        self.sessions.lock().remove(&id);
        // 开隐私屏的人走了，恢复本地屏幕
        let owned = self.privacy_owner.lock().as_deref() == Some(id.as_str());
        if owned {
            *self.privacy_owner.lock() = None;
            self.platform.read().set_privacy(false);
        }
        // 这个会话加的虚拟屏删掉；最后一个会话走了，无头兜底屏稍后也删掉
        self.virtual_displays_session_ended(&id).await;
        if let Some(p) = pending {
            let _ = p.stop.send(true);
        }
        if let Some(d) = device {
            // 保留路径一小会儿以便重连，之后清理
            let ms = self.ms.clone();
            let me = self.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(30)).await;
                if !me.sessions.lock().values().any(|s| s.info.read().from_device == d.to_hex()) && !me.pending.lock().contains_key(&d) {
                    ms.remove_peer(&d);
                }
            });
        }
        self.log(&id, &from_name, "disconnect", "会话结束", false);
        self.maybe_lock_after(&sess);
        self.events.read().on_session_ended(&id);
        self.emit_status();
        Ok(())
    }
}

fn random_code() -> String {
    const CH: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    (0..8).map(|_| CH[rand::random::<usize>() % CH.len()] as char).collect()
}
