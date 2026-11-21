//! 纯被控端（worker，如 macOS 被控端）：可以同时登录多个账号，出现在每个账号各自的设备列表里。
//! 与 control 客户端里的本机被控（单账号，跟随主控登录）不同，这里自己管登录。

use crate::callbacks::SecureStore;
use crate::client::StoreBridge;
use crate::host::HostService;
use crate::{config, locale, run, GalaxyError, Result};
use galaxy_client::account::{self, DeviceMeta, LoginOutcome};
use galaxy_client::api::Api;
use galaxy_client::identity::Identity;
use galaxy_host::accounts::HostState;
use galaxy_net::ServerAddr;
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum WorkerLoginResult {
    Success { user_id: u64, username: String },
    /// 需要两步验证码：带上 totp 再调一次
    NeedTotp,
    /// 首次登录或被管理员重置：带上 new_password 再调一次
    MustChangePassword,
}

#[derive(uniffi::Object)]
pub struct WorkerHost {
    identity: Identity,
    dir: PathBuf,
    host: Mutex<Option<Arc<HostService>>>,
    starting: tokio::sync::Mutex<()>,
}

fn meta() -> DeviceMeta {
    let c = config();
    DeviceMeta { kind: galaxy_proto::DeviceKind::Worker, platform: c.platform.into(), os_version: c.os_version, arch: std::env::consts::ARCH.into(), hostname: c.hostname, app_version: c.app_version }
}

impl WorkerHost {
    fn state_path(&self) -> PathBuf {
        self.dir.join("host.json")
    }

    async fn ensure_started(&self) -> Result<Option<Arc<HostService>>> {
        let _g = self.starting.lock().await;
        if let Some(h) = self.host.lock().clone() {
            return Ok(Some(h));
        }
        let st = HostState::load(&self.state_path());
        if st.server.is_empty() {
            return Ok(None);
        }
        let server = ServerAddr::parse(&st.server).map_err(|e| GalaxyError::Other { detail: e })?;
        let cfg = galaxy_host::HostConfig { server, data_dir: self.dir.clone(), meta: meta(), multi_account: true, file_helper: None };
        let h = galaxy_host::HostService::start(cfg, self.identity.clone()).await.map_err(|e| GalaxyError::Other { detail: format!("被控服务启动失败：{e}") })?;
        let w = HostService::wrap(h);
        *self.host.lock() = Some(w.clone());
        Ok(Some(w))
    }
}

#[uniffi::export]
impl WorkerHost {
    /// 数据放在 CoreConfig.data_dir/worker；设备身份存在 `store` 里（与主控端分开）。
    #[uniffi::constructor]
    pub fn new(store: Arc<dyn SecureStore>) -> Arc<Self> {
        let bridge: Arc<dyn galaxy_client::identity::SecureStore> = Arc::new(StoreBridge(store));
        let dir = PathBuf::from(config().data_dir).join("worker");
        let _ = std::fs::create_dir_all(&dir);
        Arc::new(Self { identity: Identity::load_or_create(&bridge), dir, host: Mutex::new(None), starting: tokio::sync::Mutex::new(()) })
    }

    pub fn device_id(&self) -> String {
        self.identity.device_id.to_hex()
    }

    /// 已登录账号所在的服务器地址；还没登录过为空。
    pub fn server(&self) -> Option<String> {
        Some(HostState::load(&self.state_path()).server).filter(|s| !s.is_empty())
    }

    /// 启动被控服务（有账号时）。应用启动时调一次；之后的状态从返回的 HostService 读。
    pub async fn start(self: Arc<Self>) -> Result<Option<Arc<HostService>>> {
        run(async move { self.ensure_started().await }).await
    }

    pub fn service(&self) -> Option<Arc<HostService>> {
        self.host.lock().clone()
    }

    /// 登录一个账号。`system_user` 为空时用当前系统用户。所有账号必须在同一台服务器上。
    #[allow(clippy::too_many_arguments)]
    pub async fn login(self: Arc<Self>, server: String, username: String, password: String, totp: Option<String>, new_password: Option<String>, system_user: Option<String>) -> Result<WorkerLoginResult> {
        run(async move {
            let addr = ServerAddr::parse(&server).map_err(|e| GalaxyError::Auth { code: "bad_server".into(), detail: e })?;
            let st = HostState::load(&self.state_path());
            if !st.server.is_empty() && st.server != addr.to_string_full() && !st.accounts.is_empty() {
                return Err(GalaxyError::Other { detail: "已登录的账号在另一台服务器上，更换服务器前请先退出这些账号".into() });
            }
            let api = Api::new(addr.clone(), &locale()).map_err(|e| GalaxyError::Network { detail: e.to_string() })?;
            let m = meta();
            let logged = match account::login(&api, &self.identity, &m, &username, &password, totp.as_deref().filter(|t| !t.is_empty())).await? {
                LoginOutcome::Done(l) => l,
                LoginOutcome::NeedTotp => return Ok(WorkerLoginResult::NeedTotp),
                LoginOutcome::MustChangePassword(fl) => match new_password.filter(|p| !p.is_empty()) {
                    Some(np) => account::complete_first_login(&api, &self.identity, &m, fl, &np).await?,
                    None => return Ok(WorkerLoginResult::MustChangePassword),
                },
            };
            let sys = system_user.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| galaxy_host::sysuser::SysUser::current().name);
            if galaxy_host::sysuser::SysUser::lookup(&sys).is_none() {
                return Err(GalaxyError::NotFound { detail: format!("系统用户不存在：{sys}") });
            }
            let policy = galaxy_host::HostPolicy::default_for(&sys);
            if self.host.lock().is_none() {
                let mut st = HostState::load(&self.state_path());
                st.server = addr.to_string_full();
                if let Ok(info) = api.get::<galaxy_proto::api::ServerInfoResp>("/info").await {
                    st.server_sign_pubkey = info.server_sign_pubkey;
                }
                st.save(&self.state_path()).map_err(|e| GalaxyError::Other { detail: e.to_string() })?;
            }
            let h = self.ensure_started().await?.ok_or_else(|| GalaxyError::Other { detail: "被控服务没有启动".into() })?;
            h.inner.add_account(&logged, policy).map_err(|e| GalaxyError::Other { detail: e.to_string() })?;
            Ok(WorkerLoginResult::Success { user_id: logged.user.id, username: logged.user.username })
        })
        .await
    }

    /// 退出某个账号（服务器上也作废这台设备的登录）。
    pub async fn logout(self: Arc<Self>, user_id: u64) -> Result<()> {
        run(async move {
            let running = self.host.lock().clone();
            if let Some(h) = running {
                h.inner.remove_account(user_id).await;
            } else {
                let mut st = HostState::load(&self.state_path());
                st.accounts.retain(|a| a.user_id != user_id);
                let _ = st.save(&self.state_path());
            }
            Ok(())
        })
        .await
    }
}
