//! 本机被控（control 客户端）。

use crate::callbacks::*;
use crate::types::*;
use crate::{rt, run, Result};
use galaxy_host::platform::{self as hp, FrameSink};
use galaxy_proto::peer as pp;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::oneshot;

/// 平台层编码后把帧推给内核。
#[derive(uniffi::Object)]
pub struct EncodedFrameSink {
    inner: Arc<dyn FrameSink>,
}

#[uniffi::export]
impl EncodedFrameSink {
    pub fn push(&self, data: Vec<u8>, pts_us: u64, keyframe: bool) {
        self.inner.push(data, pts_us, keyframe);
    }
    pub fn set_resolution(&self, width: u32, height: u32) {
        self.inner.resized(width, height);
    }
}

/// 把原生平台层适配成内核的 HostPlatform。
struct PlatformBridge(Arc<dyn HostPlatform>);

impl hp::HostPlatform for PlatformBridge {
    fn list_displays(&self) -> Vec<pp::DisplayInfo> {
        self.0.list_displays().into_iter().map(Into::into).collect()
    }
    fn start_capture(&self, display: u32, max_fps: u32, bitrate_kbps: u32, codecs: Vec<pp::VideoCodec>, sink: Arc<dyn FrameSink>) -> std::result::Result<hp::CaptureStarted, String> {
        let s = Arc::new(EncodedFrameSink { inner: sink });
        match self.0.start_capture(display, CaptureOptions { max_fps, bitrate_kbps, codecs: codecs.into_iter().map(Into::into).collect() }, s) {
            Some(c) => Ok(hp::CaptureStarted { width: c.width, height: c.height, codec: c.codec.into() }),
            None => Err("无法开始截屏（请确认已授权「屏幕录制」）".into()),
        }
    }
    fn stop_capture(&self, display: u32) {
        self.0.stop_capture(display)
    }
    fn request_keyframe(&self, display: u32) {
        self.0.request_keyframe(display)
    }
    fn set_bitrate(&self, display: u32, kbps: u32) {
        self.0.set_bitrate(display, kbps)
    }
    fn inject(&self, display: u32, ev: pp::InputEvent) {
        self.0.inject(display, ev.into())
    }
    fn screenshot(&self, display: u32, max_w: u32, max_h: u32) -> Option<pp::ImageData> {
        self.0.screenshot(display, max_w, max_h).map(Into::into)
    }
    fn ui_tree(&self, display: u32, max_nodes: u32) -> Option<String> {
        self.0.ui_tree(display, max_nodes)
    }
    fn clipboard_get(&self) -> Option<pp::ClipboardData> {
        self.0.clipboard_get().map(Into::into)
    }
    fn clipboard_set(&self, d: pp::ClipboardData) {
        self.0.clipboard_set(d.into())
    }
    fn capabilities(&self) -> Vec<String> {
        self.0.capabilities()
    }
}

struct EventsBridge {
    listener: Arc<dyn HostListener>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>,
}

impl galaxy_host::HostEvents for EventsBridge {
    fn on_status(&self, s: &galaxy_host::HostStatus) {
        self.listener.on_status(convert_status(s));
    }
    fn ask_assist(&self, from_user: String, from_device: String) -> futures::future::BoxFuture<'static, bool> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(id.clone(), tx);
        self.listener.on_assist_request(id, from_user, from_device);
        Box::pin(async move { rx.await.unwrap_or(false) })
    }
    fn on_session_started(&self, s: &galaxy_host::service::SessionInfo) {
        self.listener.on_session_started(s.clone().into());
    }
    fn on_session_ended(&self, id: &str) {
        self.listener.on_session_ended(id.to_string());
    }
    fn on_ai_activity(&self, session: &str, desc: &str) {
        self.listener.on_ai_activity(session.to_string(), desc.to_string());
    }
}

fn convert_status(s: &galaxy_host::HostStatus) -> HostStatus {
    HostStatus {
        online: s.online,
        remote_enabled: s.remote_enabled,
        device_id: s.device_id.clone(),
        public_ip: s.public_ip.clone(),
        accounts: vec![],
        sessions: s.sessions.iter().cloned().map(Into::into).collect(),
        assist: s.assist.clone().map(|(d, c)| AssistInfo { device_code: d, code: c }),
    }
}

#[derive(uniffi::Object)]
pub struct HostService {
    pub(crate) inner: Arc<galaxy_host::HostService>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>,
}

impl HostService {
    pub(crate) fn wrap(inner: Arc<galaxy_host::HostService>) -> Arc<Self> {
        Arc::new(Self { inner, pending: Default::default() })
    }
}

#[uniffi::export]
impl HostService {
    pub fn status(&self) -> HostStatus {
        let mut st = convert_status(&self.inner.status());
        st.accounts = self.accounts();
        st
    }

    pub fn set_platform(&self, platform: Arc<dyn HostPlatform>) {
        self.inner.set_platform(Arc::new(PlatformBridge(platform)));
    }

    pub fn set_listener(&self, listener: Arc<dyn HostListener>) {
        self.inner.set_events(Arc::new(EventsBridge { listener, pending: self.pending.clone() }));
    }

    pub fn accounts(&self) -> Vec<HostAccount> {
        self.inner
            .state()
            .accounts
            .into_iter()
            .map(|a| HostAccount {
                user_id: a.user_id,
                username: a.username,
                display_name: a.display_name,
                status: match a.status {
                    galaxy_host::AccountStatus::Active => AccountStatus::Active,
                    galaxy_host::AccountStatus::NeedsRelogin => AccountStatus::NeedsRelogin,
                },
                policy: a.policy.into(),
            })
            .collect()
    }

    /// 修改某账号在本机的权限（只能在本机修改）。
    pub fn update_account_policy(&self, user_id: u64, policy: HostPolicy) {
        self.inner.update_policy(user_id, policy.into());
    }

    pub fn set_remote_enabled(&self, enabled: bool) {
        self.inner.set_remote_enabled(enabled);
    }

    pub fn disconnect_all(&self) {
        self.inner.disconnect_all();
    }

    pub fn stop_ai(&self) {
        self.inner.stop_ai();
    }

    /// 开启临时协助，等到服务器分配设备码后返回。
    pub async fn start_assist(&self) -> Result<AssistInfo> {
        let h = self.inner.clone();
        run(async move {
            let code = h.start_assist();
            for _ in 0..100 {
                if let Some((d, c)) = h.status().assist {
                    return Ok(AssistInfo { device_code: d, code: c });
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            let _ = code;
            Err(crate::GalaxyError::Network { detail: "连不上服务器，无法开启临时协助".into() })
        })
        .await
    }

    pub fn stop_assist(&self) {
        let h = self.inner.clone();
        rt().spawn(async move { h.stop_assist() });
    }

    /// 自定义验证码（8 到 16 位字母数字），None 为每次随机。
    pub fn set_assist_code(&self, code: Option<String>) {
        self.inner.set_assist_code(code);
    }

    pub fn respond_assist(&self, request_id: String, allow: bool) {
        if let Some(tx) = self.pending.lock().remove(&request_id) {
            let _ = tx.send(allow);
        }
    }

    pub fn recent_events(&self, limit: u32) -> Vec<HostEvent> {
        self.inner.recent_events(limit as usize).into_iter().map(Into::into).collect()
    }
}
