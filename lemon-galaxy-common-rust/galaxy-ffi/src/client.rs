//! GalaxyClient：主控端的账号会话（每个应用一个）。

use crate::agent::{self as ag, AgentConversation};
use crate::callbacks::*;
use crate::host::HostService;
use crate::session::PeerSession;
use crate::types::*;
use crate::{config, other, rt, run, GalaxyError, Result};
use galaxy_client::account::{self, DeviceMeta, FirstLogin, LoggedIn, LoginOutcome};
use galaxy_client::api::Api;
use galaxy_client::connector::Connector;
use galaxy_client::identity::Identity;
use galaxy_client::signal::{SignalClient, SignalState};
use galaxy_client::vault::Vault;
use galaxy_crypto::AccountSecrets;
use galaxy_net::ServerAddr;
use galaxy_proto::api::{ClientDevice, ListResp, UserSelf};
use galaxy_proto::cert::SignedBlob;
use galaxy_proto::ws::WsMsg;
use galaxy_proto::DeviceId;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::sync::watch;

const SESSION_KEY: &str = "session.v1";
const VAULT_CACHE: &str = "vault-cache.v1";
const DEVICES_CACHE: &str = "devices-cache.v1";

pub(crate) struct StoreBridge(pub(crate) Arc<dyn SecureStore>);
impl galaxy_client::identity::SecureStore for StoreBridge {
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.0.get(key.to_string())
    }
    fn set(&self, key: &str, value: &[u8]) {
        self.0.set(key.to_string(), value.to_vec())
    }
    fn remove(&self, key: &str) {
        self.0.remove(key.to_string())
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Saved {
    server: String,
    token: String,
    user: UserSelf,
    account_pubkey: Vec<u8>,
    epoch: u32,
    cert: SignedBlob,
    secrets: Vec<u8>,
}

struct Online {
    signal: Arc<SignalClient>,
    connector: Arc<Connector>,
    stop: watch::Sender<bool>,
}

pub(crate) struct Inner {
    store: Arc<dyn SecureStore>,
    listener: Arc<dyn ClientListener>,
    identity: Identity,
    meta: DeviceMeta,
    state: Mutex<ClientState>,
    saved: Mutex<Option<Saved>>,
    secrets: Mutex<Option<AccountSecrets>>,
    api: Mutex<Option<Api>>,
    online: Mutex<Option<Arc<Online>>>,
    devices: Mutex<Vec<ClientDevice>>,
    vault: tokio::sync::Mutex<Option<Vault>>,
    vault_snapshot: Mutex<Vec<VaultEntry>>,
    first_login: Mutex<Option<(Api, FirstLogin)>>,
    host: Mutex<Option<Arc<HostService>>>,
    lan_active: AtomicBool,
    sessions: Mutex<HashMap<DeviceId, Weak<galaxy_client::session::PeerSession>>>,
    models: Mutex<Option<galaxy_proto::api::AiModelsResp>>,
    server_version: Mutex<String>,
    rdp_forwards: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
}

#[derive(uniffi::Object)]
pub struct GalaxyClient {
    pub(crate) inner: Arc<Inner>,
}

fn meta_from_config() -> DeviceMeta {
    let c = config();
    DeviceMeta {
        kind: c.device_kind.into(),
        platform: c.platform.into(),
        os_version: c.os_version,
        arch: std::env::consts::ARCH.into(),
        hostname: c.hostname,
        app_version: c.app_version,
    }
}

impl Inner {
    fn set_state(&self, s: ClientState) {
        let changed = {
            let mut cur = self.state.lock();
            let c = *cur != s;
            *cur = s;
            c
        };
        if changed {
            self.listener.on_state(s);
        }
    }

    pub(crate) fn api(&self) -> Result<Api> {
        self.api.lock().clone().ok_or_else(|| GalaxyError::Auth { code: "unauthorized".into(), detail: "未登录".into() })
    }

    fn wrap_key(&self) -> [u8; 32] {
        self.identity.local_wrap_key()
    }

    fn conv_key(&self) -> [u8; 32] {
        galaxy_crypto::hkdf32(&self.wrap_key(), b"lemon-galaxy/conversations/v1", b"")
    }

    fn save_session(&self, s: &Saved) {
        if let Ok(b) = serde_json::to_vec(s) {
            let (mut n, ct) = galaxy_crypto::seal(&self.wrap_key(), &b, b"lemon-galaxy/session/v1");
            n.extend_from_slice(&ct);
            self.store.set(SESSION_KEY.into(), n);
        }
    }

    fn load_session(&self) -> Option<Saved> {
        let b = self.store.get(SESSION_KEY.into())?;
        if b.len() < 24 {
            return None;
        }
        let plain = galaxy_crypto::open(&self.wrap_key(), &b[..24], &b[24..], b"lemon-galaxy/session/v1").ok()?;
        serde_json::from_slice(&plain).ok()
    }

    fn logged_in(&self) -> Option<LoggedIn> {
        let s = self.saved.lock().clone()?;
        Some(LoggedIn {
            token: s.token,
            user: s.user,
            account_pubkey: s.account_pubkey.as_slice().try_into().ok()?,
            epoch: s.epoch,
            secrets: self.secrets.lock().clone(),
            cert: s.cert,
        })
    }

    async fn refresh_devices(self: &Arc<Self>) -> Result<()> {
        let api = self.api()?;
        let r: ListResp<ClientDevice> = api.get("/devices").await?;
        if let Ok(b) = serde_json::to_vec(&r.items) {
            let (mut n, ct) = galaxy_crypto::seal(&self.wrap_key(), &b, b"lemon-galaxy/devices-cache/v1");
            n.extend_from_slice(&ct);
            self.store.set(DEVICES_CACHE.into(), n);
        }
        *self.devices.lock() = r.items;
        self.listener.on_devices_changed();
        Ok(())
    }

    fn load_device_cache(&self) {
        if let Some(b) = self.store.get(DEVICES_CACHE.into()) {
            if b.len() > 24 {
                if let Ok(p) = galaxy_crypto::open(&self.wrap_key(), &b[..24], &b[24..], b"lemon-galaxy/devices-cache/v1") {
                    if let Ok(v) = serde_json::from_slice::<Vec<ClientDevice>>(&p) {
                        *self.devices.lock() = v.into_iter().map(|mut d| {
                            d.online = false;
                            d
                        }).collect();
                    }
                }
            }
        }
    }

    async fn sync_vault(self: &Arc<Self>) -> Result<()> {
        let api = self.api()?;
        let mut g = self.vault.lock().await;
        let Some(v) = g.as_mut() else { return Ok(()) };
        let changed = v.sync(&api).await?;
        self.store.set(VAULT_CACHE.into(), v.to_cache(&self.wrap_key()));
        *self.vault_snapshot.lock() = v.entries.values().cloned().map(Into::into).collect();
        drop(g);
        if changed {
            self.listener.on_vault_changed();
        }
        Ok(())
    }

    /// 登录完成或恢复会话后：上线（信令、连接器、加密库、设备、本机被控）。
    fn go_online(self: &Arc<Self>) {
        let Some(saved) = self.saved.lock().clone() else { return };
        let Ok(server) = ServerAddr::parse(&saved.server) else { return };
        let me = self.clone();
        rt().spawn(async move {
            me.set_state(ClientState::Connecting);
            // 本地缓存先显示
            let sec_opt = me.secrets.lock().clone();
            if let Some(sec) = sec_opt {
                let mut v = Vault::new(saved.user.id, &sec);
                if let Some(c) = me.store.get(VAULT_CACHE.into()) {
                    v.load_cache(&me.wrap_key(), &c);
                }
                *me.vault_snapshot.lock() = v.entries.values().cloned().map(Into::into).collect();
                *me.vault.lock().await = Some(v);
                me.listener.on_vault_changed();
            }
            me.load_device_cache();
            me.listener.on_devices_changed();

            let signal = SignalClient::start(server.clone(), me.identity.clone(), Some(saved.token.clone()), None);
            if let Ok(api) = me.api() {
                let me2 = me.clone();
                rt().spawn(async move {
                    if let Ok(info) = api.get::<galaxy_proto::api::ServerInfoResp>("/info").await {
                        *me2.server_version.lock() = info.version;
                    }
                });
            }
            let ms = match galaxy_net::MagicSock::bind(0) {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!("无法绑定 UDP：{e}");
                    return;
                }
            };
            let ep = match galaxy_net::quic::endpoint(ms.clone(), false) {
                Ok(e) => e,
                Err(e) => {
                    tracing::error!("无法创建 QUIC 端点：{e}");
                    return;
                }
            };
            let pk: [u8; 32] = saved.account_pubkey.as_slice().try_into().unwrap_or([0; 32]);
            let connector = Connector::new(ms, ep, signal.clone(), me.identity.clone(), server.clone(), pk, saved.cert.clone());
            let (stop, mut stop_rx) = watch::channel(false);
            *me.online.lock() = Some(Arc::new(Online { signal: signal.clone(), connector, stop }));

            // 本机被控（control 客户端）
            if me.meta.kind == galaxy_proto::DeviceKind::Control && config().host_enabled && me.host.lock().is_none() {
                let cfg = galaxy_host::HostConfig {
                    server: server.clone(),
                    data_dir: std::path::PathBuf::from(config().data_dir).join("host"),
                    meta: me.meta.clone(),
                    multi_account: false,
                    file_helper: None,
                };
                match galaxy_host::HostService::start(cfg, me.identity.clone()).await {
                    Ok(h) => {
                        crate::remote_update::register_host(&h);
                        if let Some(l) = me.logged_in() {
                            let user = galaxy_host::sysuser::SysUser::current().name;
                            let policy = h.state().accounts.iter().find(|a| a.user_id == l.user.id).map(|a| a.policy.clone()).unwrap_or_else(|| galaxy_host::HostPolicy::default_for(&user));
                            let _ = h.add_account(&l, policy);
                        }
                        *me.host.lock() = Some(HostService::wrap(h));
                    }
                    Err(e) => tracing::warn!("本机被控服务启动失败：{e}"),
                }
            }

            let _ = me.refresh_devices().await;
            let _ = me.sync_vault().await;

            let mut sub = signal.subscribe();
            let mut st = signal.state();
            loop {
                tokio::select! {
                    m = sub.recv() => match m {
                        Ok(WsMsg::Presence { device_id, online }) => {
                            if let Some(d) = me.devices.lock().iter_mut().find(|d| d.id == device_id) {
                                d.online = online;
                            }
                            me.listener.on_devices_changed();
                        }
                        Ok(WsMsg::DevicesChanged) => { let _ = me.refresh_devices().await; }
                        Ok(WsMsg::VaultChanged { .. }) => { let _ = me.sync_vault().await; }
                        Ok(WsMsg::SessionRevoked) => {
                            // 确认令牌确实失效再退出
                            if let Ok(api) = me.api() {
                                if let Err(galaxy_client::ClientError::Auth { .. }) = api.get::<serde_json::Value>("/account").await {
                                    me.local_logout();
                                    me.listener.on_session_revoked();
                                    break;
                                }
                            }
                        }
                        Ok(WsMsg::AccountReset { user_id }) if Some(user_id) == me.saved.lock().as_ref().map(|s| s.user.id) => {
                            me.local_logout();
                            me.listener.on_session_revoked();
                            break;
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(_) => break,
                    },
                    r = st.changed() => {
                        if r.is_err() { break; }
                        let s = st.borrow().clone();
                        match s {
                            SignalState::Connected { .. } => {
                                me.set_state(ClientState::Online);
                                let _ = me.refresh_devices().await;
                                let _ = me.sync_vault().await;
                            }
                            SignalState::Connecting => {}
                            SignalState::Disconnected { .. } => me.set_state(ClientState::Offline),
                        }
                    }
                    _ = stop_rx.changed() => break,
                }
            }
        });
        // 局域网扫描
        let me = self.clone();
        rt().spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(8)).await;
                if me.saved.lock().is_none() {
                    break;
                }
                if !me.lan_active.load(Ordering::Relaxed) {
                    continue;
                }
                let c = me.online.lock().as_ref().map(|o| o.connector.clone());
                if let Some(c) = c {
                    c.lan_scan().await;
                    tokio::time::sleep(Duration::from_millis(600)).await;
                    me.listener.on_devices_changed();
                }
            }
        });
    }

    fn local_logout(&self) {
        // 外部智能体（MCP）用的会话随账号一起作废
        crate::mcp::reset_clients();
        if let Some(o) = self.online.lock().take() {
            let _ = o.stop.send(true);
            o.signal.stop();
        }
        self.store.remove(SESSION_KEY.into());
        self.store.remove(VAULT_CACHE.into());
        self.store.remove(DEVICES_CACHE.into());
        *self.saved.lock() = None;
        *self.secrets.lock() = None;
        *self.api.lock() = None;
        self.devices.lock().clear();
        self.vault_snapshot.lock().clear();
        if let Some(h) = self.host.lock().take() {
            h.inner.disconnect_all();
        }
        self.set_state(ClientState::LoggedOut);
    }

    fn finish_login(self: &Arc<Self>, api: Api, l: LoggedIn) {
        let server = api.server.to_string_full();
        let sec = l.secrets.clone();
        let saved = Saved {
            server,
            token: l.token.clone(),
            user: l.user.clone(),
            account_pubkey: l.account_pubkey.to_vec(),
            epoch: l.epoch,
            cert: l.cert.clone(),
            secrets: sec.as_ref().map(|s| s.wrap_local(&self.wrap_key())).unwrap_or_default(),
        };
        self.save_session(&saved);
        *self.saved.lock() = Some(saved);
        *self.secrets.lock() = sec;
        api.set_token(Some(l.token.clone()));
        *self.api.lock() = Some(api);
        self.go_online();
    }

    async fn session_for(self: &Arc<Self>, d: DeviceId) -> Result<Arc<galaxy_client::session::PeerSession>> {
        if let Some(s) = self.sessions.lock().get(&d).and_then(|w| w.upgrade()) {
            if !s.is_closed() {
                return Ok(s);
            }
        }
        let c = self.online.lock().as_ref().map(|o| o.connector.clone()).ok_or_else(|| GalaxyError::Network { detail: "尚未连上服务器".into() })?;
        let s = c.connect(d).await?;
        self.sessions.lock().insert(d, Arc::downgrade(&s));
        Ok(s)
    }

    /// 连接加密库里的 ssh_host（可经设备或另一台 SSH 主机跳转），新确认的主机密钥存回加密库。
    fn connect_ssh_inner(self: Arc<Self>, host_id: String, prompt: Arc<dyn SecretPrompt>, depth: u32) -> futures::future::BoxFuture<'static, Result<(Arc<galaxy_client::ssh::SshSession>, crate::ssh::SshInfo)>> {
        use galaxy_client::ssh as gs;
        use galaxy_client::vault::{SshAuth, VaultData as VD};
        Box::pin(async move {
            if depth > 3 {
                return Err(other("跳转层级太多"));
            }
            let entries = {
                let g = self.vault.lock().await;
                g.as_ref().map(|v| v.entries.clone()).ok_or_else(|| other("加密库不可用"))?
            };
            let e = entries.get(&host_id).ok_or_else(|| GalaxyError::NotFound { detail: "主机不存在".into() })?;
            let VD::SshHost { label, host, port, username, auth, jump_device_id, jump_host_id, .. } = &e.data else {
                return Err(other("这个条目不是 SSH 主机"));
            };
            let auth = match auth {
                SshAuth::Password { password } => gs::SshAuthMethod::Password(password.clone()),
                SshAuth::PrivateKey { credential_id } => match entries.get(credential_id).map(|c| &c.data) {
                    Some(VD::Credential { private_key: Some(k), passphrase, .. }) if !k.trim().is_empty() => gs::SshAuthMethod::PrivateKey { key: k.clone(), passphrase: passphrase.clone().filter(|p| !p.is_empty()) },
                    Some(VD::Credential { password: Some(p), .. }) => gs::SshAuthMethod::Password(Some(p.clone())),
                    _ => return Err(GalaxyError::NotFound { detail: "这台主机引用的凭据不存在或没有私钥".into() }),
                },
                SshAuth::Agent => return Err(GalaxyError::Permission { detail: "暂不支持 SSH Agent，请改用私钥或密码".into() }),
                SshAuth::Interactive => gs::SshAuthMethod::Interactive,
            };
            let (via, via_name) = if let Some(d) = jump_device_id.clone().filter(|x| !x.is_empty()) {
                let id = parse_dev(&d)?;
                let name = self.device_name(&id);
                (gs::SshVia::Device(self.session_for(id).await?), format!("device:{name}"))
            } else if let Some(j) = jump_host_id.clone().filter(|x| !x.is_empty()) {
                let (js, ji) = self.clone().connect_ssh_inner(j, prompt.clone(), depth + 1).await?;
                (gs::SshVia::Ssh(js), format!("ssh:{}", if ji.label.is_empty() { ji.host } else { ji.label }))
            } else {
                (gs::SshVia::Direct, "direct".to_string())
            };
            let known = entries.values().find_map(|k| match &k.data {
                VD::KnownHost { host: h, port: p, key_type, fingerprint } if h.eq_ignore_ascii_case(host) && p == port => Some((k.id.clone(), gs::KnownHostKey { key_type: key_type.clone(), fingerprint: fingerprint.clone() })),
                _ => None,
            });
            let target = gs::SshTarget { host: host.clone(), port: *port, username: username.clone(), auth, via, known: known.as_ref().map(|k| k.1.clone()) };
            let c = gs::SshSession::connect(target, Arc::new(crate::ssh::PromptBridge(prompt))).await?;
            if let Some(k) = c.new_host_key {
                let entry = galaxy_client::vault::VaultEntry {
                    id: known.map(|x| x.0).unwrap_or_default(),
                    folder_id: None,
                    sort: 0,
                    data: VD::KnownHost { host: host.clone(), port: *port, key_type: k.key_type, fingerprint: k.fingerprint },
                    updated_at: 0,
                    version: 0,
                };
                if let Err(e) = self.save_entry(entry).await {
                    tracing::warn!("主机密钥没能存进加密库：{e}");
                }
            }
            let info = crate::ssh::SshInfo { host_id, label: label.clone(), host: host.clone(), port: *port, username: username.clone(), via: via_name };
            Ok((c.session, info))
        })
    }

    async fn save_entry(self: &Arc<Self>, e: galaxy_client::vault::VaultEntry) -> Result<()> {
        let api = self.api()?;
        let mut g = self.vault.lock().await;
        let v = g.as_mut().ok_or_else(|| other("加密库不可用"))?;
        v.save(&api, e).await?;
        self.store.set(VAULT_CACHE.into(), v.to_cache(&self.wrap_key()));
        *self.vault_snapshot.lock() = v.entries.values().cloned().map(Into::into).collect();
        drop(g);
        self.listener.on_vault_changed();
        Ok(())
    }

    fn device_name(&self, d: &DeviceId) -> String {
        let alias = self.vault_snapshot.lock().iter().find_map(|e| match &e.data {
            VaultData::DeviceMeta { device_id, alias: Some(a), .. } if *device_id == d.to_hex() && !a.is_empty() => Some(a.clone()),
            _ => None,
        });
        alias.or_else(|| self.devices.lock().iter().find(|x| x.id == *d).map(|x| x.hostname.clone())).unwrap_or_else(|| d.short())
    }
}

