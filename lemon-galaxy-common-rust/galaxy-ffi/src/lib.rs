//! LemonGalaxy 内核导出接口（uniffi）。见 docs/ffi.md。
//!
//! 所有异步函数都投递到内核自己的多线程运行时执行，再把结果交回宿主（Swift async / Kotlin suspend），
//! 不依赖宿主的执行器。回调可能在任意内核线程触发，界面层需自行切回主线程。

uniffi::setup_scaffolding!();

mod agent;
mod callbacks;
mod client;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod decode;
mod host;
mod screen;
mod session;
mod ssh;
mod types;

pub use agent::*;
pub use callbacks::*;
pub use client::*;
pub use host::*;
pub use session::*;
pub use ssh::{SshInfo, SshSession};
pub use types::*;

use parking_lot::RwLock;
use std::future::Future;
use std::sync::OnceLock;

static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
static CONFIG: RwLock<Option<CoreConfig>> = RwLock::new(None);

pub(crate) fn rt() -> &'static tokio::runtime::Runtime {
    RT.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(4).thread_name("galaxy-core").enable_all().build().expect("tokio runtime"))
}

/// 在内核运行时里执行，宿主侧只等待结果。
pub(crate) async fn run<F, T>(f: F) -> T
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    rt().spawn(f).await.expect("内核任务异常退出")
}

pub(crate) fn config() -> CoreConfig {
    CONFIG.read().clone().unwrap_or(CoreConfig {
        data_dir: std::env::temp_dir().join("lemon-galaxy").to_string_lossy().to_string(),
        cache_dir: std::env::temp_dir().join("lemon-galaxy-cache").to_string_lossy().to_string(),
        log_level: "info".into(),
        platform: galaxy_proto::Platform::current().into(),
        device_kind: DeviceKind::Mobile,
        app_version: env!("CARGO_PKG_VERSION").into(),
        os_version: String::new(),
        hostname: "device".into(),
        locale: "zh-CN".into(),
    })
}

pub(crate) fn locale() -> String {
    CONFIG.read().as_ref().map(|c| c.locale.clone()).unwrap_or_else(|| "zh-CN".into())
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum GalaxyError {
    #[error("{detail}")]
    Network { detail: String },
    #[error("{detail}")]
    Auth { code: String, detail: String },
    #[error("{detail}")]
    Permission { detail: String },
    #[error("{detail}")]
    NotFound { detail: String },
    #[error("{detail}")]
    Protocol { detail: String },
    #[error("已取消")]
    Cancelled,
    #[error("{detail}")]
    Other { detail: String },
}

impl From<galaxy_client::ClientError> for GalaxyError {
    fn from(e: galaxy_client::ClientError) -> Self {
        use galaxy_client::ClientError as C;
        match e {
            C::Network(m) => Self::Network { detail: m },
            C::Auth { code, message } => Self::Auth { code, detail: message },
            C::Permission(m) => Self::Permission { detail: m },
            C::NotFound(m) => Self::NotFound { detail: m },
            C::Protocol(m) => Self::Protocol { detail: m },
            C::Cancelled => Self::Cancelled,
            C::Other(m) => Self::Other { detail: m },
        }
    }
}

impl From<anyhow::Error> for GalaxyError {
    fn from(e: anyhow::Error) -> Self {
        Self::Other { detail: e.to_string() }
    }
}

pub type Result<T> = std::result::Result<T, GalaxyError>;

pub(crate) fn other(m: impl Into<String>) -> GalaxyError {
    GalaxyError::Other { detail: m.into() }
}

#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// 内核日志文件（界面「导出日志」用）。安卓上日志在 logcat，返回空字符串。
#[uniffi::export]
pub fn core_log_path() -> String {
    if cfg!(target_os = "android") {
        return String::new();
    }
    CONFIG.read().as_ref().map(|c| std::path::PathBuf::from(&c.data_dir).join("logs").join("core.log").to_string_lossy().to_string()).unwrap_or_default()
}

/// 每个进程调用一次。
#[uniffi::export]
pub fn core_init(config: CoreConfig) {
    galaxy_net::init_crypto();
    let _ = std::fs::create_dir_all(&config.data_dir);
    let _ = std::fs::create_dir_all(&config.cache_dir);
    let level = config.log_level.clone();
    *CONFIG.write() = Some(config);
    #[cfg(target_os = "android")]
    {
        android_logger::init_once(android_logger::Config::default().with_tag("LemonGalaxy").with_max_level(if level == "debug" { log::LevelFilter::Debug } else { log::LevelFilter::Info }));
    }
    #[cfg(not(target_os = "android"))]
    {
        // 写到数据目录下的 logs/core.log（超过 8 MB 时转存为 core.log.1），不带颜色
        let dir = std::path::PathBuf::from(&CONFIG.read().as_ref().map(|c| c.data_dir.clone()).unwrap_or_default()).join("logs");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("core.log");
        if std::fs::metadata(&path).map(|m| m.len() > 8 * 1024 * 1024).unwrap_or(false) {
            let _ = std::fs::rename(&path, dir.join("core.log.1"));
        }
        let filter = tracing_subscriber::EnvFilter::try_new(format!("{level},quinn=warn,rustls=warn,hyper=warn,h2=warn,tungstenite=warn,reqwest=warn")).unwrap_or_else(|_| "info".into());
        match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            Ok(f) => {
                let _ = tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).with_writer(std::sync::Mutex::new(f)).try_init();
            }
            Err(_) => {
                let _ = tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).with_writer(std::io::stderr).try_init();
            }
        }
    }
    let _ = rt();
}

/// 运行时切换界面语言（影响内核返回的错误文字）。
#[uniffi::export]
pub fn core_set_locale(locale: String) {
    if let Some(c) = CONFIG.write().as_mut() {
        c.locale = locale;
    }
}
