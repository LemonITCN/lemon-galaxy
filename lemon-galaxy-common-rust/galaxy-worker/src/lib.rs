//! LemonGalaxy 被控端命令行的公共部分。Linux 与 Windows 被控端共用同一套命令：
//!
//! ```text
//! lemon-galaxy-worker                 查看状态；还没登录时引导登录
//! lemon-galaxy-worker login           登录一个账号（可登录多个）
//! lemon-galaxy-worker logout <用户名>  退出某个账号
//! lemon-galaxy-worker policy <用户名> [选项]   修改该账号在本机的权限
//! lemon-galaxy-worker install-service 安装为开机自启服务
//! lemon-galaxy-worker run             前台运行（服务调用）
//! lemon-galaxy-worker assist          临时协助：显示设备码与验证码，本人确认后对方才能连入
//! lemon-galaxy-worker disable|enable  一键禁止 / 允许远程
//! lemon-galaxy-worker events          查看本机远程操作日志
//! ```
//!
//! 各系统的差异（数据目录、服务安装、画面与输入等）由 [`WorkerOs`] 提供。

pub mod control;
#[cfg(any(unix, windows))]
mod control_server;
pub mod gdm;
#[cfg(unix)]
mod localsys;
pub mod upgrade;
#[cfg(windows)]
mod localsys_win;

use galaxy_client::account::{self, DeviceMeta, LoginOutcome};
use galaxy_client::api::Api;
use galaxy_client::identity::{FileStore, Identity, SecureStore};
use galaxy_host::accounts::HostState;
use galaxy_host::{HostConfig, HostPolicy, HostService};
use galaxy_net::ServerAddr;
use galaxy_proto::{AiMode, DeviceKind, Platform};
use galaxy_host::platform::HostPlatform;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const VERSION: &str = galaxy_proto::APP_VERSION;

/// 核心程序（被控服务）的文件名
pub const CORE_EXE: &str = if cfg!(windows) { "lemon-galaxy-worker.exe" } else { "lemon-galaxy-worker" };

/// 各系统的差异部分。
pub trait WorkerOs: Send + Sync + 'static {
    fn platform(&self) -> Platform;
    fn os_version(&self) -> String;
    /// 管理员（root / 已提升的管理员）与普通用户的默认数据目录
    fn default_data_dir(&self, admin: bool) -> PathBuf;
    /// 登录时默认的「远程终端与文件以哪个系统用户运行」
    fn default_system_user(&self) -> String;
    /// 最高权限账号名（root / SYSTEM），选它要二次确认
    fn superuser_name(&self) -> &'static str;
    fn install_service(&self, dir: &Path) -> i32;
    fn uninstall_service(&self) -> i32;
    /// 画面、输入、剪贴板（命令行被控端没有）
    fn host_platform(&self) -> Option<Arc<dyn HostPlatform>> {
        None
    }
    /// 数据目录建好后收紧权限（Windows 设 ACL）
    fn secure_data_dir(&self, _dir: &Path) {}
    /// 界面语言是否中文（None 时看 LANG 等环境变量）
    fn ui_zh(&self) -> Option<bool> {
        None
    }
    /// 系统专有命令（如 Windows 的 service）；不认识返回 None
    fn extra_command(&self, _cmd: &str, _rest: &[String], _dir: &Path) -> Option<i32> {
        None
    }
    /// 更新时要替换的程序（已安装服务时是服务用的那份）；None 为当前程序
    fn update_target(&self) -> Option<PathBuf> {
        None
    }
    /// 重启已安装的服务（自动更新换好程序后用）；没有服务或失败返回 false
    fn restart_service(&self) -> bool {
        false
    }
    /// 停止已安装的服务并等它停下（更新守护进程换回旧版本前用）；没有服务或失败返回 false
    fn stop_service(&self) -> bool {
        false
    }
    /// 启动已安装的服务（换回旧版本后用；Linux 先清掉「启动太频繁」的失败状态）
    fn start_service(&self) -> bool {
        self.restart_service()
    }
    /// 本机装的桌面端（界面 + 被控服务一起的安装包）：有的话自动更新换整套，没有只换核心程序
    fn desktop_install(&self) -> Option<upgrade::DesktopInstall> {
        None
    }
    /// 桌面端整套换成新版本并确认后（Windows：改「应用和功能」里显示的版本号）
    fn desktop_updated(&self, _version: &str) {}
    /// 帮助里补充的系统专有命令说明 (命令, 中文, 英文)
    fn extra_help(&self) -> Vec<(&'static str, &'static str, &'static str)> {
        vec![]
    }
    /// 系统服务开机自启（Windows：服务启动类型自动 / 手动）：None = 这个系统用 systemd（控制接口自己查）；
    /// Some(None) = 还没装服务；Some(Some(开没开))
    fn service_autostart(&self) -> Option<Option<bool>> {
        None
    }
    /// 改服务开机自启（只在 service_autostart 返回 Some(Some(_)) 时调用）
    fn set_service_autostart(&self, _on: bool) -> Result<(), String> {
        Err("not supported".into())
    }
}

static ZH: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

pub fn zh() -> bool {
    *ZH.get_or_init(|| {
        let l = std::env::var("LEMON_GALAXY_LANG").or_else(|_| std::env::var("LC_ALL")).or_else(|_| std::env::var("LC_MESSAGES")).or_else(|_| std::env::var("LANG")).unwrap_or_default();
        l.is_empty() || l.starts_with("zh")
    })
}

#[macro_export]
macro_rules! t {
    ($zh:expr, $en:expr) => {
        if $crate::zh() { $zh } else { $en }
    };
}

fn is_admin() -> bool {
    galaxy_host::sysuser::is_admin()
}

/// 数据目录：参数 > 环境变量 LEMON_GALAXY_WORKER_DIR > 系统默认。
pub fn data_dir(os: &dyn WorkerOs, arg: Option<PathBuf>) -> PathBuf {
    if let Some(d) = arg.or_else(|| std::env::var_os("LEMON_GALAXY_WORKER_DIR").map(PathBuf::from)) {
        return d;
    }
    os.default_data_dir(is_admin())
}

pub fn meta(os: &dyn WorkerOs, kind: DeviceKind) -> DeviceMeta {
    DeviceMeta {
        kind,
        platform: os.platform(),
        os_version: os.os_version(),
        arch: std::env::consts::ARCH.into(),
        hostname: galaxy_host::hostname(),
        app_version: VERSION.into(),
    }
}

