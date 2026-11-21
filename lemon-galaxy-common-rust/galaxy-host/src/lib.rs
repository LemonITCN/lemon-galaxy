//! LemonGalaxy 被控端内核。

pub mod accounts;
pub mod audio;
pub mod exec;
pub mod files;
pub mod fio;
pub mod platform;
pub mod rate;
pub mod serve;
pub mod service;
pub mod sysuser;
pub mod term;
#[cfg(windows)]
pub mod win;

pub use accounts::{AccountStatus, HostAccount, HostPolicy};
pub use service::{HostConfig, HostEvents, HostService, HostStatus};

/// 本机主机名。
pub fn hostname() -> String {
    #[cfg(unix)]
    {
        nix::unistd::gethostname().ok().map(|h| h.to_string_lossy().to_string()).unwrap_or_else(|| "localhost".into())
    }
    #[cfg(not(unix))]
    {
        std::env::var("COMPUTERNAME").unwrap_or_else(|_| "localhost".into())
    }
}
