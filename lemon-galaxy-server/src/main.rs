//! LemonGalaxy 服务器。
//!
//! 一个端口号（默认 7443）：TCP 上是 HTTPS（管理后台、客户端接口、信令、TCP 中转），UDP 上是探测与中转。
//! 数据全部在数据目录下：server.db（SQLite）与 keys/（密钥，权限 0600）。
//! 前台运行时数据目录默认是当前目录下的 lemon-galaxy-data；install-service 装成系统服务（Windows / Linux / macOS），见 service 模块。
//! 在终端里不带任何参数运行时进交互菜单（menu 模块），脚本、systemd、Docker 里不带参数照常前台运行。

mod admin;
mod ai;
mod audit;
mod auth;
mod client;
mod db;
mod error;
mod keys;
mod logfile;
mod menu;
mod service;
mod state;
mod tls;
mod tui;
mod udp;
mod update;
mod web;
mod ws;

use anyhow::Context;
use axum::http::{header, HeaderValue};
use axum::routing::{get, post};
use axum::Router;
use clap::{Parser, Subcommand};
use parking_lot::{Mutex, RwLock};
use r2d2_sqlite::rusqlite::{params, OptionalExtension};
use std::future::{Future, IntoFuture};
use std::net::{SocketAddr, SocketAddrV6};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tower_http::set_header::SetResponseHeaderLayer;

/// 前台运行时的默认数据目录（相对当前目录）。
pub(crate) const LOCAL_DATA_DIR: &str = "./lemon-galaxy-data";

#[derive(Parser)]
#[command(name = "lemon-galaxy-server", version, about = "LemonGalaxy 服务器")]
struct Cli {
    /// 数据目录（数据库与密钥）。前台运行默认为当前目录下的 lemon-galaxy-data；
    /// 系统服务默认为 %ProgramData%\LemonGalaxy\server（Windows）、/var/lib/lemon-galaxy-server（Linux）
    /// 或 /Library/Application Support/LemonGalaxy/server（macOS）
    #[arg(long, env = "LEMON_GALAXY_DATA", global = true)]
    data_dir: Option<PathBuf>,
    /// 端口（TCP 与 UDP 同号），默认 7443
    #[arg(long, env = "LEMON_GALAXY_PORT", global = true)]
    port: Option<u16>,
    /// 监听地址，默认 :: （同时监听 IPv4 与 IPv6）
    #[arg(long, env = "LEMON_GALAXY_BIND", global = true)]
    bind: Option<String>,
    /// 对外的主机名或 IP（写进客户端地址与自签证书）
    #[arg(long, env = "LEMON_GALAXY_PUBLIC_HOST", global = true)]
    public_host: Option<String>,
    /// 使用正规证书（PEM），不传则自动生成自签证书
    #[arg(long, env = "LEMON_GALAXY_TLS_CERT", global = true)]
    tls_cert: Option<PathBuf>,
    #[arg(long, env = "LEMON_GALAXY_TLS_KEY", global = true)]
    tls_key: Option<PathBuf>,
    /// 日志同时写到这个文件（超过 10 MB 轮换，保留 3 份）。系统服务固定写数据目录下的 logs/server.log
    #[arg(long, env = "LEMON_GALAXY_LOG_FILE", global = true)]
    log_file: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// 前台运行服务器（默认；在终端里不带任何参数运行时进交互菜单）
    Run,
    /// 显示客户端地址与指纹
    Info,
    /// 安装为系统服务：开机自启、出错自动重启（Windows 需管理员，Linux 与 macOS 需 root）。
    /// --port、--public-host、--bind、--tls-cert/--tls-key 写进服务；再次运行可修改，没写的沿用上次，写空值表示清除
    InstallService,
    /// 卸载系统服务（数据目录保留）
    UninstallService,
    /// 启动系统服务
    StartService,
    /// 停止系统服务（开机时仍会自动启动）
    StopService,
    /// 重启系统服务
    RestartService,
    /// 显示系统服务状态、客户端地址与初始化网址
    Status,
    /// 由 Windows 服务管理器启动（内部使用）
    #[command(hide = true)]
    Service,
    /// 管理员账号
    Admin {
        #[command(subcommand)]
        cmd: AdminCmd,
    },
    /// 普通用户（通常在 Web 后台管理，这里给无图形界面的场景用）
    User {
        #[command(subcommand)]
        cmd: UserCmd,
    },
    /// 大模型供应商（通常在 Web 后台管理）
    Ai {
        #[command(subcommand)]
        cmd: AiCmd,
    },
    /// 升级服务器程序（从 GitHub 取最新发布，验发布签名；Linux、Windows 与 macOS）。装了系统服务会自动重启它
    SelfUpdate {
        /// 只检查，不升级
        #[arg(long)]
        check: bool,
    },
}

