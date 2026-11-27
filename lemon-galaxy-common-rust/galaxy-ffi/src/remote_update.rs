//! 远程升级（docs/ffi.md 5.0）：
//! - 被控一侧：App 用 `set_update_request_handler` 登记一次回调；内核起的每个本机被控服务都装上「执行更新」入口
//!   （galaxy_host::update::RemoteUpdater），主控端请求时先查新版本，有就调回调，由 App 走自己的零交互升级。
//!   没登记回调的 App（比如测试）不报 remote_update 能力。
//! - 主控一侧：`Client::upgrade_device`（client.rs）发请求后等对方以新版本重新上线。

use crate::callbacks::UpdateRequestHandler;
use galaxy_client::api::Api;
use galaxy_host::AccountStatus;
use parking_lot::Mutex;
use std::sync::{Arc, Weak};

static HANDLER: Mutex<Option<Arc<dyn UpdateRequestHandler>>> = Mutex::new(None);
static HOSTS: Mutex<Vec<Weak<galaxy_host::HostService>>> = Mutex::new(Vec::new());

/// 登记远程升级回调（App 启动时调一次；之后起的和已经在跑的本机被控服务都会用它）。
#[uniffi::export]
pub fn set_update_request_handler(handler: Arc<dyn UpdateRequestHandler>) {
    *HANDLER.lock() = Some(handler);
    let mut hosts = HOSTS.lock();
    hosts.retain(|w| w.strong_count() > 0);
    for h in hosts.iter().filter_map(|w| w.upgrade()) {
        install(&h);
    }
}

/// 内核起了本机被控服务：记下来，已登记回调就装上「执行更新」入口
pub(crate) fn register_host(h: &Arc<galaxy_host::HostService>) {
    {
        let mut hosts = HOSTS.lock();
        hosts.retain(|w| w.strong_count() > 0);
        hosts.push(Arc::downgrade(h));
    }
    if HANDLER.lock().is_some() {
        install(h);
    }
}

fn install(h: &Arc<galaxy_host::HostService>) {
    h.set_updater(Arc::new(FfiUpdater { host: Arc::downgrade(h) }));
}

struct FfiUpdater {
    host: Weak<galaxy_host::HostService>,
}

impl galaxy_host::update::RemoteUpdater for FfiUpdater {
    fn start(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<String>, String>> + Send + '_>> {
        Box::pin(async move {
            let h = self.host.upgrade().ok_or("被控服务已停止")?;
            let handler = HANDLER.lock().clone().ok_or("这个应用不支持远程升级")?;
            // 用本机任一有效账号的令牌向自己的服务器查询（与本机「检查更新」同一个服务器）
            let st = h.state();
            let acct = st.accounts.iter().find(|a| a.status == AccountStatus::Active).ok_or("没有可用的账号")?;
            let api = Api::new(h.cfg.server.clone(), &crate::locale()).map_err(|e| e.to_string())?;
            api.set_token(Some(acct.token.clone()));
            let channel = galaxy_host::update::update_channel(h.cfg.meta.kind, h.cfg.meta.platform);
            let parts: Vec<&str> = channel.split('/').collect();
            let (component, platform) = (parts.first().copied().unwrap_or("worker"), parts.get(1).copied().unwrap_or(""));
            let arch = match parts.get(2).copied() {
                Some("universal") | None => "",
                Some(a) => a,
            };
            match galaxy_client::update::check(&api, component, platform, arch, &h.cfg.meta.app_version).await.map_err(|e| e.to_string())? {
                Some(info) => {
                    let v = info.version.clone();
                    handler.on_update_requested(info.into());
                    Ok(Some(v))
                }
                None => Ok(None),
            }
        })
    }
}