fn prompt(label: &str, default: Option<&str>) -> String {
    match default {
        Some(d) if !d.is_empty() => print!("{label} [{d}]: "),
        _ => print!("{label}: "),
    }
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    let _ = std::io::stdin().lock().read_line(&mut s);
    let s = s.trim().to_string();
    if s.is_empty() {
        default.unwrap_or("").to_string()
    } else {
        s
    }
}

fn prompt_secret(label: &str) -> String {
    rpassword::prompt_password(format!("{label}: ")).unwrap_or_default()
}

pub fn store(dir: &std::path::Path) -> Arc<dyn SecureStore> {
    Arc::new(FileStore::new(dir.join("keys")).expect("无法创建密钥目录"))
}

struct Args {
    cmd: String,
    rest: Vec<String>,
    data_dir: Option<PathBuf>,
}

fn parse_args() -> Args {
    let mut v: Vec<String> = std::env::args().skip(1).collect();
    let mut data_dir = None;
    if let Some(i) = v.iter().position(|a| a == "--data-dir") {
        if i + 1 < v.len() {
            data_dir = Some(PathBuf::from(v[i + 1].clone()));
            v.drain(i..=i + 1);
        }
    }
    let cmd = if v.is_empty() { "status".to_string() } else { v.remove(0) };
    Args { cmd, rest: v, data_dir }
}

fn flag(rest: &[String], name: &str) -> Option<String> {
    rest.iter().position(|a| a == name).and_then(|i| rest.get(i + 1).cloned())
}

/// 命令行入口。`__lg-file-helper` 子进程也从这里进。
pub fn main(os: Arc<dyn WorkerOs>) -> ! {
    let raw: Vec<String> = std::env::args().collect();
    if raw.get(1).map(|s| s.as_str()) == Some("__lg-file-helper") {
        std::process::exit(galaxy_host::files::helper_main(&raw[2..]));
    }
    if let Some(z) = os.ui_zh() {
        let _ = ZH.set(z);
    }
    galaxy_net::init_crypto();
    let a = parse_args();
    let dir = data_dir(os.as_ref(), a.data_dir.clone());
    let guard = a.cmd == "__lg-update-guard";
    let log_level = if a.cmd == "run" || a.cmd == "service" || guard { "info" } else { "warn" };
    let filter = || tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| format!("{log_level},quinn=warn,rustls=warn").into());
    // 系统服务没有终端：日志写到数据目录 logs/worker.log（超过 8 MB 转存为 worker.log.1）；更新守护进程写 logs/update.log
    let log_name = if guard { Some("update.log") } else if a.cmd == "service" { Some("worker.log") } else { None };
    let log_file = log_name.map(|name| {
        let d = dir.join("logs");
        let _ = std::fs::create_dir_all(&d);
        let p = d.join(name);
        if std::fs::metadata(&p).map(|m| m.len() > 8 * 1024 * 1024).unwrap_or(false) {
            let _ = std::fs::rename(&p, d.join(format!("{name}.1")));
        }
        std::fs::OpenOptions::new().create(true).append(true).open(p).ok()
    });
    match log_file.flatten() {
        Some(f) => tracing_subscriber::fmt().with_env_filter(filter()).with_writer(std::sync::Mutex::new(f)).with_ansi(false).init(),
        None => tracing_subscriber::fmt().with_env_filter(filter()).with_writer(std::io::stderr).with_ansi(std::io::stderr().is_terminal()).init(),
    }
    if galaxy_proto::update::TEST_RELEASE_KEY && (a.cmd == "run" || a.cmd == "service" || guard) {
        tracing::warn!("这是只认测试发布密钥的测试构建，不能用于正式环境");
    }
    // 自动更新：新版本试运行中的记账与守护进程，放在启动最早、最不容易出错的地方（还没联网、没起线程）
    if a.cmd == "run" || a.cmd == "service" {
        upgrade::on_start(&dir);
    }
    if guard {
        std::process::exit(upgrade::guard_main(os.clone(), &dir));
    }
    if let Some(code) = os.extra_command(&a.cmd, &a.rest, &dir) {
        std::process::exit(code);
    }
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("runtime");
    let code = rt.block_on(async move {
        match a.cmd.as_str() {
            "status" => status(os.as_ref(), &dir, true).await,
            "login" => login(os.as_ref(), &dir, &a.rest).await,
            "logout" => logout(&dir, &a.rest).await,
            "policy" => policy(&dir, &a.rest),
            "run" => run_host(os.clone(), &dir, shutdown_signal()).await,
            "assist" => assist(os.as_ref(), &a.rest).await,
            "enable" => set_enabled(&dir, true),
            "disable" => set_enabled(&dir, false),
            "events" => events(&dir, &a.rest),
            "update" => update_cmd(os.clone(), &dir, &a.rest).await,
            "auto-update" => auto_update_cmd(&dir, &a.rest),
            "lock-on-end" => lock_on_end_cmd(&dir, &a.rest),
            "headless-display" => headless_display_cmd(&dir, &a.rest),
            "install-service" => os.install_service(&dir),
            "uninstall-service" => os.uninstall_service(),
            "version" | "--version" | "-V" => {
                println!("lemon-galaxy-worker {VERSION}");
                if galaxy_proto::update::TEST_RELEASE_KEY {
                    println!("（测试构建：只认测试发布密钥 / test build: trusts the test release key only）");
                }
                0
            }
            "help" | "--help" | "-h" => {
                help(os.as_ref());
                0
            }
            other => {
                eprintln!("{} {other}", t!("未知命令", "Unknown command"));
                help(os.as_ref());
                2
            }
        }
    });
    std::process::exit(code);
}