#[derive(Subcommand)]
enum AiCmd {
    /// 添加供应商，API Key 从文件读取（避免出现在命令历史里）
    AddProvider {
        #[arg(long, default_value = "opencode_go")]
        kind: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long)]
        key_file: PathBuf,
    },
    /// 给用户开通某供应商的模型（--models 默认 *，表示全部）
    Grant {
        username: String,
        provider_id: i64,
        #[arg(long, default_value = "*")]
        models: String,
    },
}

#[derive(Subcommand)]
enum UserCmd {
    /// 创建用户（首次登录必须改密码）
    Create { username: String, password: String, #[arg(long)] display_name: Option<String> },
    /// 列出用户
    List,
}

#[derive(Subcommand)]
enum AdminCmd {
    /// 创建管理员（首次登录需绑定两步验证）
    Create { username: String, password: String },
    /// 重置管理员密码（同时解除两步验证并注销其会话）
    ResetPassword { username: String, password: String },
}

/// 运行服务器所需的参数。
#[derive(Clone)]
pub(crate) struct ServeCtx {
    data_dir: PathBuf,
    port: u16,
    bind: String,
    public_host: Option<String>,
    tls_cert: Option<PathBuf>,
    tls_key: Option<PathBuf>,
}

impl ServeCtx {
    fn tls_source(&self) -> keys::TlsSource {
        keys::TlsSource { cert_path: self.tls_cert.clone(), key_path: self.tls_key.clone() }
    }
}

fn main() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    // Windows 上从资源管理器双击打开：控制台窗口是专门开的，出错时停住让人看清
    #[cfg(windows)]
    let double_click = std::env::args_os().len() <= 1 && service::windows::launched_from_explorer();
    #[cfg(not(windows))]
    let double_click = false;
    let code = match real_main(double_click) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("错误：{e:#}");
            1
        }
    };
    if double_click && code != 0 {
        eprintln!();
        eprintln!("按回车键退出");
        let _ = std::io::stdin().read_line(&mut String::new());
    }
    std::process::exit(code);
}

fn init_logging(console: bool, file: Option<&Path>) -> anyhow::Result<()> {
    use std::io::IsTerminal;
    use tracing_subscriber::prelude::*;
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,hyper=warn,rustls=warn".into());
    // Windows 控制台默认不认 ANSI 颜色码，只在类 Unix 终端上用颜色
    let console_layer = console.then(|| tracing_subscriber::fmt::layer().with_ansi(cfg!(unix) && std::io::stdout().is_terminal()));
    let file_layer = match file {
        Some(p) => {
            let f = logfile::RotatingFile::open(p).with_context(|| format!("无法打开日志文件 {}", p.display()))?;
            Some(tracing_subscriber::fmt::layer().with_ansi(false).with_writer(std::sync::Mutex::new(f)))
        }
        None => None,
    };
    tracing_subscriber::registry().with(filter).with(console_layer).with(file_layer).init();
    Ok(())
}