fn parse_dev(id: &str) -> Result<DeviceId> {
    DeviceId::from_hex(id).ok_or_else(|| other("设备 ID 格式错误"))
}

#[uniffi::export]
impl GalaxyClient {
    #[uniffi::constructor]
    pub fn new(store: Arc<dyn SecureStore>, listener: Arc<dyn ClientListener>) -> Arc<Self> {
        galaxy_net::init_crypto();
        let bridge: Arc<dyn galaxy_client::identity::SecureStore> = Arc::new(StoreBridge(store.clone()));
        let identity = Identity::load_or_create(&bridge);
        Arc::new(Self {
            inner: Arc::new(Inner {
                store,
                listener,
                identity,
                meta: meta_from_config(),
                state: Mutex::new(ClientState::LoggedOut),
                saved: Mutex::new(None),
                secrets: Mutex::new(None),
                api: Mutex::new(None),
                online: Mutex::new(None),
                devices: Mutex::new(vec![]),
                vault: tokio::sync::Mutex::new(None),
                vault_snapshot: Mutex::new(vec![]),
                first_login: Mutex::new(None),
                host: Mutex::new(None),
                lan_active: AtomicBool::new(false),
                sessions: Mutex::new(HashMap::new()),
                models: Mutex::new(None),
                server_version: Mutex::new(String::new()),
                rdp_forwards: Mutex::new(HashMap::new()),
            }),
        })
    }