fn help(os: &dyn WorkerOs) {
    println!("LemonGalaxy worker {VERSION}\n");
    if zh() {
        println!("用法：lemon-galaxy-worker [命令]\n");
        println!("  （无）              查看状态；还没登录时引导登录");
        println!("  login               登录一个账号（可登录多个账号）");
        println!("  logout <用户名>     退出某个账号");
        println!("  policy <用户名>     修改该账号在本机的权限：");
        println!("                        --system-user <名>  终端与文件以哪个系统用户运行");
        println!("                        --ai off|read_only|confirm_all|confirm_dangerous|full_trust");
        println!("                        --terminal on|off  --files on|off");
        println!("  install-service     安装开机自启服务");
        println!("  uninstall-service   卸载服务");
        println!("  run                 前台运行（服务调用）");
        println!("  assist              临时协助：显示设备码与验证码，需本人确认对方才能连入");
        println!("  disable / enable    一键禁止 / 允许远程");
        println!("  events [-n 50]      查看本机远程操作日志");
        println!("  update [--check]    检查并安装新版本（发布签名校验通过才装）");
        println!("  auto-update on|off  服务模式下是否自动更新（默认开）");
        println!("  lock-on-end on|off  远程看过桌面的会话都结束后自动锁屏（默认关）");
        println!("  headless-display off|宽x高[@刷新率]|default");
        println!("                      本机没有亮着的显示器时自动加的虚拟屏（默认 1920x1080@60，需要虚拟屏驱动）");
        for (c, z, _) in os.extra_help() {
            println!("  {c:<19} {z}");
        }
        println!("\n全局选项：--data-dir <目录>（默认管理员为 {}，普通用户为 {}）", os.default_data_dir(true).display(), os.default_data_dir(false).display());
    } else {
        println!("Usage: lemon-galaxy-worker [command]\n");
        println!("  (none)              Show status; guide you to sign in if not signed in");
        println!("  login               Sign in an account (multiple accounts allowed)");
        println!("  logout <user>       Sign out an account");
        println!("  policy <user>       Change this account's permissions on this machine:");
        println!("                        --system-user <name> --ai <mode> --terminal on|off --files on|off");
        println!("  install-service     Install as a startup service");
        println!("  uninstall-service   Remove the service");
        println!("  run                 Run in foreground (used by the service)");
        println!("  assist              Remote assistance with device code and verification code");
        println!("  disable / enable    Disable / enable remote access");
        println!("  events [-n 50]      Show local remote-operation log");
        println!("  update [--check]    Check for and install a new version (signature verified)");
        println!("  auto-update on|off  Auto-update when running as a service (default on)");
        println!("  lock-on-end on|off  Lock the screen after remote desktop sessions end (default off)");
        println!("  headless-display off|WxH[@Hz]|default");
        println!("                      Virtual display added when no monitor is on (default 1920x1080@60, needs the driver)");
        for (c, _, e) in os.extra_help() {
            println!("  {c:<19} {e}");
        }
        println!("\nGlobal option: --data-dir <dir> (default {} as administrator, {} otherwise)", os.default_data_dir(true).display(), os.default_data_dir(false).display());
    }
}