/// 打开数据库、读设置（命令行的 --public-host 覆盖设置里的）。
fn open_data(data_dir: &Path, public_host: Option<&str>) -> anyhow::Result<(db::Db, PathBuf, state::Settings)> {
    std::fs::create_dir_all(data_dir).with_context(|| format!("无法创建数据目录 {}", data_dir.display()))?;
    let db_path = data_dir.join("server.db");
    let db = db::Db::open(&db_path).with_context(|| format!("无法打开数据库 {}（系统服务的数据目录只有管理员 / root 能访问）", db_path.display()))?;
    let mut settings: state::Settings = db
        .call_sync(|c| c.query_row("SELECT value FROM settings WHERE key = 'settings'", [], |r| r.get::<_, String>(0)).optional())?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    if let Some(h) = public_host {
        settings.public_host = h.to_string();
    }
    Ok((db, db_path, settings))
}

/// 管理命令（admin / user / ai / info / status / self-update）用的数据目录与参数：
/// 命令行给了就用命令行的；当前目录下有 lemon-galaxy-data 就用它（与以前一致）；否则装了系统服务就用服务的数据目录和参数。
fn manage_ctx(cli: &Cli) -> ServeCtx {
    let (ctx, from_service) = resolve_manage_ctx(cli);
    if from_service && cli.data_dir.is_none() {
        eprintln!("（使用系统服务的数据目录 {}）", ctx.data_dir.display());
    }
    ctx
}

/// 同 manage_ctx，不打印提示；第二项表示用的是系统服务的数据目录与参数。
pub(crate) fn resolve_manage_ctx(cli: &Cli) -> (ServeCtx, bool) {
    let svc = service::load_config();
    let (data_dir, svc) = match &cli.data_dir {
        Some(d) => {
            let same = svc.filter(|c| std::path::absolute(d).map(|a| a == c.data_dir).unwrap_or(false));
            (d.clone(), same)
        }
        None if Path::new(LOCAL_DATA_DIR).exists() => (PathBuf::from(LOCAL_DATA_DIR), None),
        None => match svc {
            Some(c) => (c.data_dir.clone(), Some(c)),
            None => (PathBuf::from(LOCAL_DATA_DIR), None),
        },
    };
    let from_service = svc.is_some();
    let svc = svc.as_ref();
    let ctx = ServeCtx {
        data_dir,
        port: cli.port.or(svc.map(|c| c.port)).unwrap_or(galaxy_proto::DEFAULT_PORT),
        bind: cli.bind.clone().or(svc.and_then(|c| c.bind.clone())).unwrap_or_else(|| "::".into()),
        public_host: cli.public_host.clone().or(svc.and_then(|c| c.public_host.clone())),
        tls_cert: cli.tls_cert.clone().or(svc.and_then(|c| c.tls_cert.clone())),
        tls_key: cli.tls_key.clone().or(svc.and_then(|c| c.tls_key.clone())),
    };
    (ctx, from_service)
}

/// 前台运行与服务运行用的参数：只看命令行（服务的参数写在启动命令里）。
fn serve_ctx(cli: &Cli, default_dir: PathBuf) -> ServeCtx {
    ServeCtx {
        data_dir: cli.data_dir.clone().unwrap_or(default_dir),
        port: cli.port.unwrap_or(galaxy_proto::DEFAULT_PORT),
        bind: cli.bind.clone().unwrap_or_else(|| "::".into()),
        public_host: cli.public_host.clone(),
        tls_cert: cli.tls_cert.clone(),
        tls_key: cli.tls_key.clone(),
    }
}

