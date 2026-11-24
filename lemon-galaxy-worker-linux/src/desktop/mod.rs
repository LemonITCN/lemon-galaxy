//! 远程桌面：画面、键鼠、剪贴板、声音、虚拟桌面。设计见 docs/linux-desktop.md。
//!
//! 只在 Linux 且开了 `desktop` 特性时编进来；其他情况（musl 精简版、在 macOS 上编来做端到端测试）没有画面能力。

use galaxy_host::platform::HostPlatform;
use std::path::Path;
use std::sync::Arc;

#[cfg(all(feature = "desktop", target_os = "linux"))]
mod imp;

/// 服务用的平台层（全进程一份：主服务与临时协助共用同一个画面代理）。
pub fn host_platform() -> Option<Arc<dyn HostPlatform>> {
    #[cfg(all(feature = "desktop", target_os = "linux"))]
    {
        Some(imp::platform())
    }
    #[cfg(not(all(feature = "desktop", target_os = "linux")))]
    {
        None
    }
}

/// 内部命令（画面代理）与自检、登录界面等命令；不认识返回 None。
pub fn extra_command(cmd: &str, rest: &[String], dir: &Path) -> Option<i32> {
    #[cfg(all(feature = "desktop", target_os = "linux"))]
    {
        imp::extra_command(cmd, rest, dir)
    }
    #[cfg(not(all(feature = "desktop", target_os = "linux")))]
    {
        let _ = (cmd, rest, dir);
        None
    }
}

pub fn extra_help() -> Vec<(&'static str, &'static str, &'static str)> {
    #[cfg(all(feature = "desktop", target_os = "linux"))]
    {
        imp::extra_help()
    }
    #[cfg(not(all(feature = "desktop", target_os = "linux")))]
    {
        vec![]
    }
}