async fn status(os: &dyn WorkerOs, dir: &std::path::Path, guide: bool) -> i32 {
    let st = HostState::load(&dir.join("host.json"));
    let id = Identity::load_or_create(&store(dir));
    println!("LemonGalaxy worker {VERSION}");
    println!("{}: {}", t!("设备 ID", "Device ID"), id.device_id);
    println!("{}: {}", t!("数据目录", "Data dir"), dir.display());
    if st.accounts.is_empty() {
        println!("\n{}", t!("还没有登录任何账号。", "No account signed in."));
        if guide && atty_stdin() {
            println!();
            return login(os, dir, &[]).await;
        }
        println!("{}", t!("运行 lemon-galaxy-worker login 登录。", "Run `lemon-galaxy-worker login` to sign in."));
        return 0;
    }
    println!("{}: {}", t!("服务器", "Server"), st.server);
    let live: Option<galaxy_host::HostStatus> = std::fs::read(dir.join("status.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let fresh = std::fs::metadata(dir.join("status.json")).and_then(|m| m.modified()).map(|t| t.elapsed().map(|e| e.as_secs() < 30).unwrap_or(false)).unwrap_or(false);
    match (&live, fresh) {
        (Some(s), true) => {
            println!("{}: {}", t!("服务", "Service"), if s.online { t!("运行中，已连上服务器", "running, connected") } else { t!("运行中，正在连接服务器", "running, connecting") });
            if !s.public_ip.is_empty() {
                println!("{}: {}", t!("公网 IP", "Public IP"), s.public_ip);
            }
            if !s.sessions.is_empty() {
                println!("\n{}", t!("当前被控：", "Active sessions:"));
                for x in &s.sessions {
                    println!("  {}  {:?}  {}{}", x.from_user, x.path, if x.ai_active { "AI " } else { "" }, x.id);
                }
            }
        }
        _ => println!("{}: {}", t!("服务", "Service"), t!("未运行（运行 lemon-galaxy-worker install-service 安装开机自启，或 lemon-galaxy-worker run 前台运行）", "not running (run `lemon-galaxy-worker install-service`, or `lemon-galaxy-worker run` in foreground)")),
    }
    println!("{}: {}", t!("远程", "Remote access"), if st.remote_enabled { t!("允许", "enabled") } else { t!("已禁止", "disabled") });
    println!("\n{}", t!("账号：", "Accounts:"));
    for a in &st.accounts {
        let status = match a.status {
            galaxy_host::AccountStatus::Active => t!("正常", "active"),
            galaxy_host::AccountStatus::NeedsRelogin => t!("需要重新登录", "needs re-login"),
        };
        println!(
            "  {} ({})  {}  {}={}  AI={}  {}={} {}={}",
            a.username,
            a.display_name,
            status,
            t!("系统用户", "system user"),
            a.policy.system_user,
            a.policy.ai_cap.as_str(),
            t!("终端", "terminal"),
            onoff(a.policy.terminal),
            t!("文件", "files"),
            onoff(a.policy.files)
        );
    }
    0
}

fn onoff(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

fn atty_stdin() -> bool {
    std::io::stdin().is_terminal()
}

async fn login(os: &dyn WorkerOs, dir: &std::path::Path, rest: &[String]) -> i32 {
    let path = dir.join("host.json");
    let mut st = HostState::load(&path);
    let server_s = match flag(rest, "--server") {
        Some(s) => s,
        None => prompt(t!("服务器地址（后台「概览」页给出，形如 host:7443#sha256-...）", "Server address (host:7443#sha256-...)"), if st.server.is_empty() { None } else { Some(&st.server) }),
    };
    let server = match ServerAddr::parse(&server_s) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}: {e}", t!("服务器地址错误", "Invalid server address"));
            return 2;
        }
    };
    if !st.server.is_empty() && st.server != server.to_string_full() && !st.accounts.is_empty() {
        let ok = prompt(t!("更换服务器会退出所有已登录的账号，继续？(yes/no)", "Changing server signs out all accounts. Continue? (yes/no)"), Some("no"));
        if ok != "yes" {
            return 1;
        }
        st.accounts.clear();
    }
    let username = flag(rest, "--user").unwrap_or_else(|| prompt(t!("用户名", "Username"), None));
    // 无人值守部署可用环境变量传密码（不要放在命令行参数里，会被 ps 看到）
    let password = std::env::var("LEMON_GALAXY_PASSWORD").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| prompt_secret(t!("密码", "Password")));
    let api = match Api::new(server.clone(), if zh() { "zh-CN" } else { "en" }) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let id = Identity::load_or_create(&store(dir));
    let m = meta(os, DeviceKind::Worker);
    let mut out = account::login(&api, &id, &m, &username, &password, None).await;
    if let Ok(LoginOutcome::NeedTotp) = out {
        let code = prompt(t!("两步验证码", "Two-factor code"), None);
        out = account::login(&api, &id, &m, &username, &password, Some(&code)).await;
    }
    let logged = match out {
        Ok(LoginOutcome::Done(l)) => l,
        Ok(LoginOutcome::MustChangePassword(fl)) => {
            println!("{}", t!("首次登录需要设置新密码（至少 8 位）。设置后服务器和管理员都不知道你的密码。", "First sign-in: please set a new password (8+ characters)."));
            let (p1, p2) = match std::env::var("LEMON_GALAXY_NEW_PASSWORD").ok().filter(|s| !s.is_empty()) {
                Some(p) => (p.clone(), p),
                None => (prompt_secret(t!("新密码", "New password")), prompt_secret(t!("再输入一次", "Repeat"))),
            };
            if p1 != p2 {
                eprintln!("{}", t!("两次输入不一致", "Passwords do not match"));
                return 1;
            }
            match account::complete_first_login(&api, &id, &m, fl, &p1).await {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("{e}");
                    return 1;
                }
            }
        }
        Ok(LoginOutcome::NeedTotp) => {
            eprintln!("{}", t!("两步验证码不正确", "Invalid two-factor code"));
            return 1;
        }
        Err(e) => {
            eprintln!("{}: {e}", t!("登录失败", "Sign-in failed"));
            return 1;
        }
    };

    // 选择系统用户
    let default_user = os.default_system_user();
    let sys = match flag(rest, "--system-user") {
        Some(s) => s,
        None if is_admin() => prompt(t!("这个账号的远程终端和文件操作以哪个系统用户运行", "Run remote terminal and files as which system user"), Some(&default_user)),
        None => default_user.clone(),
    };
    let Some(sys_user) = galaxy_host::sysuser::SysUser::lookup(&sys) else {
        eprintln!("{} {sys}", t!("系统用户不存在：", "No such system user:"));
        return 1;
    };
    let sys = sys_user.display_name();
    if sys.eq_ignore_ascii_case(os.superuser_name()) {
        let ok = flag(rest, "--yes-root").map(|_| "yes".to_string()).unwrap_or_else(|| prompt(&format!("{} {} {}", t!("以", "Allow remote terminal as"), os.superuser_name(), t!("身份开放远程终端风险很高，确认？(yes/no)", "? This is risky. (yes/no)")), Some("no")));
        if ok != "yes" {
            return 1;
        }
    }
    let mut st = HostState::load(&path);
    st.server = server.to_string_full();
    st.accounts.retain(|a| a.user_id != logged.user.id);
    st.accounts.push(galaxy_host::HostAccount {
        user_id: logged.user.id,
        username: logged.user.username.clone(),
        display_name: logged.user.display_name.clone(),
        account_pubkey: logged.account_pubkey,
        epoch: logged.epoch,
        cert: logged.cert.clone(),
        token: logged.token.clone(),
        status: galaxy_host::AccountStatus::Active,
        policy: HostPolicy::default_for(&sys),
        revoked: vec![],
        added_at: galaxy_proto::now_ms(),
    });
    if let Ok(info) = api.get::<galaxy_proto::api::ServerInfoResp>("/info").await {
        st.server_sign_pubkey = info.server_sign_pubkey;
    }
    os.secure_data_dir(dir);
    if let Err(e) = st.save(&path) {
        eprintln!("{}: {e}", t!("保存失败", "Save failed"));
        return 1;
    }
    println!("\n{} {}（{}）", t!("已登录", "Signed in as"), logged.user.username, logged.user.display_name);
    println!("{} {sys}", t!("远程终端与文件以此系统用户运行：", "Remote terminal and files run as:"));
    let running = std::fs::metadata(dir.join("status.json")).and_then(|m| m.modified()).map(|t| t.elapsed().map(|e| e.as_secs() < 30).unwrap_or(false)).unwrap_or(false);
    if !running {
        println!("{}", t!("服务未运行。运行 lemon-galaxy-worker install-service 安装开机自启（推荐），或 lemon-galaxy-worker run 前台运行。", "Service not running. Run `lemon-galaxy-worker install-service`."));
    }
    0
}

async fn logout(dir: &std::path::Path, rest: &[String]) -> i32 {
    let Some(name) = rest.first() else {
        eprintln!("{}", t!("用法：lemon-galaxy-worker logout <用户名>", "Usage: lemon-galaxy-worker logout <user>"));
        return 2;
    };
    let path = dir.join("host.json");
    let mut st = HostState::load(&path);
    let Some(a) = st.accounts.iter().find(|a| a.username.eq_ignore_ascii_case(name)).cloned() else {
        eprintln!("{} {name}", t!("本机没有登录这个账号：", "Account not signed in:"));
        return 1;
    };
    if let Ok(server) = ServerAddr::parse(&st.server) {
        if let Ok(api) = Api::new(server, "zh-CN") {
            api.set_token(Some(a.token.clone()));
            let _ = api.post::<_, serde_json::Value>("/auth/logout", &serde_json::json!({})).await;
        }
    }
    st.accounts.retain(|x| x.user_id != a.user_id);
    let _ = st.save(&path);
    println!("{} {}", t!("已退出", "Signed out"), a.username);
    0
}

