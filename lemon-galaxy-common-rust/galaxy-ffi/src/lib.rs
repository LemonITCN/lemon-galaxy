//! LemonGalaxy 内核导出接口（uniffi）。见 docs/ffi.md。
//!
//! 所有异步函数都投递到内核自己的多线程运行时执行，再把结果交回宿主（Swift async / Kotlin suspend），
//! 不依赖宿主的执行器。回调可能在任意内核线程触发，界面层需自行切回主线程。

uniffi::setup_scaffolding!();

mod agent;
mod callbacks;
mod client;
mod host;
mod session;
mod types;

pub use agent::*;
pub use callbacks::*;
pub use client::*;
pub use host::*;
pub use session::*;
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
    #[error("{message}")]
    Network { message: String },
    #[error("{message}")]
    Auth { code: String, message: String },
    #[error("{message}")]
    Permission { message: String },
    #[error("{message}")]
    NotFound { message: String },
    #[error("{message}")]
    Protocol { message: String },
    #[error("已取消")]
    Cancelled,
    #[error("{message}")]
    Other { message: String },
}

impl From<galaxy_client::ClientError> for GalaxyError {
    fn from(e: galaxy_client::ClientError) -> Self {
        use galaxy_client::ClientError as C;
        match e {
            C::Network(m) => Self::Network { message: m },
            C::Auth { code, message } => Self::Auth { code, message },
            C::Permission(m) => Self::Permission { message: m },
            C::NotFound(m) => Self::NotFound { message: m },
            C::Protocol(m) => Self::Protocol { message: m },
            C::Cancelled => Self::Cancelled,
            C::Other(m) => Self::Other { message: m },
        }
    }
}

impl From<anyhow::Error> for GalaxyError {
    fn from(e: anyhow::Error) -> Self {
        Self::Other { message: e.to_string() }
    }
}

pub type Result<T> = std::result::Result<T, GalaxyError>;

pub(crate) fn other(m: impl Into<String>) -> GalaxyError {
    GalaxyError::Other { message: m.into() }
}

#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
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
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::try_new(format!("{level},quinn=warn,rustls=warn,hyper=warn,h2=warn,tungstenite=warn")).unwrap_or_else(|_| "info".into()))
            .with_writer(std::io::stderr)
            .try_init();
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