    pub fn device_id(&self) -> String {
        self.inner.identity.device_id.to_hex()
    }

    pub fn state(&self) -> ClientState {
        *self.inner.state.lock()
    }

    pub fn current_user(&self) -> Option<UserInfo> {
        self.inner.saved.lock().as_ref().map(|s| UserInfo { id: s.user.id, username: s.user.username.clone(), display_name: s.user.display_name.clone(), ai_enabled: s.user.ai_enabled, totp_enabled: s.user.totp_enabled })
    }

    pub fn server(&self) -> Option<ServerInfo> {
        self.inner.saved.lock().as_ref().map(|s| ServerInfo { address: s.server.clone(), name: "LemonGalaxy".into(), version: self.inner.server_version.lock().clone() })
    }

    /// 启动时用本机保存的会话恢复登录（不需要联网就能显示缓存的设备与主机）。
    pub fn restore(&self) -> bool {
        let Some(s) = self.inner.load_session() else { return false };
        let Ok(server) = ServerAddr::parse(&s.server) else { return false };
        let Ok(api) = Api::new(server, &crate::locale()) else { return false };
        api.set_token(Some(s.token.clone()));
        let sec = AccountSecrets::unwrap_local(&self.inner.wrap_key(), &s.secrets).ok();
        *self.inner.secrets.lock() = sec;
        *self.inner.saved.lock() = Some(s);
        *self.inner.api.lock() = Some(api);
        self.inner.go_online();
        true
    }

    /// 登录。server 为后台给出的完整地址 "host:port#sha256-指纹"。
    pub async fn login(&self, server: String, username: String, password: String, totp: Option<String>) -> Result<LoginResult> {
        let inner = self.inner.clone();
        run(async move {
            let addr = ServerAddr::parse(&server).map_err(|e| GalaxyError::Auth { code: "bad_server".into(), detail: e })?;
            let api = Api::new(addr, &crate::locale())?;
            match account::login(&api, &inner.identity, &inner.meta, &username, &password, totp.as_deref()).await? {
                LoginOutcome::Done(l) => {
                    inner.finish_login(api, l);
                    Ok(LoginResult::Success)
                }
                LoginOutcome::NeedTotp => Ok(LoginResult::NeedTotp),
                LoginOutcome::MustChangePassword(fl) => {
                    *inner.first_login.lock() = Some((api, fl));
                    Ok(LoginResult::MustChangePassword)
                }
            }
        })
        .await
    }

    /// 首次登录设置新密码（会生成账号密钥）。
    pub async fn complete_first_login(&self, new_password: String) -> Result<()> {
        let inner = self.inner.clone();
        run(async move {
            let (api, fl) = inner.first_login.lock().take().ok_or_else(|| other("请先登录"))?;
            let l = account::complete_first_login(&api, &inner.identity, &inner.meta, fl, &new_password).await?;
            inner.finish_login(api, l);
            Ok(())
        })
        .await
    }