fn policy(dir: &std::path::Path, rest: &[String]) -> i32 {
    let Some(name) = rest.first() else {
        eprintln!("{}", t!("用法：lemon-galaxy-worker policy <用户名> [选项]", "Usage: lemon-galaxy-worker policy <user> [options]"));
        return 2;
    };
    let path = dir.join("host.json");
    let mut st = HostState::load(&path);
    let Some(a) = st.accounts.iter_mut().find(|a| a.username.eq_ignore_ascii_case(name)) else {
        eprintln!("{} {name}", t!("本机没有登录这个账号：", "Account not signed in:"));
        return 1;
    };
    if let Some(u) = flag(rest, "--system-user") {
        if galaxy_host::sysuser::SysUser::lookup(&u).is_none() {
            eprintln!("{} {u}", t!("系统用户不存在：", "No such system user:"));
            return 1;
        }
        a.policy.system_user = u;
    }
    if let Some(m) = flag(rest, "--ai") {
        match AiMode::parse(&m) {
            Some(x) => a.policy.ai_cap = x,
            None => {
                eprintln!("--ai off|read_only|confirm_all|confirm_dangerous|full_trust");
                return 2;
            }
        }
    }
    for (f, field) in [("--terminal", 0), ("--files", 1), ("--view", 2), ("--control", 3), ("--clipboard", 4)] {
        if let Some(v) = flag(rest, f) {
            let on = v == "on" || v == "true" || v == "1";
            match field {
                0 => a.policy.terminal = on,
                1 => a.policy.files = on,
                2 => a.policy.view = on,
                3 => a.policy.control = on,
                _ => a.policy.clipboard = on,
            }
        }
    }
    let p = a.policy.clone();
    let _ = st.save(&path);
    println!("{}: {} AI={} {}={} {}={}", name, p.system_user, p.ai_cap.as_str(), t!("终端", "terminal"), onoff(p.terminal), t!("文件", "files"), onoff(p.files));
    0
}

fn set_enabled(dir: &std::path::Path, on: bool) -> i32 {
    let path = dir.join("host.json");
    let mut st = HostState::load(&path);
    st.remote_enabled = on;
    let _ = st.save(&path);
    println!("{}", if on { t!("已允许远程。", "Remote access enabled.") } else { t!("已禁止远程，现有连接会被断开。", "Remote access disabled.") });
    0
}

fn events(dir: &std::path::Path, rest: &[String]) -> i32 {
    let n: usize = flag(rest, "-n").and_then(|s| s.parse().ok()).unwrap_or(50);
    let Ok(s) = std::fs::read_to_string(dir.join("events.log")) else {
        println!("{}", t!("暂无记录", "No events"));
        return 0;
    };
    let lines: Vec<&str> = s.lines().collect();
    for l in lines.iter().skip(lines.len().saturating_sub(n)) {
        if let Ok(e) = serde_json::from_str::<galaxy_host::service::HostLogEvent>(l) {
            let secs = e.at / 1000;
            println!("{} {} {:<10} {}{}", fmt_time(secs), e.user, e.kind, if e.ai { "[AI] " } else { "" }, e.detail);
        }
    }
    0
}

fn fmt_time(secs: i64) -> String {
    let d = secs / 86400;
    let s = secs % 86400;
    // 以 UTC 显示，避免引入时区依赖
    let (y, m, dd) = civil(d);
    format!("{y:04}-{m:02}-{dd:02} {:02}:{:02}:{:02}Z", s / 3600, (s % 3600) / 60, s % 60)
}

fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 平台层：系统自带的；端到端测试时（LEMON_GALAXY_TEST_TONE=1）换成只有一路测试音的平台层。
fn platform_for(os: &dyn WorkerOs) -> Option<Arc<dyn galaxy_host::platform::HostPlatform>> {
    if std::env::var("LEMON_GALAXY_TEST_TONE").is_ok_and(|v| v == "1") {
        return Some(Arc::new(galaxy_host::platform::TestPlatform::from_env()));
    }
    os.host_platform()
}