fn real_main(double_click: bool) -> anyhow::Result<()> {
    let cli = Cli::parse();
    // 一次性打印的命令（status、用户列表等）输出接到管道、对方先关了（如 `status | head`）时按命令行惯例安静退出。
    // 前台运行与交互菜单（菜单里也能前台运行）保持 Rust 默认的忽略 SIGPIPE：恢复默认的话，给已断开的客户端写数据
    // （writev 不带 MSG_NOSIGNAL）会让整个服务器被系统直接杀掉、不留日志
    #[cfg(unix)]
    if !matches!(cli.cmd, None | Some(Cmd::Run)) {
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        }
    }
    // 上次自动升级留下的旧程序（Windows 上运行中的程序只能改名，不能覆盖）
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old"));
    }
    // 在终端里不带任何参数运行：交互菜单（Windows 双击打开的控制台也算终端）
    if cli.cmd.is_none() && std::env::args_os().len() <= 1 && menu::available() {
        return menu::run(&cli);
    }

    match &cli.cmd {
        None | Some(Cmd::Run) => {
            init_logging(true, cli.log_file.as_deref())?;
            if double_click {
                println!("LemonGalaxy 服务器 {}，在这个窗口里前台运行，关闭窗口服务器就会停止。", galaxy_proto::APP_VERSION);
                println!("要装成开机自启的系统服务，请以管理员身份打开 PowerShell，在本目录运行：");
                println!("  .\\lemon-galaxy-server.exe install-service --public-host <你的域名或 IP>");
                println!();
            }
            return serve_blocking(serve_ctx(&cli, PathBuf::from(LOCAL_DATA_DIR)), shutdown_signal());
        }
        Some(Cmd::Service) => {
            #[cfg(not(windows))]
            anyhow::bail!("service 命令只给 Windows 服务管理器用，前台运行请用 lemon-galaxy-server run");
            #[cfg(windows)]
            {
                let default_dir = service::default_data_dir().unwrap_or_else(|| PathBuf::from(LOCAL_DATA_DIR));
                let ctx = serve_ctx(&cli, default_dir);
                // 服务没有控制台，日志只写文件
                let log = cli.log_file.clone().unwrap_or_else(|| service::log_file_in(&ctx.data_dir));
                init_logging(false, Some(&log))?;
                return service::windows::run_dispatcher(ctx);
            }
        }
        Some(Cmd::InstallService) => {
            let o = service::Overrides {
                data_dir: cli.data_dir.clone(),
                port: cli.port,
                bind: cli.bind.clone(),
                public_host: cli.public_host.clone(),
                tls_cert: cli.tls_cert.clone(),
                tls_key: cli.tls_key.clone(),
            };
            return service::install(&o);
        }
        Some(Cmd::UninstallService) => return service::uninstall(),
        Some(Cmd::StartService) => return service::control(service::Ctl::Start),
        Some(Cmd::StopService) => return service::control(service::Ctl::Stop),
        Some(Cmd::RestartService) => return service::control(service::Ctl::Restart),
        _ => {}
    }

    init_logging(true, None)?;
    let ctx = manage_ctx(&cli);
    // 这两个命令不要凭空建数据目录
    let has_db = ctx.data_dir.join("server.db").exists();
    match &cli.cmd {
        Some(Cmd::SelfUpdate { check }) => {
            let repo = if has_db { open_data(&ctx.data_dir, None)?.2.update_github_repo } else { state::Settings::default().update_github_repo };
            let r = tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(update::self_update(&repo, *check))?;
            match r {
                None => println!("已是最新版本 {}", galaxy_proto::APP_VERSION),
                Some(u) if *check => println!("有新版本 {}（当前 {}）。运行 lemon-galaxy-server self-update 升级", u.version, galaxy_proto::APP_VERSION),
                Some(u) => {
                    println!("已升级到 {}：{}", u.version, u.path.display());
                    match service::restart_if_installed() {
                        Ok(true) => println!("已重启系统服务，新版本已生效。"),
                        Ok(false) => println!("请重启服务器让新版本生效。"),
                        Err(e) => println!("重启系统服务失败（{e:#}），请手动重启。"),
                    }
                }
            }
            return Ok(());
        }
        Some(Cmd::Status) => {
            print_status(&ctx);
            return Ok(());
        }
        _ => {}
    }

    let (db, _db_path, settings) = open_data(&ctx.data_dir, ctx.public_host.as_deref())?;
    match &cli.cmd {
        Some(Cmd::Admin { cmd }) => admin_cli(&db, cmd),
        Some(Cmd::Ai { cmd }) => {
            let keys = keys::Keys::load(&ctx.data_dir, &settings.public_host, &ctx.tls_source())?;
            ai_cli(&db, &keys, cmd)
        }
        Some(Cmd::User { cmd }) => {
            let keys = keys::Keys::load(&ctx.data_dir, &settings.public_host, &ctx.tls_source())?;
            user_cli(&db, &keys, cmd)
        }
        Some(Cmd::Info) => {
            let keys = keys::Keys::load(&ctx.data_dir, &settings.public_host, &ctx.tls_source())?;
            println!("客户端地址: {}", state::client_address(&settings.public_host, ctx.port, &keys.fingerprint));
            Ok(())
        }
        _ => Ok(()),
    }
}