    pub async fn change_password(&self, old_password: String, new_password: String, totp: Option<String>) -> Result<()> {
        let inner = self.inner.clone();
        run(async move {
            let api = inner.api()?;
            let username = inner.saved.lock().as_ref().map(|s| s.user.username.clone()).ok_or_else(|| other("未登录"))?;
            let fresh = Api::new(api.server.clone(), &crate::locale())?;
            let l = account::change_password(&fresh, &inner.identity, &inner.meta, &username, &old_password, &new_password, totp.as_deref()).await?;
            if let Some(s) = inner.saved.lock().as_mut() {
                s.token = l.token.clone();
            }
            if let Some(s) = inner.saved.lock().clone() {
                inner.save_session(&s);
            }
            api.set_token(Some(l.token));
            Ok(())
        })
        .await
    }

    pub async fn logout(&self) -> Result<()> {
        let inner = self.inner.clone();
        run(async move {
            if let Ok(api) = inner.api() {
                let _ = api.post::<_, serde_json::Value>("/auth/logout", &serde_json::json!({})).await;
            }
            inner.local_logout();
            Ok(())
        })
        .await
    }

    pub async fn totp_setup(&self) -> Result<TotpSetup> {
        let inner = self.inner.clone();
        run(async move {
            let r: galaxy_proto::api::TotpSetupResp = inner.api()?.post("/auth/totp/setup", &serde_json::json!({})).await?;
            Ok(TotpSetup { secret: r.secret, otpauth_uri: r.otpauth_uri })
        })
        .await
    }

    pub async fn totp_verify(&self, code: String) -> Result<()> {
        let inner = self.inner.clone();
        run(async move {
            let _: serde_json::Value = inner.api()?.post("/auth/totp/verify", &serde_json::json!({ "code": code })).await?;
            if let Some(s) = inner.saved.lock().as_mut() {
                s.user.totp_enabled = true;
            }
            Ok(())
        })
        .await
    }

    pub async fn totp_disable(&self, code: String) -> Result<()> {
        let inner = self.inner.clone();
        run(async move {
            let _: serde_json::Value = inner.api()?.post("/auth/totp/disable", &serde_json::json!({ "code": code })).await?;
            if let Some(s) = inner.saved.lock().as_mut() {
                s.user.totp_enabled = false;
            }
            Ok(())
        })
        .await
    }

    // ---------------- 设备与加密库 ----------------

    pub fn devices(&self) -> Vec<DeviceInfo> {
        let me = self.inner.identity.device_id;
        let c = self.inner.online.lock().as_ref().map(|o| o.connector.clone());
        self.inner
            .devices
            .lock()
            .iter()
            .map(|d| DeviceInfo {
                id: d.id.to_hex(),
                kind: d.kind.into(),
                platform: d.platform.into(),
                hostname: d.hostname.clone(),
                os_version: d.os_version.clone(),
                arch: d.arch.clone(),
                app_version: d.app_version.clone(),
                online: d.online,
                lan_seen: c.as_ref().map(|c| c.lan_seen(&d.id)).unwrap_or(false),
                last_seen_at: d.last_seen_at,
                is_self: d.id == me,
                update_version: d.update_version.clone(),
            })
            .collect()
    }