/// 运行被控服务，直到 `stop` 完成。命令行 `run` 与 Windows 服务共用。
///
/// 还没登录时不退出：开着本机控制接口等图形界面或命令行登录，登录后马上开始；换了服务器就重启被控服务。
pub async fn run_host(os: Arc<dyn WorkerOs>, dir: &std::path::Path, stop: impl std::future::Future<Output = ()>) -> i32 {
    let _ = std::fs::create_dir_all(dir);
    os.secure_data_dir(dir);
    #[cfg(any(unix, windows))]
    let ctl = control_server::start(os.clone(), dir).await;
    #[cfg(not(any(unix, windows)))]
    let ctl: Option<Arc<NoControl>> = None;
    // 正在运行的被控服务（自动更新看有没有人在远程、新版本有没有连上服务器）
    let current: Arc<parking_lot::RwLock<Option<Arc<HostService>>>> = Arc::default();
    // 新版本试运行中：连上服务器并稳定一段时间后确认
    {
        let cur = current.clone();
        upgrade::watch_health(os.clone(), dir, move || cur.read().as_ref().is_some_and(|h| h.status().online));
    }
    // 服务模式下定时自动更新：换好程序后退出，由服务管理器拉起新版本；有人在远程时先推迟
    let supervised = std::env::var("LEMON_GALAXY_SUPERVISED").as_deref() == Ok("1");
    if supervised {
        let cur = current.clone();
        let ctl2 = ctl.clone();
        let busy: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(move || {
            let host_busy = cur.read().as_ref().is_some_and(|h| !h.status().sessions.is_empty());
            host_busy || ctl2.as_ref().is_some_and(|c| c.assist_busy())
        });
        tokio::spawn(upgrade::auto_loop(os.clone(), dir.to_path_buf(), busy));
    }
    // 端到端测试的故障注入（只在测试构建里生效）：起来就崩 / 一直不连服务器
    let fault = upgrade::test_fault();
    if fault == "crash" {
        tracing::error!("测试故障注入：启动后崩溃");
        tokio::time::sleep(Duration::from_secs(1)).await;
        std::process::exit(3);
    }
    let path = dir.join("host.json");
    let device_id = Identity::load_or_create(&store(dir)).device_id.to_hex();
    let mut host: Option<Arc<HostService>> = None;
    let mut last = None;
    let mut retry_at: Option<tokio::time::Instant> = None;
    let mut told_waiting = false;
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    let mut n = 0u64;
    tokio::pin!(stop);
    loop {
        // 还没有在运行的被控服务：看看登录了没有
        if host.is_none() && fault != "offline" && retry_at.is_none_or(|t| tokio::time::Instant::now() >= t) {
            retry_at = None;
            let st = HostState::load(&path);
            if st.server.is_empty() {
                if !told_waiting {
                    told_waiting = true;
                    eprintln!("{}", t!("还没有登录，等待登录（运行 lemon-galaxy-worker login，或在图形界面里登录）……", "Not signed in. Waiting for sign-in (run `lemon-galaxy-worker login` or use the app)..."));
                }
            } else {
                match ServerAddr::parse(&st.server) {
                    Err(e) => {
                        tracing::warn!("服务器地址无效：{e}");
                        retry_at = Some(tokio::time::Instant::now() + Duration::from_secs(30));
                    }
                    Ok(server) => {
                        last = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                        match start_host(os.clone(), dir, server).await {
                            Ok(h) => {
                                // 主控端在设备详情里「升级被控端」：与界面「立即更新」同一条路（只在有服务管理器拉起新版本时提供）
                                if supervised {
                                    h.set_updater(Arc::new(RemoteUpdate { os: os.clone(), dir: dir.to_path_buf() }));
                                }
                                if let Some(c) = &ctl {
                                    c.set_host(Some(h.clone()));
                                }
                                h.write_status_file();
                                *current.write() = Some(h.clone());
                                host = Some(h);
                            }
                            Err(e) => {
                                eprintln!("{}: {e}", t!("启动失败", "Start failed"));
                                retry_at = Some(tokio::time::Instant::now() + Duration::from_secs(10));
                            }
                        }
                    }
                }
            }
        }
        let woken = async {
            match &ctl {
                Some(c) => c.woken().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = tick.tick() => {
                n += 1;
                if n % 3 == 1 {
                    match &host {
                        Some(h) => h.write_status_file(),
                        None => write_waiting_status(dir, &device_id),
                    }
                }
            }
            _ = woken => {}
            _ = &mut stop => {
                tracing::info!("收到退出信号");
                if let Some(c) = &ctl {
                    c.close();
                }
                if let Some(h) = &host {
                    h.disconnect_all();
                }
                let _ = std::fs::remove_file(dir.join("status.json"));
                return 0;
            }
        }
        let Some(h) = host.clone() else { continue };
        let now = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if now == last {
            continue;
        }
        last = now;
        // 换了服务器（或退出了服务器）：停掉现在的被控服务，下一轮按新配置启动
        let disk = HostState::load(&path);
        if disk.server != h.cfg.server.to_string_full() {
            tracing::info!("服务器地址变了，重启被控服务");
            if let Some(c) = &ctl {
                c.set_host(None);
            }
            h.shutdown();
            *current.write() = None;
            host = None;
            continue;
        }
        if h.reload_from_disk() {
            tracing::info!("已重新读取账号配置");
        }
    }
}

/// 远程升级（主控端在设备详情里点「升级被控端」，galaxy-host 的 MgmtReq::ApplyUpdate）：先查有没有新版本，有就立即回复版本号，
/// 在后台做完整的零交互升级（下载、整套换、退出让服务管理器拉起新版本、出问题自动换回），与界面「立即更新」同一条路。
/// 远程会话随服务重启断开，被控端以新版本自己连回服务器。
struct RemoteUpdate {
    os: Arc<dyn WorkerOs>,
    dir: PathBuf,
}

impl galaxy_host::update::RemoteUpdater for RemoteUpdate {
    fn start(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<String>, String>> + Send + '_>> {
        Box::pin(async move {
            let Some(found) = upgrade::check(self.os.as_ref(), &self.dir).await? else { return Ok(None) };
            let version = found.info.version.clone();
            tracing::info!("主控端要求远程升级到 {version}");
            let (os, dir) = (self.os.clone(), self.dir.clone());
            let t0 = std::time::Instant::now();
            tokio::spawn(async move {
                match upgrade::update_now(os, &dir, true).await {
                    Ok(Some(a)) if a.exit => {
                        // 给回复留时间发到主控端，再退出让服务管理器拉起新版本
                        if let Some(rest) = Duration::from_secs(2).checked_sub(t0.elapsed()) {
                            tokio::time::sleep(rest).await;
                        }
                        tracing::info!("远程升级：已换上 {}，重启服务", a.version);
                        exit_for_update(&dir);
                    }
                    Ok(Some(a)) => tracing::info!("远程升级：开始安装 {}，装好后服务自动重启", a.version),
                    Ok(None) => tracing::info!("远程升级：已是最新版本"),
                    Err(e) => tracing::warn!("远程升级失败：{e}"),
                }
            });
            Ok(Some(version))
        })
    }
}

async fn start_host(os: Arc<dyn WorkerOs>, dir: &Path, server: ServerAddr) -> anyhow::Result<Arc<HostService>> {
    let id = Identity::load_or_create(&store(dir));
    // 以最高权限运行时，文件操作交给以目标用户身份启动的子进程
    let helper = if galaxy_host::sysuser::is_root() { std::env::current_exe().ok() } else { None };
    let cfg = HostConfig { server, data_dir: dir.to_path_buf(), meta: meta(os.as_ref(), DeviceKind::Worker), multi_account: true, file_helper: helper };
    let host = HostService::start(cfg, id).await?;
    if let Some(p) = platform_for(&*os) {
        host.set_platform(p);
    }
    tracing::info!("LemonGalaxy worker {VERSION} 已启动，设备 {}", host.identity.device_id);
    Ok(host)
}

/// 还没登录时也写 status.json，命令行 status 据此知道服务在运行。
fn write_waiting_status(dir: &Path, device_id: &str) {
    let st = HostState::load(&dir.join("host.json"));
    let s = galaxy_host::HostStatus { online: false, server: st.server.clone(), device_id: device_id.into(), accounts: vec![], sessions: vec![], assist: None, remote_enabled: st.remote_enabled, public_ip: String::new() };
    if let Ok(b) = serde_json::to_vec_pretty(&s) {
        let p = dir.join("status.json");
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, b).is_ok() {
            let _ = std::fs::rename(tmp, p);
        }
    }
}