/// 管理员个数（数据库不存在或打不开时为 None，不会新建数据库）。
pub(crate) fn admin_count(data_dir: &Path) -> Option<i64> {
    if !data_dir.join("server.db").exists() {
        return None;
    }
    open_data(data_dir, None).ok().and_then(|(db, _, _)| db.call_sync(|c| c.query_row("SELECT COUNT(*) FROM admins", [], |r| r.get::<_, i64>(0))).ok())
}

/// 服务器没运行过时用的客户端地址（从密钥算；密钥不存在时不生成）。
pub(crate) fn fallback_address(ctx: &ServeCtx) -> String {
    match std::fs::metadata(ctx.data_dir.join("keys")) {
        Ok(_) => open_data(&ctx.data_dir, ctx.public_host.as_deref())
            .and_then(|(_, _, s)| Ok(state::client_address(&s.public_host, ctx.port, &keys::Keys::load(&ctx.data_dir, &s.public_host, &ctx.tls_source())?.fingerprint)))
            .unwrap_or_else(|e| format!("（{e:#}）")),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => "（没有权限读数据目录，请以管理员 / root 身份运行）".to_string(),
        Err(_) => "（服务器还没有运行过）".to_string(),
    }
}

/// status 命令。
pub(crate) fn print_status(ctx: &ServeCtx) {
    service::print_status(&ctx.data_dir, &fallback_address(ctx), admin_count(&ctx.data_dir));
}

/// 前台运行时的停止信号：Ctrl+C，Unix 上还有 SIGTERM（systemd stop、kill）。
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    r = tokio::signal::ctrl_c() => if r.is_err() { term.recv().await; },
                    _ = term.recv() => {}
                }
            }
            Err(_) => {
                if tokio::signal::ctrl_c().await.is_err() {
                    std::future::pending::<()>().await;
                }
            }
        }
    }
    #[cfg(not(unix))]
    {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
    tracing::info!("收到停止信号，正在停止");
}

/// 运行服务器，直到 shutdown 完成（前台运行与系统服务共用）。
pub(crate) fn serve_blocking(ctx: ServeCtx, shutdown: impl Future<Output = ()> + Send + 'static) -> anyhow::Result<()> {
    let (db, db_path, settings) = open_data(&ctx.data_dir, ctx.public_host.as_deref())?;
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let r = rt.block_on(run(ctx, db, db_path, settings, shutdown));
    rt.shutdown_timeout(Duration::from_secs(3));
    r
}