    /// 远程升级被控端（设备详情里的「升级被控端」，不用先开远程桌面）：连上对方发请求，对方自己走完整个零交互升级；
    /// 这里接着轮询设备列表，直到它以新版本重新上线（最多 10 分钟）。进度经 listener，立即返回。
    pub fn upgrade_device(&self, device_id: String, listener: Arc<dyn crate::callbacks::UpgradeListener>) {
        let inner = self.inner.clone();
        crate::rt().spawn(async move {
            // 升级特有的几种失败按界面语言给文字（连不上等通用错误照内核原样）
            let zh = crate::locale().starts_with("zh");
            let tr = move |c: String, e: String| if zh { c } else { e };
            let r: std::result::Result<Option<String>, String> = async {
                let d = parse_dev(&device_id).map_err(|e| e.to_string())?;
                let target = {
                    let s = inner.session_for(d).await.map_err(|e| e.to_string())?;
                    if !s.ack.capabilities.iter().any(|c| c == galaxy_proto::peer::CAP_REMOTE_UPDATE) {
                        return Err(tr(
                            "对方版本太旧，不支持远程升级，需要先在那台设备上更新一次".into(),
                            "This device is too old for remote upgrade. Update it once on the device itself.".into(),
                        ));
                    }
                    match s.apply_update().await {
                        Ok(v) => v,
                        Err(galaxy_client::ClientError::Permission(_)) => {
                            return Err(tr("没有控制这台设备的权限，不能升级".into(), "You don't have control permission on this device, so it can't be upgraded.".into()))
                        }
                        Err(e) => return Err(e.to_string()),
                    }
                };
                let Some(target) = target else {
                    let cur = inner.devices.lock().iter().find(|x| x.id == d).map(|x| x.app_version.clone());
                    return Ok(cur);
                };
                listener.on_started(target.clone());
                // 对方断开后旧会话作废，别再复用
                inner.sessions.lock().remove(&d);
                let t0 = std::time::Instant::now();
                let mut told_offline = false;
                // 断开后又以旧版本上线的时刻：持续一分钟仍是旧版本就算没装上（短暂断网再连上时还会继续等）
                let mut back_old: Option<std::time::Instant> = None;
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    let _ = inner.refresh_devices().await;
                    let (online, ver) = inner.devices.lock().iter().find(|x| x.id == d).map(|x| (x.online, x.app_version.clone())).unwrap_or((false, String::new()));
                    if !online && !told_offline {
                        told_offline = true;
                        listener.on_offline();
                    }
                    if online && ver == target {
                        return Ok(Some(ver));
                    }
                    if told_offline && online && !ver.is_empty() {
                        let since = *back_old.get_or_insert_with(std::time::Instant::now);
                        if since.elapsed() > std::time::Duration::from_secs(60) {
                            return Err(tr(
                                format!("对方重新上线了，但仍是 {ver}，新版本 {target} 没有装上（详情看对方的日志）"),
                                format!("The device came back online still on {ver}; {target} was not installed (see the device's log)."),
                            ));
                        }
                    } else {
                        back_old = None;
                    }
                    if t0.elapsed() > std::time::Duration::from_secs(600) {
                        let ver = if ver.is_empty() { "?".to_string() } else { ver };
                        return Err(tr(
                            format!("10 分钟内没有以新版本 {target} 重新上线（现在 {ver}，{}）", if online { "在线" } else { "离线" }),
                            format!("The device did not come back online on {target} within 10 minutes (now {ver}, {}).", if online { "online" } else { "offline" }),
                        ));
                    }
                }
            }
            .await;
            match r {
                Ok(v) => listener.on_finished(v, None),
                Err(e) => listener.on_finished(None, Some(e)),
            }
        });
    }

    pub async fn refresh_devices(&self) -> Result<()> {
        let inner = self.inner.clone();
        run(async move { inner.refresh_devices().await }).await
    }

    pub async fn remove_device(&self, device_id: String) -> Result<()> {
        let inner = self.inner.clone();
        run(async move {
            let _: serde_json::Value = inner.api()?.delete(&format!("/devices/{device_id}")).await?;
            // 移除的是本机：服务器已作废本机的登录，直接退出到登录页
            if device_id.eq_ignore_ascii_case(&inner.identity.device_id.to_hex()) {
                inner.local_logout();
                return Ok(());
            }
            inner.refresh_devices().await
        })
        .await
    }

    /// 设备列表页打开期间设为 true：每隔几秒查询局域网内的同账号设备。
    pub fn set_lan_discovery(&self, active: bool) {
        self.inner.lan_active.store(active, Ordering::Relaxed);
        if active {
            let c = self.inner.online.lock().as_ref().map(|o| o.connector.clone());
            if let Some(c) = c {
                rt().spawn(async move { c.lan_scan().await });
            }
        }
    }

    pub fn vault(&self) -> Vec<VaultEntry> {
        self.inner.vault_snapshot.lock().clone()
    }

    /// 保存加密库条目（id 为空则新建），返回保存后的条目。
    pub async fn vault_save(&self, entry: VaultEntry) -> Result<VaultEntry> {
        let inner = self.inner.clone();
        run(async move {
            let api = inner.api()?;
            let mut g = inner.vault.lock().await;
            let v = g.as_mut().ok_or_else(|| other("加密库不可用"))?;
            let e = galaxy_client::vault::VaultEntry { id: entry.id, folder_id: entry.folder_id, sort: entry.sort, data: entry.data.into(), updated_at: 0, version: 0 };
            let e = v.save(&api, e).await?;
            inner.store.set(VAULT_CACHE.into(), v.to_cache(&inner.wrap_key()));
            *inner.vault_snapshot.lock() = v.entries.values().cloned().map(Into::into).collect();
            drop(g);
            inner.listener.on_vault_changed();
            Ok(e.into())
        })
        .await
    }

    pub async fn vault_delete(&self, id: String) -> Result<()> {
        let inner = self.inner.clone();
        run(async move {
            let api = inner.api()?;
            let mut g = inner.vault.lock().await;
            let v = g.as_mut().ok_or_else(|| other("加密库不可用"))?;
            v.delete(&api, &id).await?;
            inner.store.set(VAULT_CACHE.into(), v.to_cache(&inner.wrap_key()));
            *inner.vault_snapshot.lock() = v.entries.values().cloned().map(Into::into).collect();
            drop(g);
            inner.listener.on_vault_changed();
            Ok(())
        })
        .await
    }

    // ---------------- 远程开机 ----------------

    /// 能不能远程开机：设备离线、能被控、上报过网卡地址（界面据此显示「远程开机」）。
    pub fn can_wake(&self, device_id: String) -> bool {
        let Ok(d) = parse_dev(&device_id) else { return false };
        self.inner.devices.lock().iter().find(|x| x.id == d).map(galaxy_client::wake::can_wake).unwrap_or(false)
    }

    /// 远程开机：由本机（与目标同一网络时）或同一网络里在线的设备代发唤醒包。
    /// 返回代发设备的名字，空字符串表示本机。目标要在主板 / 网卡设置里开启网络唤醒。
    pub async fn wake_device(&self, device_id: String) -> Result<Vec<String>> {
        let inner = self.inner.clone();
        run(async move {
            let d = parse_dev(&device_id)?;
            let c = inner.online.lock().as_ref().map(|o| o.connector.clone()).ok_or_else(|| GalaxyError::Network { detail: "尚未连上服务器".into() })?;
            let list = inner.devices.lock().clone();
            Ok(galaxy_client::wake::wake(&c, &list, inner.identity.device_id, d).await?)
        })
        .await
    }

    // ---------------- 连接 ----------------

    pub async fn connect(&self, device_id: String) -> Result<Arc<PeerSession>> {
        let inner = self.inner.clone();
        run(async move {
            let d = parse_dev(&device_id)?;
            let c = inner.online.lock().as_ref().map(|o| o.connector.clone()).ok_or_else(|| GalaxyError::Network { detail: "尚未连上服务器".into() })?;
            let s = c.connect(d).await?;
            inner.sessions.lock().insert(d, Arc::downgrade(&s));
            Ok(PeerSession::wrap(s, inner.device_name(&d)))
        })
        .await
    }

    pub async fn connect_assist(&self, device_code: String, verification_code: String) -> Result<Arc<PeerSession>> {
        let inner = self.inner.clone();
        run(async move {
            let c = inner.online.lock().as_ref().map(|o| o.connector.clone()).ok_or_else(|| GalaxyError::Network { detail: "尚未连上服务器".into() })?;
            let s = c.connect_assist(&device_code, &verification_code).await?;
            let name = s.ack.host.hostname.clone();
            Ok(PeerSession::wrap(s, name))
        })
        .await
    }

    // ---------------- AI ----------------

    pub async fn ai_models(&self) -> Result<AiModels> {
        let inner = self.inner.clone();
        run(async move {
            let r: galaxy_proto::api::AiModelsResp = inner.api()?.get("/ai/models").await?;
            *inner.models.lock() = Some(r.clone());
            Ok(AiModels {
                enabled: r.enabled,
                max_mode: r.max_mode.into(),
                items: r.items.into_iter().map(|m| AiModel { provider_id: m.provider_id, provider_name: m.provider_name, id: m.id, name: m.name, vision: m.vision, tools: m.tools, context_length: m.context_length }).collect(),
            })
        })
        .await
    }

    /// 新建对话：先连上所有目标设备。
    pub async fn new_conversation(&self, config: ConversationConfig, listener: Arc<dyn AgentListener>) -> Result<Arc<AgentConversation>> {
        let inner = self.inner.clone();
        run(async move { build_conversation(inner, config, listener, None).await }).await
    }

    /// 账号最近的活动（登录、连接等），新的在前。
    pub async fn account_activity(&self, limit: u32) -> Result<Vec<ActivityItem>> {
        let inner = self.inner.clone();
        run(async move { Ok(inner.activity(limit).await?.into_iter().map(Into::into).collect()) }).await
    }

    pub fn list_conversations(&self) -> Vec<ConversationSummary> {
        ag::list_conversations(&self.inner.conv_key())
    }

    /// 打开本机保存的对话（需要重新连接目标设备）。`config` 为空时用创建时保存的配置。
    pub async fn open_conversation(&self, id: String, config: Option<ConversationConfig>, listener: Arc<dyn AgentListener>) -> Result<Arc<AgentConversation>> {
        let inner = self.inner.clone();
        run(async move {
            let path = ag::conv_dir().join(format!("{}.bin", id.chars().filter(|x| x.is_ascii_alphanumeric() || *x == '-').collect::<String>()));
            let saved = ag::load_conv(&inner.conv_key(), &path).ok_or_else(|| GalaxyError::NotFound { detail: "对话不存在".into() })?;
            let config = config.or_else(|| saved.config.clone()).ok_or_else(|| GalaxyError::NotFound { detail: "这条对话没有保存配置，请重新选择模型和设备".into() })?;
            build_conversation(inner, config, listener, Some(saved)).await
        })
        .await
    }

    pub fn delete_conversation(&self, id: String) {
        let safe: String = id.chars().filter(|x| x.is_ascii_alphanumeric() || *x == '-').collect();
        let _ = std::fs::remove_file(ag::conv_dir().join(format!("{safe}.bin")));
    }

    // ---------------- 本机被控 ----------------

    // ---------------- RDP ----------------

    /// 准备连接加密库里的 rdp_host：设了经由设备时在本机开端口转发穿过那台设备。
    /// 界面拿返回的地址交给系统的远程桌面客户端（Windows 的 mstsc，其他平台的微软 Windows App）。
    pub async fn open_rdp(&self, host_id: String) -> Result<RdpTarget> {
        let inner = self.inner.clone();
        run(async move {
            use galaxy_client::vault::VaultData as VD;
            let entries = {
                let g = inner.vault.lock().await;
                g.as_ref().map(|v| v.entries.clone()).ok_or_else(|| other("加密库不可用"))?
            };
            let e = entries.get(&host_id).ok_or_else(|| GalaxyError::NotFound { detail: "主机不存在".into() })?;
            let VD::RdpHost { label, host, port, username, password, domain, jump_device_id, .. } = &e.data else {
                return Err(other("这个条目不是 RDP 主机"));
            };
            let port = if *port == 0 { 3389 } else { *port };
            match jump_device_id.clone().filter(|d| !d.is_empty()) {
                Some(d) => {
                    let s = inner.session_for(parse_dev(&d)?).await?;
                    let (local, h) = s.forward_port(0, host.clone(), port).await?;
                    let id = uuid::Uuid::new_v4().simple().to_string();
                    inner.rdp_forwards.lock().insert(id.clone(), h);
                    Ok(RdpTarget { label: label.clone(), host: "127.0.0.1".into(), port: local, username: username.clone(), domain: domain.clone(), password: password.clone(), forwarded: true, forward_id: id })
                }
                None => Ok(RdpTarget { label: label.clone(), host: host.clone(), port, username: username.clone(), domain: domain.clone(), password: password.clone(), forwarded: false, forward_id: String::new() }),
            }
        })
        .await
    }

    /// 按加密库里的端口映射规则开启映射：经由设备时连上该设备转发，经由 SSH 主机时连上该主机做本地转发。
    /// 返回的 PortForward 一直有效直到 stop()（期间保持与设备 / 主机的连接）。
    pub async fn start_port_forward(&self, entry_id: String, prompt: Arc<dyn SecretPrompt>) -> Result<Arc<crate::session::PortForward>> {
        let inner = self.inner.clone();
        run(async move {
            use galaxy_client::vault::VaultData as VD;
            let entries = {
                let g = inner.vault.lock().await;
                g.as_ref().map(|v| v.entries.clone()).ok_or_else(|| other("加密库不可用"))?
            };
            let e = entries.get(&entry_id).ok_or_else(|| GalaxyError::NotFound { detail: "端口映射规则不存在".into() })?;
            let VD::PortForward { via_device_id, via_ssh_host_id, local_port, remote_host, remote_port, .. } = &e.data else {
                return Err(other("这个条目不是端口映射规则"));
            };
            if remote_host.trim().is_empty() || *remote_port == 0 {
                return Err(other("目标地址或端口没填"));
            }
            let (host, port) = (remote_host.trim().to_string(), *remote_port);
            if let Some(d) = via_device_id.clone().filter(|d| !d.is_empty()) {
                let dev = parse_dev(&d)?;
                // 先连一次，设备不在线时立即报错
                let first = inner.session_for(dev).await?;
                let l = tokio::net::TcpListener::bind(("127.0.0.1", *local_port)).await.map_err(|e| other(&format!("本机端口 {local_port} 用不了：{e}")))?;
                let bound = l.local_addr().map(|a| a.port()).unwrap_or(*local_port);
                // 每来一个连接都用当前可用的会话；断线后下一个连接自动重连，映射一直有效
                let cur = Arc::new(tokio::sync::Mutex::new(Some(first)));
                let h = tokio::spawn(async move {
                    while let Ok((tcp, _)) = l.accept().await {
                        let (inner, cur, host) = (inner.clone(), cur.clone(), host.clone());
                        tokio::spawn(async move {
                            let s = {
                                let mut g = cur.lock().await;
                                match g.as_ref().filter(|s| !s.is_closed()) {
                                    Some(s) => s.clone(),
                                    None => match inner.session_for(dev).await {
                                        Ok(s) => {
                                            *g = Some(s.clone());
                                            s
                                        }
                                        Err(_) => return,
                                    },
                                }
                            };
                            let (mut ws, mut wr) = match s.open_tcp(&host, port).await {
                                Ok(x) => x,
                                Err(_) => {
                                    // 可能是被控端重启了而旧连接还没超时：探一下，确实失联就丢掉旧会话，下一个连接重连
                                    let alive = tokio::time::timeout(std::time::Duration::from_secs(5), s.mgmt(galaxy_proto::peer::MgmtReq::HostStatus)).await.map(|r| r.is_ok()).unwrap_or(false);
                                    if !alive {
                                        s.close();
                                        let mut g = cur.lock().await;
                                        if g.as_ref().map(|x| Arc::ptr_eq(x, &s)).unwrap_or(false) {
                                            *g = None;
                                        }
                                    }
                                    return;
                                }
                            };
                            let (mut tr, mut tw) = tokio::io::split(tcp);
                            let a = async {
                                let _ = tokio::io::copy(&mut tr, &mut ws).await;
                                let _ = ws.finish();
                            };
                            let b = async {
                                let _ = tokio::io::copy(&mut wr, &mut tw).await;
                            };
                            tokio::join!(a, b);
                        });
                    }
                });
                return Ok(Arc::new(crate::session::PortForward { port: bound, handle: parking_lot::Mutex::new(Some(h)) }));
            }
            if let Some(hid) = via_ssh_host_id.clone().filter(|d| !d.is_empty()) {
                let (s, _) = inner.connect_ssh_inner(hid, prompt, 0).await?;
                let (p, h) = s.forward_local(*local_port, host, port).await?;
                return Ok(Arc::new(crate::session::PortForward { port: p, handle: parking_lot::Mutex::new(Some(h)) }));
            }
            Err(other("没有指定经由哪台设备或 SSH 主机"))
        })
        .await
    }

    /// 关掉 open_rdp 开的端口转发。
    pub fn close_rdp(&self, forward_id: String) {
        if let Some(h) = self.inner.rdp_forwards.lock().remove(&forward_id) {
            h.abort();
        }
    }

    // ---------------- 自动更新 ----------------

    /// 向服务器查询新版本。`component`：control / mobile / worker；`platform`：macos / android / windows / ios；
    /// `arch` 为空表示不限；`current` 是本机版本。返回前已验发布签名。
    pub async fn check_update(&self, component: String, platform: String, arch: String, current: String) -> Result<Option<UpdateInfo>> {
        let inner = self.inner.clone();
        run(async move {
            let api = inner.api()?;
            Ok(galaxy_client::update::check(&api, &component, &platform, &arch, &current).await?.map(Into::into))
        })
        .await
    }

    /// 下载新版本到 `dest`（本地文件路径），核对 SHA-256。进度经 listener 的 on_progress 上报，完成或失败经 on_finished。
    pub fn download_update(&self, info: UpdateInfo, dest: String, listener: Arc<dyn TransferListener>) {
        let inner = self.inner.clone();
        rt().spawn(async move {
            let r = async {
                let api = inner.api()?;
                let i = galaxy_client::update::UpdateInfo { version: info.version, file_name: info.file_name, size: info.size, sha256: info.sha256, url: info.url, notes: info.notes };
                let l = listener.clone();
                galaxy_client::update::download(&api, &i, std::path::Path::new(&dest), &move |d, t| l.on_progress(d, t)).await.map_err(GalaxyError::from)
            }
            .await;
            listener.on_finished(r.err().map(|e| e.to_string()));
        });
    }

    // ---------------- SSH 直连 ----------------

    /// 连接加密库里的 ssh_host 条目。需要密码、口令或确认主机密钥时经 `prompt` 问用户。
    pub async fn connect_ssh(&self, host_id: String, prompt: Arc<dyn SecretPrompt>) -> Result<Arc<crate::ssh::SshSession>> {
        let inner = self.inner.clone();
        run(async move {
            let (s, info) = inner.connect_ssh_inner(host_id, prompt, 0).await?;
            Ok(Arc::new(crate::ssh::SshSession { inner: s, info }))
        })
        .await
    }

    /// control 客户端的本机被控服务；登录完成并上线后才有。
    pub fn host(&self) -> Option<Arc<HostService>> {
        self.inner.host.lock().clone()
    }
}