/// 没有本机控制接口的系统。
#[cfg(not(any(unix, windows)))]
struct NoControl;
#[cfg(not(any(unix, windows)))]
impl NoControl {
    fn set_host(&self, _h: Option<Arc<HostService>>) {}
    async fn woken(&self) {
        std::future::pending::<()>().await
    }
    fn close(&self) {}
    fn assist_busy(&self) -> bool {
        false
    }
}

pub async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// 临时协助：用一次性的匿名设备身份前台运行，在终端里确认请求。
async fn assist(os: &dyn WorkerOs, rest: &[String]) -> i32 {
    let server_s = flag(rest, "--server").unwrap_or_else(|| prompt(t!("服务器地址", "Server address"), None));
    let server = match ServerAddr::parse(&server_s) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let tmp = std::env::temp_dir().join(format!("lemon-galaxy-assist-{}", std::process::id()));
    let id = Identity::load_or_create(&store(&tmp));
    let cfg = HostConfig { server, data_dir: tmp.clone(), meta: meta(os, DeviceKind::Worker), multi_account: false, file_helper: None };
    let host = match HostService::start(cfg, id).await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    struct Ask;
    impl galaxy_host::HostEvents for Ask {
        fn ask_assist(&self, from_user: String, _from_device: String) -> futures::future::BoxFuture<'static, bool> {
            Box::pin(async move {
                tokio::task::spawn_blocking(move || {
                    let a = prompt(&format!("{} {from_user} {} (yes/no)", t!("收到协助请求：", "Assistance request from"), t!("允许对方控制本机终端和文件？", "allow?")), Some("no"));
                    a == "yes" || a == "y"
                })
                .await
                .unwrap_or(false)
            })
        }
        fn on_session_started(&self, s: &galaxy_host::service::SessionInfo) {
            println!("{} {}", t!("对方已连入：", "Connected:"), s.from_user);
        }
        fn on_session_ended(&self, _id: &str) {
            println!("{}", t!("对方已断开。", "Disconnected."));
        }
    }
    host.set_events(Arc::new(Ask));
    if let Some(p) = platform_for(&*os) {
        host.set_platform(p);
    }
    let code = host.start_assist();
    let mut shown = false;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if let Some((dc, c)) = host.status().assist {
            println!("\n{}: {}", t!("设备码", "Device code"), dc);
            println!("{}: {}\n", t!("验证码", "Verification code"), c);
            println!("{}", t!("把这两个码告诉协助你的人。按 Ctrl+C 结束协助。", "Tell both codes to your helper. Press Ctrl+C to stop."));
            shown = true;
            break;
        }
    }
    if !shown {
        eprintln!("{} ({code})", t!("连不上服务器", "Cannot reach server"));
        return 1;
    }
    shutdown_signal().await;
    let _ = std::fs::remove_dir_all(tmp);
    0
}

// ---------------- 更新 ----------------

pub(crate) fn platform_name(os: &dyn WorkerOs) -> &'static str {
    match os.platform() {
        // XP 兼容版（构建时加 --cfg lemon_galaxy_xp）、Windows 7 兼容版单独发布，不能更新成 Windows 10 起的版本
        Platform::Windows if cfg!(lemon_galaxy_xp) => "windowsxp",
        Platform::Windows if cfg!(target_vendor = "win7") => "windows7",
        Platform::Windows => "windows",
        Platform::Macos => "macos",
        // musl 静态精简版（Alpine 等）单独发布，不能更新成 glibc 版
        _ if cfg!(target_env = "musl") => "linux-musl",
        _ => "linux",
    }
}

/// 用本机任一有效账号的令牌访问服务器。
pub(crate) fn api_for(dir: &Path) -> Result<Api, String> {
    let st = HostState::load(&dir.join("host.json"));
    let server = ServerAddr::parse(&st.server).map_err(|_| t!("还没有登录", "Not signed in").to_string())?;
    let acct = st.accounts.iter().find(|a| a.status == galaxy_host::AccountStatus::Active).ok_or_else(|| t!("没有可用的账号", "No active account").to_string())?;
    let api = Api::new(server, if zh() { "zh-CN" } else { "en" }).map_err(|e| e.to_string())?;
    api.set_token(Some(acct.token.clone()));
    Ok(api)
}

pub(crate) async fn check_update(os: &dyn WorkerOs, dir: &Path) -> Result<Option<galaxy_client::update::UpdateInfo>, String> {
    upgrade::check(os, dir).await.map(|f| f.map(|f| f.info))
}

/// 换好新版本后退出，由服务管理器拉起新版本（退出码 75：Windows 服务管理器按「意外退出」处理、执行恢复操作；
/// systemd 的 Restart=always 照常拉起）。
pub(crate) fn exit_for_update(dir: &Path) -> ! {
    let _ = std::fs::remove_file(dir.join("status.json"));
    std::process::exit(upgrade::EXIT_RESTART);
}

