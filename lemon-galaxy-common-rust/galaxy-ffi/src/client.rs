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

struct StoreBridge(Arc<dyn SecureStore>);
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

struct Inner {
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
}

#[derive(uniffi::Object)]
pub struct GalaxyClient {
    inner: Arc<Inner>,
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

    fn api(&self) -> Result<Api> {
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
            if me.meta.kind == galaxy_proto::DeviceKind::Control && me.host.lock().is_none() {
                let cfg = galaxy_host::HostConfig {
                    server: server.clone(),
                    data_dir: std::path::PathBuf::from(config().data_dir).join("host"),
                    meta: me.meta.clone(),
                    multi_account: false,
                    file_helper: None,
                };
                match galaxy_host::HostService::start(cfg, me.identity.clone()).await {
                    Ok(h) => {
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
            })
            .collect()
    }

    pub async fn refresh_devices(&self) -> Result<()> {
        let inner = self.inner.clone();
        run(async move { inner.refresh_devices().await }).await
    }

    pub async fn remove_device(&self, device_id: String) -> Result<()> {
        let inner = self.inner.clone();
        run(async move {
            let _: serde_json::Value = inner.api()?.delete(&format!("/devices/{device_id}")).await?;
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
    let cached = inner.models.lock().clone();
    let models = match cached {
        Some(m) => m,
        None => api.get::<galaxy_proto::api::AiModelsResp>("/ai/models").await?,
    };
    if !models.enabled {
        return Err(GalaxyError::Permission { detail: "未开通 AI 功能".into() });
    }
    let m = models.items.iter().find(|m| m.provider_id == c.provider_id && m.id == c.model_id).cloned().ok_or_else(|| GalaxyError::Permission { detail: "未开通该模型".into() })?;
    let exec = galaxy_client::ai::SessionExecutor::new();
    let mut names = vec![];
    for t in &c.targets {
        if t.kind == AgentTargetKind::SshHost {
            // AI 对话里没法弹窗问密码：用加密库里保存的凭据与已确认的主机密钥
            let (s, info) = inner.clone().connect_ssh_inner(t.id.clone(), Arc::new(NoPrompt), 0).await.map_err(|e| match e {
                GalaxyError::Cancelled | GalaxyError::Permission { .. } => other("这台 SSH 主机需要输入密码或确认主机密钥，请先在主机列表里连接一次并保存密码"),
                other => other,
            })?;
            let name = if info.label.is_empty() { format!("{}@{}", info.username, info.host) } else { info.label.clone() };
            names.push(name.clone());
            exec.add_ssh(t.id.clone(), s, name).await;
            continue;
        }
        let d = parse_dev(&t.id)?;
        let s = inner.session_for(d).await?;
        let name = inner.device_name(&d);
        names.push(name.clone());
        exec.add(t.id.clone(), s, name);
    }
    if names.is_empty() {
        return Err(other("请至少选择一台设备"));
    }
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
        max_steps: 60,
        locale: crate::locale(),
        trust_minutes: c.trust_minutes,
    };
    let title = c.title.clone().or_else(|| restore.as_ref().map(|r| r.title.clone())).unwrap_or_default();
    let conv = AgentConversation::build(cfg, Arc::new(galaxy_client::ai::GatewayLlm { api }), exec, listener, inner.conv_key(), title, names, c.clone());
    if let Some(r) = restore {
        conv.agent.restore(r.messages);
    }
    Ok(conv)
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