fn admin_cli(db: &db::Db, cmd: &AdminCmd) -> anyhow::Result<()> {
    let now = galaxy_proto::now_ms();
    match cmd {
        AdminCmd::Create { username, password } => {
            anyhow::ensure!(admin::valid_username(username), "用户名需为 3 到 32 位字母、数字或 ._-");
            anyhow::ensure!(password.chars().count() >= 8, "密码至少 8 位");
            let h = admin::hash_password(password)?;
            db.call_sync(|c| c.execute("INSERT INTO admins(username, password_hash, created_at) VALUES(?1,?2,?3)", params![username, h, now]))?;
            println!("已创建管理员 {username}，首次登录后台时需要绑定两步验证。");
        }
        AdminCmd::ResetPassword { username, password } => {
            anyhow::ensure!(password.chars().count() >= 8, "密码至少 8 位");
            let h = admin::hash_password(password)?;
            let n = db.call_sync(|c| {
                let n = c.execute("UPDATE admins SET password_hash = ?1, totp_secret = NULL, totp_enabled = 0 WHERE username = ?2", params![h, username])?;
                c.execute("DELETE FROM admin_sessions WHERE admin_id IN (SELECT id FROM admins WHERE username = ?1)", params![username])?;
                Ok(n)
            })?;
            anyhow::ensure!(n > 0, "管理员 {username} 不存在");
            println!("已重置管理员 {username} 的密码，并解除了两步验证，下次登录需重新绑定。");
        }
    }
    Ok(())
}

fn user_cli(db: &db::Db, keys: &keys::Keys, cmd: &UserCmd) -> anyhow::Result<()> {
    match cmd {
        UserCmd::Create { username, password, display_name } => {
            anyhow::ensure!(admin::valid_username(username), "用户名需为 3 到 32 位字母、数字或 ._-");
            anyhow::ensure!(password.chars().count() >= 8, "密码至少 8 位");
            let record = keys.opaque.register_with_password(username, password)?;
            let now = galaxy_proto::now_ms();
            let dn = display_name.clone().unwrap_or_default();
            db.call_sync(|c| c.execute("INSERT INTO users(username, display_name, opaque_record, created_at) VALUES(?1,?2,?3,?4)", params![username, dn, record, now]))?;
            println!("已创建用户 {username}，首次登录时需要改密码。");
        }
        UserCmd::List => {
            let rows: Vec<(i64, String, String, i64)> = db.call_sync(|c| {
                let mut s = c.prepare("SELECT id, username, display_name, disabled FROM users ORDER BY id")?;
                let v = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<Vec<_>, _>>()?;
                Ok(v)
            })?;
            for (id, u, d, dis) in rows {
                println!("{id}\t{u}\t{d}{}", if dis != 0 { "\t(已禁用)" } else { "" });
            }
        }
    }
    Ok(())
}

fn ai_cli(db: &db::Db, keys: &keys::Keys, cmd: &AiCmd) -> anyhow::Result<()> {
    match cmd {
        AiCmd::AddProvider { kind, name, base_url, key_file } => {
            anyhow::ensure!(kind == "opencode_go" || kind == "openai_compatible", "kind 只能是 opencode_go 或 openai_compatible");
            let key = std::fs::read_to_string(key_file)?.trim().to_string();
            let enc = keys.seal(key.as_bytes(), "ai-key");
            let base = base_url.clone().unwrap_or_else(|| ai::OPENCODE_GO_MODELS.to_string());
            let name = name.clone().unwrap_or_else(|| if kind == "opencode_go" { "OpenCode Go".into() } else { "OpenAI Compatible".into() });
            let now = galaxy_proto::now_ms();
            let id = db.call_sync(|c| {
                c.execute("INSERT INTO ai_providers(name, kind, base_url, api_key_enc, created_at) VALUES(?1,?2,?3,?4,?5)", params![name, kind, base, enc, now])?;
                Ok(c.last_insert_rowid())
            })?;
            println!("已添加供应商 {id}（{name}）。在后台或客户端首次请求时会拉取模型列表。");
        }
        AiCmd::Grant { username, provider_id, models } => {
            let list: Vec<String> = models.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            let j = serde_json::to_string(&list)?;
            let n = db.call_sync(|c| {
                let uid: i64 = c.query_row("SELECT id FROM users WHERE username = ?1", params![username], |r| r.get(0))?;
                c.execute("INSERT INTO ai_grants(user_id, provider_id, models_json) VALUES(?1,?2,?3) ON CONFLICT(user_id, provider_id) DO UPDATE SET models_json = excluded.models_json", params![uid, provider_id, j])
            })?;
            println!("已为 {username} 开通供应商 {provider_id} 的模型 {models}（{n}）");
        }
    }
    Ok(())
}

