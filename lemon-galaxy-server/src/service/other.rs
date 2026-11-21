//! 其他系统（macOS 等）：服务器只用于试用，不安装系统服务。

use super::SvcConfig;
use std::path::PathBuf;

pub const SERVICE_NAME: &str = "lemon-galaxy-server";

const UNSUPPORTED: &str = "这个系统上的服务器只用于试用，不支持安装为系统服务；正式部署请用 Linux 或 Windows 版";

pub fn require_admin() -> anyhow::Result<()> {
    anyhow::bail!(UNSUPPORTED)
}

pub fn default_data_dir() -> Option<PathBuf> {
    None
}

pub fn config_path() -> Option<PathBuf> {
    None
}

pub fn installed_exe() -> PathBuf {
    std::env::current_exe().unwrap_or_default()
}

pub fn secure_dir(_dir: &std::path::Path) {}

pub fn install(_cfg: &SvcConfig) -> anyhow::Result<Result<bool, String>> {
    anyhow::bail!(UNSUPPORTED)
}

pub fn uninstall() -> anyhow::Result<bool> {
    anyhow::bail!(UNSUPPORTED)
}

pub fn state() -> Option<String> {
    None
}

pub fn is_stopped() -> bool {
    true
}

pub fn update_target() -> Option<PathBuf> {
    None
}

pub fn restart_if_installed() -> anyhow::Result<bool> {
    Ok(false)
}