async fn build_conversation(inner: Arc<Inner>, c: ConversationConfig, listener: Arc<dyn AgentListener>, restore: Option<ag::SavedConversation>) -> Result<Arc<AgentConversation>> {
    let api = inner.api()?;
    let models = inner.ai_models_cached().await?;
    if !models.enabled {
        return Err(GalaxyError::Permission { detail: "未开通 AI 功能".into() });
    }
    let m = models.items.iter().find(|m| m.provider_id == c.provider_id && m.id == c.model_id).cloned().ok_or_else(|| GalaxyError::Permission { detail: "未开通该模型".into() })?;
    let exec = galaxy_client::ai::SessionExecutor::new();
    let ws = Arc::new(ChatWorkspace { inner: Arc::downgrade(&inner), exec: exec.clone() });
    let id = restore.as_ref().map(|r| r.id.clone()).unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
    let cfg = galaxy_agent::agent::AgentConfig {
        conversation_id: id,
        provider_id: m.provider_id,
        model_id: m.id.clone(),
        vision: m.vision,
        context_length: m.context_length,
        mode: c.mode.into(),
        admin_cap: models.max_mode,
        plan_first: c.plan_first,
        max_steps: 100,
        locale: crate::locale(),
        trust_minutes: c.trust_minutes,
    };
    let title = c.title.clone().or_else(|| restore.as_ref().map(|r| r.title.clone())).unwrap_or_default();
    let names: Vec<String> = c.targets.iter().map(|t| ws.name_of(&t.id)).collect();
    // 桌面端：本机直接可用，不用添加或引用
    if let Some(l) = inner.local_ai() {
        exec.add_local(LOCAL_TARGET.into(), l, inner.local_name());
    }
    let conv = AgentConversation::build(cfg, Arc::new(galaxy_client::ai::GatewayLlm { api }), exec, ws.clone(), listener, inner.conv_key(), title, names, c.clone());
    if let Some(r) = restore {
        conv.agent.restore(r.messages);
    }
    if ws.exec.has(LOCAL_TARGET) {
        conv.agent.authorize(vec![LOCAL_TARGET.into()]);
    }
    // 建对话时选的目标（老界面）：视同用户引用过，用到时再连
    conv.agent.authorize(c.targets.iter().map(|t| t.id.clone()).collect());
    Ok(conv)
}

