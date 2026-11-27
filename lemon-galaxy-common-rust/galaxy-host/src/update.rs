//! 远程升级被控端（`MgmtReq::ApplyUpdate`）：主控端在设备详情里点「升级被控端」，被控端自己走完整个升级。
//!
//! 具体怎么升级由运行被控内核的程序提供（`HostService::set_updater`）：Windows / Linux 的被控服务换程序后由服务管理器拉起，
//! Mac 应用交给应用自己的升级流程。没装的程序（比如测试用的被控端）不报 "remote_update" 能力。

use galaxy_proto::{DeviceKind, Platform};
use std::future::Future;
use std::pin::Pin;

/// 执行远程升级。`start` 先查有没有新版本：有就把升级放到后台开始做并立即返回要升级到的版本（之后进程会退出、
/// 以新版本重新上线），没有返回 None，出错返回原因。必须全程不需要人操作。
pub trait RemoteUpdater: Send + Sync {
    fn start(&self) -> Pin<Box<dyn Future<Output = Result<Option<String>, String>> + Send + '_>>;
}

/// 本程序的更新通道「组件/平台/架构」，与发布文件的分类（galaxy_proto::update::classify_asset）一致。
/// Windows 兼容版（win7 目标）、XP 版（构建时加 --cfg lemon_galaxy_xp）、Linux musl 版各算一个平台。
pub fn update_channel(kind: DeviceKind, platform: Platform) -> String {
    let component = match kind {
        DeviceKind::Control => "control",
        DeviceKind::Mobile => "mobile",
        _ => "worker",
    };
    let (plat, arch): (&str, String) = match platform {
        Platform::Macos => ("macos", "universal".into()),
        Platform::Android => ("android", "universal".into()),
        Platform::Ios => ("ios", "universal".into()),
        Platform::Windows if cfg!(lemon_galaxy_xp) => ("windowsxp", galaxy_proto::update::this_arch().into()),
        Platform::Windows if cfg!(target_vendor = "win7") => ("windows7", galaxy_proto::update::this_arch().into()),
        Platform::Windows => ("windows", galaxy_proto::update::this_arch().into()),
        _ if cfg!(target_env = "musl") => ("linux-musl", galaxy_proto::update::this_arch().into()),
        _ => ("linux", galaxy_proto::update::this_arch().into()),
    };
    format!("{component}/{plat}/{arch}")
}