/// 命令行 `update`：服务在运行时交给服务做（与界面「立即更新」同一条路：整套换、自己重启、出问题自动换回）；
/// 服务没在运行时只在本机换程序，重启由用户自己来。
async fn update_cmd(os: Arc<dyn WorkerOs>, dir: &Path, rest: &[String]) -> i32 {
    if rest.iter().any(|a| a == "--check") {
        return match check_update(os.as_ref(), dir).await {
            Ok(Some(i)) => {
                println!("{} {}（{} {}）", t!("有新版本", "New version"), i.version, t!("当前", "current"), VERSION);
                0
            }
            Ok(None) => {
                println!("{} {VERSION}", t!("已是最新版本", "Up to date:"));
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        };
    }
    let d = dir.to_path_buf();
    match tokio::task::spawn_blocking(move || ask_service_update(&d)).await.ok().flatten() {
        Some(Ok((Some(v), restarting))) => {
            println!("{} {v}", t!("已更新到", "Updated to"));
            println!("{}", if restarting { t!("服务正在重启成新版本；出问题会自动换回旧版本。", "The service is restarting into the new version; it rolls back automatically on problems.") } else { t!("重启服务后生效。", "Restart the service to use it.") });
            return 0;
        }
        Some(Ok((None, _))) => {
            println!("{} {VERSION}", t!("已是最新版本", "Up to date:"));
            return 0;
        }
        Some(Err(e)) => {
            eprintln!("{e}");
            return 1;
        }
        None => {}
    }
    match upgrade::update_now(os.clone(), dir, false).await {
        Ok(Some(a)) => {
            println!("{} {}", t!("已更新到", "Updated to"), a.version);
            if os.restart_service() {
                println!("{}", t!("已重启服务。", "Service restarted."));
            } else {
                println!("{}", t!("如果装了服务，请重启它让新版本生效。", "Restart the service to use the new version."));
            }
            0
        }
        Ok(None) => {
            println!("{} {VERSION}", t!("已是最新版本", "Up to date:"));
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// 经本机控制接口让正在运行的服务更新（ApplyUpdate）。连不上服务返回 None；返回 (新版本, 服务是否马上重启)。
fn ask_service_update(dir: &Path) -> Option<Result<(Option<String>, bool), String>> {
    use crate::control::{encode, ErrorCode, Request, RequestLine, Response, ServerLine};
    use std::io::BufRead;
    let lang = if zh() { "zh-CN" } else { "en" };
    #[cfg(unix)]
    let (reader, mut writer): (Box<dyn std::io::Read>, Box<dyn std::io::Write>) = {
        let mut cands = vec![];
        if let Some(p) = std::env::var_os("LEMON_GALAXY_CONTROL_SOCKET").filter(|v| !v.is_empty()) {
            cands.push(PathBuf::from(p));
        } else {
            if galaxy_host::sysuser::is_root() {
                cands.push(control::system_socket_path());
            }
            cands.push(control::user_socket_path(dir));
            cands.push(dir.join("control.sock"));
        }
        let s = cands.iter().find_map(|p| std::os::unix::net::UnixStream::connect(p).ok())?;
        let _ = s.set_read_timeout(Some(Duration::from_secs(3600)));
        (Box::new(s.try_clone().ok()?), Box::new(s))
    };
    #[cfg(windows)]
    let (reader, mut writer): (Box<dyn std::io::Read>, Box<dyn std::io::Write>) = {
        let _ = dir;
        let mut cands = vec![];
        if let Some(p) = std::env::var_os("LEMON_GALAXY_CONTROL_SOCKET").filter(|v| !v.is_empty()) {
            cands.push(p.to_string_lossy().to_string());
        } else {
            cands.push(control::SYSTEM_PIPE.to_string());
            cands.push(control::user_pipe_name(&galaxy_host::sysuser::SysUser::current().sid));
        }
        let f = cands.iter().find_map(|p| std::fs::OpenOptions::new().read(true).write(true).open(p).ok())?;
        (Box::new(f.try_clone().ok()?), Box::new(f))
    };
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (dir, lang);
        return None;
    }
    #[cfg(any(unix, windows))]
    {
        let send = |w: &mut Box<dyn std::io::Write>, id: u64, req: Request| w.write_all(encode(&RequestLine { id, req }).as_bytes()).and_then(|_| w.flush());
        if send(&mut writer, 1, Request::Hello { lang: lang.into() }).is_err() || send(&mut writer, 2, Request::ApplyUpdate).is_err() {
            return None;
        }
        println!("{}", t!("正在由服务下载并安装新版本……", "The service is downloading and installing the update..."));
        for line in std::io::BufReader::new(reader).lines() {
            let Ok(line) = line else { break };
            let Ok(ServerLine::Response(r)) = serde_json::from_str::<ServerLine>(&line) else { continue };
            if r.id != 2 {
                continue;
            }
            return Some(match r.resp {
                Response::Updated { version, restarting } => Ok((version, restarting)),
                Response::Error { code: ErrorCode::Denied, message } => Err(format!("{message}（{}）", t!("请以管理员 / root 身份运行", "run as administrator / root"))),
                Response::Error { message, .. } => Err(message),
                other => Err(format!("{other:?}")),
            });
        }
        Some(Err(t!("服务没有回复（可能正在重启）", "No reply from the service (it may be restarting)").into()))
    }
}

fn lock_on_end_cmd(dir: &Path, rest: &[String]) -> i32 {
    let path = dir.join("host.json");
    let mut st = HostState::load(&path);
    match rest.first().map(|s| s.as_str()) {
        Some("on") => st.lock_on_end = true,
        Some("off") => st.lock_on_end = false,
        _ => {
            println!("{}: {}", t!("远程结束后自动锁屏", "Lock when remote session ends"), onoff(st.lock_on_end));
            return 0;
        }
    }
    let _ = st.save(&path);
    println!("{}: {}", t!("远程结束后自动锁屏", "Lock when remote session ends"), onoff(st.lock_on_end));
    0
}

fn headless_display_cmd(dir: &Path, rest: &[String]) -> i32 {
    let path = dir.join("host.json");
    let mut st = HostState::load(&path);
    let show = |v: &str| match galaxy_host::accounts::parse_headless_display(v) {
        Some((w, h, hz)) => format!("{w}x{h}@{hz}"),
        None => t!("关", "off").to_string(),
    };
    match rest.first().map(|s| s.as_str()) {
        None => {
            println!("{}: {}", t!("无头兜底屏", "Headless display"), show(&st.headless_display));
            return 0;
        }
        Some("default") => st.headless_display.clear(),
        Some(v) => {
            let off = matches!(v.to_ascii_lowercase().as_str(), "off" | "no" | "false" | "0");
            if !off && galaxy_host::accounts::parse_headless_display(v).is_none() {
                eprintln!("{} {v}（{}）", t!("写法不对：", "Invalid value:"), t!("off、default 或 宽x高[@刷新率]，如 2560x1440@60", "off, default or WxH[@Hz], e.g. 2560x1440@60"));
                return 1;
            }
            st.headless_display = if off { "off".into() } else { v.to_string() };
        }
    }
    let _ = st.save(&path);
    println!("{}: {}", t!("无头兜底屏", "Headless display"), show(&st.headless_display));
    0
}

fn auto_update_cmd(dir: &Path, rest: &[String]) -> i32 {
    let path = dir.join("host.json");
    let mut st = HostState::load(&path);
    match rest.first().map(|s| s.as_str()) {
        Some("on") => st.auto_update = true,
        Some("off") => st.auto_update = false,
        _ => {
            println!("{}: {}", t!("自动更新", "Auto update"), onoff(st.auto_update));
            return 0;
        }
    }
    let _ = st.save(&path);
    println!("{}: {}", t!("自动更新", "Auto update"), onoff(st.auto_update));
    0
}