/// AI 里「本机」的目标 ID
pub(crate) const LOCAL_TARGET: &str = "local";

impl Inner {
    /// 本机执行后端：桌面客户端才有（手机不能跑命令）。Windows 主控端的被控是系统服务，命令、文件、终端照样在本进程执行
    pub(crate) fn local_ai(&self) -> Option<Arc<dyn galaxy_client::ai::LocalHost>> {
        use galaxy_proto::Platform::*;
        if !matches!(self.meta.platform, Macos | Windows | Linux) {
            return None;
        }
        let h = self.host.lock().clone();
        Some(Arc::new(crate::local_ai::LocalAi {
            host: h.map(|h| h.inner.clone()),
            platform: self.meta.platform,
            os: self.meta.os_version.clone(),
            user_name: galaxy_host::sysuser::SysUser::current().name,
        }))
    }

    pub(crate) fn local_name(&self) -> String {
        if self.meta.hostname.is_empty() { "本机".into() } else { format!("本机（{}）", self.meta.hostname) }
    }

    pub(crate) async fn ai_models_cached(&self) -> Result<galaxy_proto::api::AiModelsResp> {
        let cached = self.models.lock().clone();
        match cached {
            Some(m) => Ok(m),
            None => {
                let r: galaxy_proto::api::AiModelsResp = self.api()?.get("/ai/models").await?;
                *self.models.lock() = Some(r.clone());
                Ok(r)
            }
        }
    }

    pub(crate) async fn activity(&self, limit: u32) -> Result<Vec<galaxy_proto::api::ActivityItem>> {
        let r: galaxy_proto::api::ListResp<galaxy_proto::api::ActivityItem> = self.api()?.get(&format!("/account/activity?limit={}", limit.clamp(1, 200))).await?;
        Ok(r.items)
    }
}

fn fmt_time(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms).single().map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_default()
}

pub(crate) fn activity_text(items: &[galaxy_proto::api::ActivityItem]) -> String {
    let mut out = String::new();
    for a in items {
        let what = match a.action.as_str() {
            "connection.start" => "开始连接",
            "connection.end" => "结束连接",
            "user.login" => "登录",
            "user.login_failed" => "登录失败",
            "device.register" => "登记设备",
            "user.password_change" => "修改密码",
            x => x,
        };
        let detail = serde_json::from_str::<serde_json::Value>(&a.detail).ok().map(|v| {
            let mut d = vec![];
            if let Some(ms) = v["duration_ms"].as_i64() {
                d.push(format!("时长 {} 秒", ms / 1000));
            }
            if let Some(p) = v["path"].as_str() {
                d.push(format!("路径 {p}"));
            }
            if let Some(m) = v["mode"].as_str() {
                d.push(format!("方式 {m}"));
            }
            d.join("，")
        }).unwrap_or_default();
        out.push_str(&format!(
            "{}\t{}\t{}{}{}{}\n",
            fmt_time(a.at),
            a.actor_name,
            what,
            if a.target_name.is_empty() { String::new() } else { format!("\t{}", a.target_name) },
            if a.ip.is_empty() { String::new() } else { format!("\tIP {}", a.ip) },
            if detail.is_empty() { String::new() } else { format!("\t{detail}") }
        ));
    }
    out
}

/// 聊天式对话用的账号资源：设备、SSH 主机、片段、端口映射、连接记录；用到设备时再连接。
pub(crate) struct ChatWorkspace {
    inner: std::sync::Weak<Inner>,
    pub(crate) exec: Arc<galaxy_client::ai::SessionExecutor>,
}

impl ChatWorkspace {
    /// MCP 用（手机上没有 MCP 服务）
    #[cfg_attr(any(target_os = "ios", target_os = "android"), allow(dead_code))]
    pub(crate) fn new(inner: &Arc<Inner>, exec: Arc<galaxy_client::ai::SessionExecutor>) -> Arc<Self> {
        Arc::new(Self { inner: Arc::downgrade(inner), exec })
    }

    fn inner(&self) -> anyhow::Result<Arc<Inner>> {
        self.inner.upgrade().ok_or_else(|| anyhow::anyhow!("已退出登录"))
    }

    pub(crate) fn cached_models(&self) -> Option<galaxy_proto::api::AiModelsResp> {
        self.inner.upgrade().and_then(|i| i.models.lock().clone())
    }

    pub(crate) fn name_of(&self, id: &str) -> String {
        let Some(inner) = self.inner.upgrade() else { return id.to_string() };
        if let Some(d) = DeviceId::from_hex(id) {
            return inner.device_name(&d);
        }
        let v = inner.vault_snapshot.lock().clone();
        v.iter().find(|e| e.id == id).map(|e| match &e.data {
            VaultData::SshHost { label, host, username, .. } => if label.is_empty() { format!("{username}@{host}") } else { label.clone() },
            VaultData::Snippet { label, .. } | VaultData::PortForward { label, .. } => label.clone(),
            _ => id.to_string(),
        }).unwrap_or_else(|| id.to_string())
    }

    fn catalog_now(&self) -> anyhow::Result<Vec<ag_core::CatalogItem>> {
        let inner = self.inner()?;
        let me = inner.identity.device_id;
        let vault = inner.vault_snapshot.lock().clone();
        let folder_name = |fid: &Option<String>| fid.as_ref().and_then(|f| vault.iter().find(|e| &e.id == f)).and_then(|e| match &e.data { VaultData::Folder { name, .. } => Some(name.clone()), _ => None });
        let mut v = vec![];
        if inner.local_ai().is_some() {
            v.push(ag_core::CatalogItem {
                id: LOCAL_TARGET.into(),
                name: inner.local_name(),
                kind: ag_core::TargetKind::Device,
                platform: inner.meta.platform,
                online: true,
                detail: format!("用户正在用的这台电脑，{}", inner.meta.os_version),
            });
        }
        // 先复制一份再遍历：下面的 device_name 也要拿 devices 这把锁（拿着锁调它会死锁）
        let devices = inner.devices.lock().clone();
        for d in devices.iter() {
            if d.id == me || d.kind == galaxy_proto::DeviceKind::Mobile {
                continue;
            }
            let hex = d.id.to_hex();
            let meta = vault.iter().find(|e| matches!(&e.data, VaultData::DeviceMeta { device_id, .. } if *device_id == hex));
            let mut detail = vec![];
            if !d.os_version.is_empty() {
                detail.push(d.os_version.clone());
            }
            if !d.hostname.is_empty() {
                detail.push(format!("主机名 {}", d.hostname));
            }
            if let Some(m) = meta {
                if let Some(f) = folder_name(&m.folder_id) {
                    detail.push(format!("分组 {f}"));
                }
                if let VaultData::DeviceMeta { tags, note, .. } = &m.data {
                    if !tags.is_empty() {
                        detail.push(format!("标签 {}", tags.join("、")));
                    }
                    if !note.is_empty() {
                        detail.push(format!("备注 {note}"));
                    }
                }
            }
            if !d.last_ip.is_empty() {
                detail.push(format!("最近 IP {}", d.last_ip));
            }
            if !d.online && d.last_seen_at > 0 {
                detail.push(format!("最后在线 {}", fmt_time(d.last_seen_at)));
            }
            v.push(ag_core::CatalogItem { id: hex, name: inner.device_name(&d.id), kind: ag_core::TargetKind::Device, platform: d.platform, online: d.online, detail: detail.join("，") });
        }
        for e in &vault {
            if let VaultData::SshHost { label, host, port, username, tags, note, .. } = &e.data {
                let mut detail = vec![format!("{username}@{host}:{port}")];
                if let Some(f) = folder_name(&e.folder_id) {
                    detail.push(format!("分组 {f}"));
                }
                if !tags.is_empty() {
                    detail.push(format!("标签 {}", tags.join("、")));
                }
                if !note.is_empty() {
                    detail.push(format!("备注 {note}"));
                }
                let name = if label.is_empty() { format!("{username}@{host}") } else { label.clone() };
                v.push(ag_core::CatalogItem { id: e.id.clone(), name, kind: ag_core::TargetKind::SshHost, platform: galaxy_proto::Platform::Linux, online: true, detail: detail.join("，") });
            }
        }
        Ok(v)
    }