fn bind_dual(bind: &str, port: u16) -> anyhow::Result<(std::net::TcpListener, std::net::UdpSocket)> {
    use socket2::{Domain, Protocol, Socket, Type};
    let ip: std::net::IpAddr = bind.parse()?;
    let addr = SocketAddr::new(ip, port);
    let domain = if ip.is_ipv6() { Domain::IPV6 } else { Domain::IPV4 };

    let tcp = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
    if ip.is_ipv6() {
        let _ = tcp.set_only_v6(false);
    }
    tcp.set_reuse_address(true)?;
    tcp.bind(&addr.into())?;
    tcp.listen(1024)?;
    tcp.set_nonblocking(true)?;

    let udp = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;
    if ip.is_ipv6() {
        let _ = udp.set_only_v6(false);
    }
    let _ = udp.set_recv_buffer_size(8 << 20);
    let _ = udp.set_send_buffer_size(8 << 20);
    udp.bind(&addr.into())?;
    udp.set_nonblocking(true)?;
    Ok((tcp.into(), udp.into()))
}

async fn run(ctx: ServeCtx, db: db::Db, db_path: PathBuf, settings: state::Settings, shutdown: impl Future<Output = ()> + Send + 'static) -> anyhow::Result<()> {
    let keys = keys::Keys::load(&ctx.data_dir, &settings.public_host, &ctx.tls_source())?;
    let (tcp_std, udp_std) = bind_dual(&ctx.bind, ctx.port)
        .or_else(|e| {
            tracing::warn!("双栈监听失败（{e}），改为只监听 IPv4");
            bind_dual("0.0.0.0", ctx.port)
        })
        .with_context(|| format!("无法监听端口 {}（是不是已经有服务器在运行？）", ctx.port))?;
    let tcp = tokio::net::TcpListener::from_std(tcp_std)?;
    let udp = Arc::new(tokio::net::UdpSocket::from_std(udp_std)?);

    let admins: i64 = db.call(|c| c.query_row("SELECT COUNT(*) FROM admins", [], |r| r.get(0))).await?;
    let setup_token = if admins == 0 { Some(galaxy_proto::b64::encode(&galaxy_crypto::random_bytes::<18>())) } else { None };

    let http = reqwest::Client::builder()
        .use_rustls_tls()
        .user_agent(format!("LemonGalaxy/{}", galaxy_proto::APP_VERSION))
        .connect_timeout(Duration::from_secs(15))
        .build()?;

    let st: state::St = Arc::new(state::AppState {
        db,
        db_path,
        data_dir: ctx.data_dir.clone(),
        keys,
        settings: RwLock::new(settings),
        hub: Default::default(),
        relay: Default::default(),
        conns: RwLock::new(Default::default()),
        logins: Mutex::new(Default::default()),
        rate: Default::default(),
        started_at: galaxy_proto::now_ms(),
        port: ctx.port,
        udp,
        http,
        setup_token: Mutex::new(setup_token.clone()),
        updates: Default::default(),
    });

    tokio::spawn(udp::run(st.clone()));
    tokio::spawn(housekeeping(st.clone()));
    tokio::spawn(update::run(st.clone()));

    let csp = "default-src 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'; font-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";
    let app = Router::new()
        .nest("/api/v1/admin", admin::routes())
        .nest("/api/v1", client::routes())
        .route("/api/v1/ai/models", get(ai::client_models))
        .route("/api/v1/ai/chat/completions", post(ai::client_chat))
        .route("/api/v1/ws", get(ws::ws_handler))
        .route("/api/v1/relay", get(ws::relay_handler))
        .fallback(web::static_handler)
        .layer(SetResponseHeaderLayer::overriding(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(csp)))
        .layer(SetResponseHeaderLayer::overriding(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(SetResponseHeaderLayer::overriding(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")))
        .layer(SetResponseHeaderLayer::overriding(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer")))
        .layer(axum::extract::DefaultBodyLimit::max(8 * 1024 * 1024))
        .with_state(st.clone());

    let listener = tls::TlsListener::new(tcp, st.keys.tls.clone())?;
    let setup_url = setup_token.map(|t| format!("https://{}:{}/setup?token={}", state::bracket_host(&st.public_host()), ctx.port, t));
    tracing::info!("LemonGalaxy 服务器 {} 已启动，端口 {}（TCP + UDP），数据目录 {}", galaxy_proto::APP_VERSION, ctx.port, ctx.data_dir.display());
    tracing::info!("客户端填写的服务器地址：{}", st.client_address());
    if let Some(u) = &setup_url {
        tracing::info!("尚未创建管理员。请打开 {u} 完成初始化（浏览器会提示自签证书，确认指纹 {} 后继续）", st.keys.fingerprint);
    }
    let run_info = service::RunInfo { pid: std::process::id(), version: galaxy_proto::APP_VERSION.into(), started_at: galaxy_proto::now_ms(), address: st.client_address(), setup_url };
    if let Err(e) = service::write_run_info(&ctx.data_dir, &run_info) {
        tracing::warn!("写运行信息失败：{e}");
    }

    // 收到停止信号后不再接新连接，等现有请求结束；长连接（WebSocket）最多再等 5 秒
    let stop = tokio_util::sync::CancellationToken::new();
    let stop2 = stop.clone();
    tokio::spawn(async move {
        shutdown.await;
        stop2.cancel();
    });
    let stop3 = stop.clone();
    let serve = axum::serve(listener, app.into_make_service_with_connect_info::<tls::PeerAddr>()).with_graceful_shutdown(async move { stop3.cancelled().await });
    tokio::select! {
        r = serve.into_future() => r?,
        _ = async {
            stop.cancelled().await;
            tokio::time::sleep(Duration::from_secs(5)).await;
        } => {}
    }
    tracing::info!("服务器已停止");
    Ok(())
}

/// 定时清理：过期的中转、登录状态、会话、审计日志。
async fn housekeeping(st: state::St) {
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    let mut n: u64 = 0;
    loop {
        tick.tick().await;
        n += 1;
        for conn in st.relay.sweep(10 * 60_000) {
            st.conns.write().remove(&conn);
        }
        // 长时间停在「连接中」且没有中转的记录
        let now = galaxy_proto::now_ms();
        st.conns.write().retain(|id, c| now - c.started_at < 24 * 3600_000 || st.relay.pairs.read().contains_key(id));
        st.logins.lock().retain(|_, p| p.created.elapsed().as_secs() < 120);
        if n % 60 == 1 {
            let (idle_days, audit_days) = {
                let s = st.settings.read();
                (s.session_idle_days as i64, s.audit_retention_days as i64)
            };
            let _ = st
                .db
                .call(move |c| {
                    if idle_days > 0 {
                        c.execute("DELETE FROM sessions WHERE last_used_at < ?1", params![now - idle_days * 86_400_000])?;
                    }
                    if audit_days > 0 {
                        c.execute("DELETE FROM audit WHERE at < ?1", params![now - audit_days * 86_400_000])?;
                    }
                    c.execute("DELETE FROM admin_sessions WHERE expires_at < ?1", params![now])?;
                    Ok(())
                })
                .await;
            // 中转流量计入当日统计
            let total = st.relay.total_bytes.swap(0, std::sync::atomic::Ordering::Relaxed);
            if total > 0 {
                state::bump_stat(&st, "relay_bytes", total as i64).await;
            }
        }
    }
}

#[allow(dead_code)]
fn _unused(_: SocketAddrV6) {}
