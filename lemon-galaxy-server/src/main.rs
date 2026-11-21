//! LemonGalaxy 服务器。
//!
//! 一个端口号（默认 7443）：TCP 上是 HTTPS（管理后台、客户端接口、信令、TCP 中转），UDP 上是探测与中转。
//! 数据全部在 --data-dir 下：server.db（SQLite）与 keys/（密钥，权限 0600）。

mod admin;
mod ai;
mod audit;
mod auth;
mod client;
mod db;
mod error;
mod keys;
mod state;
mod tls;
mod udp;
mod web;
mod ws;

use axum::http::{header, HeaderValue};
use axum::routing::{get, post};
use axum::Router;
use clap::{Parser, Subcommand};
use parking_lot::{Mutex, RwLock};
use r2d2_sqlite::rusqlite::{params, OptionalExtension};
use std::net::{SocketAddr, SocketAddrV6};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tower_http::set_header::SetResponseHeaderLayer;

#[derive(Parser)]
#[command(name = "lemon-galaxy-server", version, about = "LemonGalaxy 服务器")]
struct Cli {
    /// 数据目录（数据库与密钥）
    #[arg(long, env = "LEMON_GALAXY_DATA", default_value = "./lemon-galaxy-data", global = true)]
    data_dir: PathBuf,
    /// 端口（TCP 与 UDP 同号）
    #[arg(long, env = "LEMON_GALAXY_PORT", default_value_t = galaxy_proto::DEFAULT_PORT, global = true)]
    port: u16,
    /// 监听地址，默认同时监听 IPv4 与 IPv6
    #[arg(long, env = "LEMON_GALAXY_BIND", default_value = "::", global = true)]
    bind: String,
    /// 对外的主机名或 IP（写进客户端地址与自签证书）
    #[arg(long, env = "LEMON_GALAXY_PUBLIC_HOST", global = true)]
    public_host: Option<String>,
    /// 使用正规证书（PEM），不传则自动生成自签证书
    #[arg(long, env = "LEMON_GALAXY_TLS_CERT", global = true)]
    tls_cert: Option<PathBuf>,
    #[arg(long, env = "LEMON_GALAXY_TLS_KEY", global = true)]
    tls_key: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// 运行服务器（默认）
    Run,
    /// 显示客户端地址与指纹
    Info,
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

fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,hyper=warn,rustls=warn".into()))
        .init();
    let cli = Cli::parse();
    std::fs::create_dir_all(&cli.data_dir)?;
    let db_path = cli.data_dir.join("server.db");
    let db = db::Db::open(&db_path)?;

    let mut settings: state::Settings = db
        .call_sync(|c| c.query_row("SELECT value FROM settings WHERE key = 'settings'", [], |r| r.get::<_, String>(0)).optional())?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    if let Some(h) = &cli.public_host {
        settings.public_host = h.clone();
    }

    match &cli.cmd {
        Some(Cmd::Admin { cmd }) => return admin_cli(&db, cmd),
        Some(Cmd::User { cmd }) => {
            let keys = keys::Keys::load(&cli.data_dir, &settings.public_host, &keys::TlsSource { cert_path: cli.tls_cert.clone(), key_path: cli.tls_key.clone() })?;
            return user_cli(&db, &keys, cmd);
        }
        Some(Cmd::Info) => {
            let keys = keys::Keys::load(&cli.data_dir, &settings.public_host, &keys::TlsSource { cert_path: cli.tls_cert.clone(), key_path: cli.tls_key.clone() })?;
            let host = if settings.public_host.is_empty() { "127.0.0.1".into() } else { settings.public_host.clone() };
            println!("客户端地址: {}:{}#{}", host, cli.port, keys.fingerprint);
            return Ok(());
        }
        _ => {}
    }

    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(run(cli, db, db_path, settings))
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

async fn run(cli: Cli, db: db::Db, db_path: PathBuf, settings: state::Settings) -> anyhow::Result<()> {
    let keys = keys::Keys::load(&cli.data_dir, &settings.public_host, &keys::TlsSource { cert_path: cli.tls_cert.clone(), key_path: cli.tls_key.clone() })?;
    let (tcp_std, udp_std) = bind_dual(&cli.bind, cli.port).or_else(|e| {
        tracing::warn!("双栈监听失败（{e}），改为只监听 IPv4");
        bind_dual("0.0.0.0", cli.port)
    })?;
    let tcp = tokio::net::TcpListener::from_std(tcp_std)?;
    let udp = Arc::new(tokio::net::UdpSocket::from_std(udp_std)?);

    let admins: i64 = db.call(|c| c.query_row("SELECT COUNT(*) FROM admins", [], |r| r.get(0))).await?;
    let setup_token = if admins == 0 { Some(galaxy_proto::b64::encode(&galaxy_crypto::random_bytes::<18>())) } else { None };

    let http = reqwest::Client::builder()
        .use_rustls_tls()
        .user_agent(format!("LemonGalaxy/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .build()?;

    let st: state::St = Arc::new(state::AppState {
        db,
        db_path,
        data_dir: cli.data_dir.clone(),
        keys,
        settings: RwLock::new(settings),
        hub: Default::default(),
        relay: Default::default(),
        conns: RwLock::new(Default::default()),
        logins: Mutex::new(Default::default()),
        rate: Default::default(),
        started_at: galaxy_proto::now_ms(),
        port: cli.port,
        udp,
        http,
        setup_token: Mutex::new(setup_token.clone()),
    });

    tokio::spawn(udp::run(st.clone()));
    tokio::spawn(housekeeping(st.clone()));

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
    tracing::info!("LemonGalaxy 服务器 {} 已启动，端口 {}（TCP + UDP）", env!("CARGO_PKG_VERSION"), cli.port);
    tracing::info!("客户端填写的服务器地址：{}", st.client_address());
    if let Some(t) = setup_token {
        tracing::info!("尚未创建管理员。请打开 https://{}:{}/setup?token={} 完成初始化（浏览器会提示自签证书，确认指纹 {} 后继续）", st.public_host(), cli.port, t, st.keys.fingerprint);
    }
    axum::serve(listener, app.into_make_service_with_connect_info::<tls::PeerAddr>()).await?;
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