    pub(crate) fn snippets_text(&self, only: Option<&str>) -> String {
        let Some(inner) = self.inner.upgrade() else { return String::new() };
        let mut out = String::new();
        for e in inner.vault_snapshot.lock().iter() {
            if only.is_some_and(|o| o != e.id) {
                continue;
            }
            if let VaultData::Snippet { label, command, tags } = &e.data {
                out.push_str(&format!("## {label}{}\n```\n{command}\n```\n", if tags.is_empty() { String::new() } else { format!("（标签：{}）", tags.join("、")) }));
            }
        }
        out
    }

    pub(crate) fn forwards_text(&self, only: Option<&str>) -> String {
        let Some(inner) = self.inner.upgrade() else { return String::new() };
        let mut out = String::new();
        for e in inner.vault_snapshot.lock().clone().iter() {
            if only.is_some_and(|o| o != e.id) {
                continue;
            }
            if let VaultData::PortForward { label, via_device_id, via_ssh_host_id, local_port, remote_host, remote_port, auto_start, note } = &e.data {
                let via = via_device_id.as_deref().or(via_ssh_host_id.as_deref()).map(|x| self.name_of(x)).unwrap_or_default();
                out.push_str(&format!(
                    "{label}\t本机端口 {}\t经由 {via}\t到 {remote_host}:{remote_port}{}{}\n",
                    if *local_port == 0 { "自动".to_string() } else { local_port.to_string() },
                    if *auto_start { "\t自动启动" } else { "" },
                    if note.is_empty() { String::new() } else { format!("\t备注 {note}") }
                ));
            }
        }
        out
    }
}

use galaxy_agent::agent as ag_core;

impl ag_core::Workspace for ChatWorkspace {
    fn catalog(&self) -> ag_core::BoxFut<'_, anyhow::Result<Vec<ag_core::CatalogItem>>> {
        Box::pin(async move {
            // 设备列表可能过时：先刷新一次（失败就用缓存）
            if let Ok(inner) = self.inner() {
                let _ = inner.refresh_devices().await;
            }
            self.catalog_now()
        })
    }

    fn query(&self, kind: &str, limit: u32) -> ag_core::BoxFut<'_, anyhow::Result<String>> {
        let kind = kind.to_string();
        Box::pin(async move {
            match kind.as_str() {
                "snippets" => Ok(self.snippets_text(None)),
                "port_forwards" => Ok(self.forwards_text(None)),
                _ => {
                    let inner = self.inner()?;
                    let items = inner.activity(limit).await.map_err(|e| anyhow::anyhow!("{e}"))?;
                    Ok(activity_text(&items))
                }
            }
        })
    }

    fn connect(&self, target: &str) -> ag_core::BoxFut<'_, anyhow::Result<()>> {
        let target = target.to_string();
        Box::pin(async move {
            if self.exec.sessions.lock().contains_key(&target) {
                return Ok(());
            }
            let inner = self.inner()?;
            if target == LOCAL_TARGET {
                let l = inner.local_ai().ok_or_else(|| anyhow::anyhow!("这台设备不能在本机执行"))?;
                self.exec.add_local(target, l, inner.local_name());
                return Ok(());
            }
            if let Some(d) = DeviceId::from_hex(&target) {
                let s = inner.session_for(d).await.map_err(|e| anyhow::anyhow!("{e}"))?;
                self.exec.add(target.clone(), s, inner.device_name(&d));
                return Ok(());
            }
            // AI 对话里没法弹窗问密码：用加密库里保存的凭据与已确认的主机密钥
            let (s, info) = inner.clone().connect_ssh_inner(target.clone(), Arc::new(NoPrompt), 0).await.map_err(|e| match e {
                GalaxyError::Cancelled | GalaxyError::Permission { .. } => anyhow::anyhow!("这台 SSH 主机需要输入密码或确认主机密钥，请先在主机列表里连接一次并保存密码"),
                other => anyhow::anyhow!("{other}"),
            })?;
            let name = if info.label.is_empty() { format!("{}@{}", info.username, info.host) } else { info.label.clone() };
            self.exec.add_ssh(target, s, name).await;
            Ok(())
        })
    }

    fn wake(&self, target: &str) -> ag_core::BoxFut<'_, anyhow::Result<String>> {
        let target = target.to_string();
        Box::pin(async move {
            let inner = self.inner()?;
            let d = match DeviceId::from_hex(&target) {
                Some(d) => d,
                None => {
                    let id = self.catalog_now()?.into_iter().find(|c| c.name == target || c.name.eq_ignore_ascii_case(&target)).map(|c| c.id).unwrap_or_default();
                    DeviceId::from_hex(&id).ok_or_else(|| anyhow::anyhow!("找不到设备「{target}」：只能唤醒账号里的设备，先用 list_devices 查 id"))?
                }
            };
            let c = inner.online.lock().as_ref().map(|o| o.connector.clone()).ok_or_else(|| anyhow::anyhow!("尚未连上服务器"))?;
            let list = inner.devices.lock().clone();
            let via = galaxy_client::wake::wake(&c, &list, inner.identity.device_id, d).await.map_err(|e| anyhow::anyhow!("{e}"))?;
            let via: Vec<String> = via.into_iter().map(|v| if v.is_empty() { "本机".to_string() } else { v }).collect();
            Ok(format!("已经由 {} 发出唤醒包；设备开机并连上服务器一般要几十秒到几分钟，之后用 list_devices 看是否在线", via.join("、")))
        })
    }

    fn fetch(&self, url: &str, max_bytes: usize) -> ag_core::BoxFut<'_, anyhow::Result<(String, String, Vec<u8>)>> {
        let url = url.to_string();
        Box::pin(async move {
            use futures::StreamExt;
            let client = reqwest::Client::builder()
                .user_agent(format!("LemonGalaxy/{} (AI web_fetch)", env!("CARGO_PKG_VERSION")))
                .timeout(std::time::Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::limited(5))
                .build()?;
            let resp = client.get(&url).header("accept", "text/html,text/plain,application/json,*/*;q=0.5").send().await?;
            let status = resp.status();
            let final_url = resp.url().to_string();
            let ctype = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            if !status.is_success() {
                anyhow::bail!("HTTP {status}");
            }
            if ctype.starts_with("image/") || ctype.starts_with("video/") || ctype.starts_with("audio/") || ctype.contains("octet-stream") || ctype.contains("zip") {
                anyhow::bail!("不是文本内容（{ctype}）");
            }
            let mut body = Vec::new();
            let mut st = resp.bytes_stream();
            while let Some(c) = st.next().await {
                let c = c?;
                body.extend_from_slice(&c);
                if body.len() >= max_bytes {
                    body.truncate(max_bytes);
                    break;
                }
            }
            Ok((final_url, ctype, body))
        })
    }
}

/// 不能询问用户时用：一律取消。
struct NoPrompt;
impl SecretPrompt for NoPrompt {
    fn ask(&self, _prompt: String, _echo: bool) -> Option<String> {
        None
    }
    fn confirm_host_key(&self, _host: String, _port: u16, _key_type: String, _fingerprint: String, _changed: bool) -> bool {
        false
    }
}
