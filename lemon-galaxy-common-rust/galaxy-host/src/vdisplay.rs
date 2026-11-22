//! 虚拟屏的归属与生命周期（平台层只管加删，见 platform.rs）。
//!
//! - 主控用 `VirtualDisplayAdd` 加的屏属于那个会话，会话结束时删掉；只能删自己加的。
//! - 无头兜底屏：本机没有亮着的显示器（笔记本合盖、没接显示器）时，有会话打开屏幕列表就自动加一块
//!   （模式见 host.json 的 headless_display），不算进主控的上限；最后一个会话结束 10 秒后仍没有新会话就删掉。
//! - 平台层的虚拟屏可能自己消失（Windows 画面代理随控制台会话切换重启时，驱动随句柄删屏），每次操作前先对账。

use crate::service::HostService;
use galaxy_proto::peer::{DisplayInfo, PeerError};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    Session(String),
    Headless,
}

#[derive(Default)]
pub(crate) struct VirtualDisplays {
    owners: Mutex<HashMap<u32, Owner>>,
    /// 加删串行进行（两个会话同时打开屏幕列表时只加一块无头屏）
    op: tokio::sync::Mutex<()>,
}

impl HostService {
    /// 和平台层对账：已经不存在的虚拟屏从归属表里去掉。
    async fn vd_reconcile(&self) {
        let platform = self.platform.read().clone();
        let live = tokio::task::spawn_blocking(move || platform.virtual_display_ids()).await.unwrap_or_default();
        self.vdisplays.owners.lock().retain(|id, _| live.contains(id));
    }

    /// 主控打开屏幕列表前：本机没有亮着的显示器就加一块无头兜底屏。
    pub(crate) async fn ensure_headless_display(&self) {
        let Some((w, h, hz)) = self.headless_display() else { return };
        let platform = self.platform.read().clone();
        if platform.virtual_display_max() == 0 {
            return;
        }
        let _g = self.vdisplays.op.lock().await;
        self.vd_reconcile().await;
        if self.vdisplays.owners.lock().values().any(|o| *o == Owner::Headless) {
            return;
        }
        let r = tokio::task::spawn_blocking(move || if platform.needs_headless_display() { Some(platform.add_virtual_display(w, h, hz)) } else { None }).await.ok().flatten();
        match r {
            Some(Ok(d)) => {
                tracing::info!("本机没有亮着的显示器，已加无头兜底屏 {}（{}x{}@{hz}）", d.name, d.width, d.height);
                self.vdisplays.owners.lock().insert(d.id, Owner::Headless);
            }
            Some(Err(e)) => tracing::warn!("加无头兜底屏失败：{e}"),
            None => {}
        }
    }

    /// 主控请求加虚拟屏。
    pub(crate) async fn add_virtual_display(&self, session: &str, width: u32, height: u32, refresh: u32) -> Result<DisplayInfo, PeerError> {
        let refresh = if refresh == 0 { 60 } else { refresh };
        if !galaxy_proto::peer::virtual_display_mode_ok(width, height, refresh) {
            return Err(PeerError::new("bad_request", format!("不支持的分辨率 {width}x{height}@{refresh}")));
        }
        let platform = self.platform.read().clone();
        let max = platform.virtual_display_max() as usize;
        if max == 0 {
            return Err(PeerError::new("unsupported", "被控端不支持虚拟屏（没有装虚拟屏驱动或系统不支持）"));
        }
        let _g = self.vdisplays.op.lock().await;
        self.vd_reconcile().await;
        let used = self.vdisplays.owners.lock().values().filter(|o| matches!(o, Owner::Session(_))).count();
        if used >= max {
            return Err(PeerError::new("limit", format!("虚拟屏最多 {max} 块，已经用完")));
        }
        match tokio::task::spawn_blocking(move || platform.add_virtual_display(width, height, refresh)).await {
            Ok(Ok(d)) => {
                self.vdisplays.owners.lock().insert(d.id, Owner::Session(session.to_string()));
                Ok(d)
            }
            Ok(Err(e)) => Err(PeerError::new("virtual_display", e)),
            Err(_) => Err(PeerError::new("virtual_display", "加虚拟屏时出错")),
        }
    }

    /// 主控请求删虚拟屏（只能删本会话加的）。
    pub(crate) async fn remove_virtual_display(&self, session: &str, id: u32) -> Result<(), PeerError> {
        let _g = self.vdisplays.op.lock().await;
        self.vd_reconcile().await;
        match self.vdisplays.owners.lock().get(&id) {
            Some(Owner::Session(s)) if s == session => {}
            Some(_) => return Err(PeerError::denied("这块虚拟屏不是本会话加的")),
            None => return Err(PeerError::new("not_found", "没有这块虚拟屏")),
        }
        let platform = self.platform.read().clone();
        let r = tokio::task::spawn_blocking(move || platform.remove_virtual_display(id)).await.unwrap_or_else(|_| Err("删虚拟屏时出错".into()));
        self.vdisplays.owners.lock().remove(&id);
        r.map_err(|e| PeerError::new("virtual_display", e))
    }

    /// 会话结束：删掉它加的虚拟屏；没有会话了，10 秒后删无头兜底屏（给断线重连留时间）。
    pub(crate) async fn virtual_displays_session_ended(self: &Arc<Self>, session: &str) {
        let mine: Vec<u32> = self.vdisplays.owners.lock().iter().filter(|(_, o)| **o == Owner::Session(session.to_string())).map(|(id, _)| *id).collect();
        if !mine.is_empty() {
            let _g = self.vdisplays.op.lock().await;
            let platform = self.platform.read().clone();
            for id in mine {
                self.vdisplays.owners.lock().remove(&id);
                let p = platform.clone();
                match tokio::task::spawn_blocking(move || p.remove_virtual_display(id)).await {
                    Ok(Ok(())) => tracing::info!("会话结束，已删掉它加的虚拟屏 {id}"),
                    Ok(Err(e)) => tracing::warn!("会话结束删虚拟屏 {id} 失败：{e}"),
                    Err(_) => {}
                }
            }
        }
        if self.session_count() > 0 || !self.vdisplays.owners.lock().values().any(|o| *o == Owner::Headless) {
            return;
        }
        let me = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            if me.session_count() > 0 {
                return;
            }
            let _g = me.vdisplays.op.lock().await;
            let ids: Vec<u32> = me.vdisplays.owners.lock().iter().filter(|(_, o)| **o == Owner::Headless).map(|(id, _)| *id).collect();
            let platform = me.platform.read().clone();
            for id in ids {
                me.vdisplays.owners.lock().remove(&id);
                let p = platform.clone();
                if let Ok(Err(e)) = tokio::task::spawn_blocking(move || p.remove_virtual_display(id)).await {
                    tracing::warn!("删无头兜底屏 {id} 失败：{e}");
                } else {
                    tracing::info!("远程会话都结束了，已删掉无头兜底屏 {id}");
                }
            }
        });
    }
}
