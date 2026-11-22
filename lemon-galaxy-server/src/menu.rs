//! 交互菜单：在终端里不带任何参数运行 lemon-galaxy-server 时出现，一项一项选，不用记命令。
//!
//! - 只在标准输入输出都是终端时出现（Windows 双击打开的控制台也算）；脚本、systemd、launchd、Docker 里不带参数照常前台运行。
//! - 装服务、启停、卸载、升级都调用同名子命令（子进程，输出与命令行一致）；需要 root 时经 sudo，
//!   Windows 上需要管理员时以管理员身份重新打开菜单。
//! - 菜单文字跟随系统语言（中文 / 英文），也可用环境变量 LEMON_GALAXY_LANG=zh|en 指定；子命令的输出仍是中文。

use crate::service::{self, SvcState};
use crate::tui::{self, bold, dim, green, red, yellow, Item, Quit, Tui};
use crate::{AdminCmd, Cli, ServeCtx, UserCmd};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

// ---------------- 语言 ----------------

static ZH: OnceLock<bool> = OnceLock::new();

/// 菜单用中文还是英文。
pub fn zh() -> bool {
    *ZH.get_or_init(detect_zh)
}

macro_rules! t {
    ($zh:expr, $en:expr) => {
        if $crate::menu::zh() {
            $zh
        } else {
            $en
        }
    };
}

macro_rules! tf {
    ($zh:literal, $en:literal $(, $a:expr)* $(,)?) => {
        if $crate::menu::zh() { format!($zh $(, $a)*) } else { format!($en $(, $a)*) }
    };
}

/// 环境变量 LEMON_GALAXY_LANG > LC_ALL / LC_MESSAGES / LANG > 系统界面语言；都看不出来时用中文（与其他命令行输出一致）。
fn detect_zh() -> bool {
    if let Ok(v) = std::env::var("LEMON_GALAXY_LANG") {
        if !v.trim().is_empty() {
            return v.trim().to_ascii_lowercase().starts_with("zh");
        }
    }
    for k in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        let v = std::env::var(k).unwrap_or_default().to_ascii_lowercase();
        if v.is_empty() {
            continue;
        }
        if v == "c" || v == "posix" || v.starts_with("c.") {
            break;
        }
        return v.starts_with("zh");
    }
    os_zh().unwrap_or(true)
}

#[cfg(target_os = "macos")]
fn os_zh() -> Option<bool> {
    let o = Command::new("defaults").args(["read", "-g", "AppleLanguages"]).stderr(std::process::Stdio::null()).output().ok()?;
    let s = String::from_utf8_lossy(&o.stdout);
    let first = s.lines().map(|l| l.trim().trim_matches(|c| c == '"' || c == ',' || c == '(' || c == ')').trim()).find(|l| !l.is_empty())?;
    Some(first.to_ascii_lowercase().starts_with("zh"))
}

#[cfg(windows)]
fn os_zh() -> Option<bool> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetUserDefaultUILanguage() -> u16;
    }
    // 主语言号 0x04 是中文
    Some(unsafe { GetUserDefaultUILanguage() } & 0x3ff == 0x04)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn os_zh() -> Option<bool> {
    None
}

// ---------------- 入口 ----------------

/// 是否进菜单：标准输入输出都是终端，且不在容器里（容器里不带参数就是要运行服务器）。
pub fn available() -> bool {
    use std::io::IsTerminal;
    let container = cfg!(target_os = "linux") && (Path::new("/.dockerenv").exists() || Path::new("/run/.containerenv").exists());
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal() && !container
}

pub fn run(cli: &Cli) -> anyhow::Result<()> {
    let m = Menu {
        t: Tui {
            keys_help: t!("↑↓ 选择 · 回车确认 · 数字直达 · Esc 返回", "Up/Down select · Enter confirm · number jumps · Esc back").into(),
            keys_help_top: t!("↑↓ 选择 · 回车确认 · 数字直达 · q 退出", "Up/Down select · Enter confirm · number jumps · q quit").into(),
            input_help: t!("回车确认 · Esc 取消", "Enter confirm · Esc cancel").into(),
            pause_text: t!("按回车返回菜单", "Press Enter to go back").into(),
            yes: t!("是", "Yes").into(),
            no: t!("否", "No").into(),
            plain_prompt: t!("输入编号回车（直接回车返回）：", "Enter a number (empty to go back): ").into(),
        },
        cli,
    };
    println!();
    println!("{}", bold(&tf!("LemonGalaxy 服务器 {}", "LemonGalaxy Server {}", galaxy_proto::APP_VERSION)));
    match m.main_menu() {
        Ok(Next::Quit) => Ok(()),
        Ok(Next::Foreground(ctx)) => {
            crate::init_logging(true, m.cli.log_file.as_deref())?;
            crate::serve_blocking(ctx, crate::shutdown_signal())
        }
        Err(e) if tui::is_quit(&e) => Ok(()),
        Err(e) => Err(e),
    }
}

enum Next {
    Quit,
    Foreground(ServeCtx),
}

struct Menu<'a> {
    t: Tui,
    cli: &'a Cli,
}

// ---------------- 状态 ----------------

#[derive(PartialEq)]
enum Access {
    Ok,
    Denied,
    Missing,
}

struct Snap {
    ctx: ServeCtx,
    from_service: bool,
    svc: Option<SvcState>,
    autostart: Option<bool>,
    cfg: Option<service::SvcConfig>,
    access: Access,
    run: Option<service::RunInfo>,
    admins: Option<i64>,
    users: Option<i64>,
    /// 数据库设置里的对外地址
    settings_host: Option<String>,
    repo: Option<String>,
}

fn data_access(dir: &Path) -> Access {
    match std::fs::read_dir(dir) {
        Ok(_) => Access::Ok,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Access::Denied,
        Err(_) => Access::Missing,
    }
}

impl Snap {
    fn load(cli: &Cli) -> Snap {
        let (ctx, from_service) = crate::resolve_manage_ctx(cli);
        let access = data_access(&ctx.data_dir);
        let run = service::read_run_info(&ctx.data_dir);
        let (mut admins, mut users, mut settings_host, mut repo) = (None, None, None, None);
        if access == Access::Ok && ctx.data_dir.join("server.db").exists() {
            if let Ok((db, _, s)) = crate::open_data(&ctx.data_dir, None) {
                admins = db.call_sync(|c| c.query_row("SELECT COUNT(*) FROM admins", [], |r| r.get::<_, i64>(0))).ok();
                users = db.call_sync(|c| c.query_row("SELECT COUNT(*) FROM users", [], |r| r.get::<_, i64>(0))).ok();
                settings_host = Some(s.public_host).filter(|h| !h.is_empty());
                repo = Some(s.update_github_repo);
            }
        }
        Snap { ctx, from_service, svc: service::query(), autostart: service::autostart(), cfg: service::load_config(), access, run, admins, users, settings_host, repo }
    }

    fn running(&self) -> bool {
        matches!(self.svc, Some(SvcState::Running(_)) | Some(SvcState::Starting))
    }

    fn public_host(&self) -> Option<String> {
        self.ctx.public_host.clone().filter(|h| !h.is_empty()).or_else(|| self.settings_host.clone())
    }

    /// 需要提升权限才能看全 / 操作：数据目录读不了，或装了服务。
    fn wants_elevation(&self) -> bool {
        !service::is_admin() && (self.access == Access::Denied || self.svc.is_some())
    }

    fn summary(&self) -> String {
        let mut rows: Vec<(String, String)> = Vec::new();
        let svc = match self.svc {
            None => format!("{} {}", dim("○"), t!("未安装", "not installed")),
            Some(st) => {
                let dot = if matches!(st, SvcState::Running(_)) { green("●") } else if st == SvcState::Failed { red("●") } else { dim("○") };
                let mut s = format!("{dot} {}", state_text(st));
                match self.autostart {
                    Some(true) => s.push_str(&dim(t!(" · 开机自启", " · starts at boot"))),
                    Some(false) => s.push_str(&dim(t!(" · 开机不自启", " · not started at boot"))),
                    None => {}
                }
                s
            }
        };
        rows.push((t!("系统服务", "Service").into(), svc));
        let mut dir = self.ctx.data_dir.display().to_string();
        if self.from_service {
            dir.push_str(&dim(t!("（系统服务）", " (service)")));
        } else if self.ctx.data_dir == Path::new(crate::LOCAL_DATA_DIR) {
            dir = std::path::absolute(&self.ctx.data_dir).unwrap_or_else(|_| self.ctx.data_dir.clone()).display().to_string();
            dir.push_str(&dim(t!("（前台运行）", " (foreground)")));
        }
        rows.push((t!("数据目录", "Data dir").into(), dir));
        let host = match self.public_host() {
            Some(h) => h,
            None => yellow(t!("未设置（只有本机能连）", "not set (local only)")),
        };
        rows.push((t!("端口", "Port").into(), format!("{}   {} {host}", self.ctx.port, dim(t!("对外地址", "public host")))));
        match (&self.run, &self.access) {
            (_, Access::Denied) => rows.push((t!("客户端地址", "Client address").into(), yellow(t!("需要 root / 管理员权限才能读取", "needs root / administrator to read")))),
            (Some(r), _) => {
                rows.push((t!("客户端地址", "Client address").into(), r.address.clone()));
                if let Some(u) = r.setup_url.as_ref().filter(|_| self.admins.unwrap_or(0) == 0) {
                    rows.push((t!("初始化网址", "Setup URL").into(), u.clone()));
                }
            }
            (None, _) => rows.push((t!("客户端地址", "Client address").into(), dim(t!("服务器还没有运行过", "server has not run yet")))),
        }
        if let (Some(a), Some(u)) = (self.admins, self.users) {
            let s = if a == 0 { yellow(t!("还没有管理员", "none yet")) } else { tf!("{a} 个", "{a}") };
            rows.push((t!("管理员", "Admins").into(), format!("{s}   {} {}", dim(t!("用户", "users")), tf!("{u} 个", "{u}"))));
        }
        let w = rows.iter().map(|(k, _)| tui::display_width(k)).max().unwrap_or(0);
        let mut out: Vec<String> = rows.into_iter().map(|(k, v)| format!("  {}  {v}", dim(&pad(&k, w)))).collect();
        if self.wants_elevation() {
            out.push(format!("  {}", yellow(&elevate_note())));
        }
        out.join("\n")
    }
}

fn pad(s: &str, w: usize) -> String {
    format!("{s}{}", " ".repeat(w.saturating_sub(tui::display_width(s))))
}

fn state_text(st: SvcState) -> String {
    match st {
        SvcState::Running(Some(pid)) => tf!("运行中（进程 {pid}）", "running (pid {pid})"),
        SvcState::Running(None) => t!("运行中", "running").into(),
        SvcState::Starting => t!("正在启动", "starting").into(),
        SvcState::Stopping => t!("正在停止", "stopping").into(),
        SvcState::Stopped => t!("已停止", "stopped").into(),
        SvcState::Failed => t!("启动失败", "failed").into(),
        SvcState::Paused => t!("已暂停", "paused").into(),
    }
}

fn elevate_note() -> String {
    if cfg!(windows) {
        t!("当前不是管理员：管理服务、读系统服务的数据需要「以管理员身份重新打开」", "Not elevated: managing the service needs \"Reopen as administrator\"").into()
    } else {
        t!("当前不是 root：管理服务、读系统服务的数据需要 sudo", "Not root: managing the service needs sudo").into()
    }
}

// ---------------- 菜单 ----------------

impl Menu<'_> {
    fn main_menu(&self) -> anyhow::Result<Next> {
        let mut sel = 0;
        loop {
            let s = Snap::load(self.cli);
            let svc_hint = match s.svc {
                None => t!("未安装 · 可装成开机自启的服务", "not installed · install to start at boot").to_string(),
                Some(st) => state_text(st),
            };
            let mut items = vec![
                Item::new("service", t!("系统服务", "System service"), svc_hint),
                Item::new("run", t!("运行服务器（前台）", "Run server in foreground"), t!("在这个窗口里运行，Ctrl+C 停止", "runs in this window, Ctrl+C stops")),
                Item::new("status", t!("查看服务状态与最近日志", "Status and recent logs"), ""),
                Item::new("address", t!("客户端地址与初始化网址", "Client address and setup URL"), t!("客户端登录时填的地址", "what clients enter to connect")),
                Item::new("admins", t!("管理员", "Administrators"), s.admins.map(|n| tf!("{n} 个 · 创建、重置密码", "{n} · create, reset password")).unwrap_or_else(|| t!("创建、重置密码", "create, reset password").into())),
                Item::new("users", t!("用户", "Users"), s.users.map(|n| tf!("{n} 个 · 创建、列出", "{n} · create, list")).unwrap_or_else(|| t!("创建、列出", "create, list").into())),
                Item::new("update", t!("检查更新与升级", "Check for updates"), tf!("当前版本 {}", "current {}", galaxy_proto::APP_VERSION)),
            ];
            if s.wants_elevation() {
                items.push(Item::new("elevate", elevate_label(), ""));
            }
            items.push(Item::new("quit", t!("退出", "Quit"), ""));
            let title = format!("\n{}\n\n{}", s.summary(), bold(t!("要做什么？", "What would you like to do?")));
            let Some(k) = self.t.choose(&title, &items, sel, true)? else { return Ok(Next::Quit) };
            sel = items.iter().position(|i| i.key == k).unwrap_or(0);
            let r = match k.as_str() {
                "quit" => return Ok(Next::Quit),
                "run" => match self.foreground(&s) {
                    Ok(Some(ctx)) => return Ok(Next::Foreground(ctx)),
                    Ok(None) => Ok(()),
                    Err(e) => Err(e),
                },
                "service" => self.service_menu(),
                "status" => self.show_status(),
                "address" => self.show_address(),
                "admins" => self.admins_menu(),
                "users" => self.users_menu(),
                "update" => self.update(&s),
                "elevate" => Err(self.elevate()),
                _ => Ok(()),
            };
            self.after(r)?;
        }
    }

    /// 操作结束：Ctrl+C 往上传，其他错误显示出来等用户看完。
    fn after(&self, r: anyhow::Result<()>) -> anyhow::Result<()> {
        match r {
            Ok(()) => Ok(()),
            Err(e) if tui::is_quit(&e) => Err(e),
            Err(e) => {
                println!("{}", red(&tf!("  出错：{e:#}", "  Error: {e:#}")));
                self.t.pause()
            }
        }
    }

    fn service_menu(&self) -> anyhow::Result<()> {
        loop {
            let s = Snap::load(self.cli);
            let mut items = Vec::new();
            match s.svc {
                None => items.push(Item::new(
                    "install",
                    t!("安装为系统服务并开机自启", "Install as a system service (start at boot)"),
                    t!("会问端口、对外地址、证书", "asks for port, public host, certificate"),
                )),
                Some(_) => {
                    if s.running() {
                        items.push(Item::new("restart", t!("重启服务", "Restart service"), ""));
                        items.push(Item::new("stop", t!("停止服务", "Stop service"), t!("开机时仍会自动启动", "still starts at next boot")));
                    } else {
                        items.push(Item::new("start", t!("启动服务", "Start service"), ""));
                    }
                    items.push(Item::new("config", t!("修改服务参数", "Change service settings"), t!("端口、对外地址、TLS 证书", "port, public host, TLS certificate")));
                    items.push(Item::new("status", t!("查看服务状态与最近日志", "Status and recent logs"), ""));
                    items.push(Item::new("uninstall", t!("卸载服务", "Uninstall service"), t!("数据目录保留", "data directory is kept")));
                }
            }
            let title = format!("{}\n\n{}", s.summary(), bold(&tf!("系统服务 {}", "System service {}", service::service_name())));
            let Some(k) = self.t.choose(&title, &items, 0, false)? else { return Ok(()) };
            let r = match k.as_str() {
                "install" => self.install(&s, false),
                "config" => self.install(&s, true),
                "start" => self.run_self(&["start-service"], true),
                "restart" => self.run_self(&["restart-service"], true),
                "stop" => {
                    if self.t.confirm(t!("停止服务？客户端会断开，开机时仍会自动启动。", "Stop the service? Clients will disconnect; it still starts at next boot."))? {
                        self.run_self(&["stop-service"], true)
                    } else {
                        continue;
                    }
                }
                "uninstall" => {
                    if self.t.confirm(t!("卸载系统服务？服务器会停止、不再开机自启，数据目录保留。", "Uninstall the service? The server stops and no longer starts at boot; data is kept."))? {
                        self.run_self(&["uninstall-service"], true)
                    } else {
                        continue;
                    }
                }
                "status" => self.show_status(),
                _ => Ok(()),
            };
            self.after(r)?;
        }
    }

    /// 安装或修改服务参数：问端口、对外地址、证书，拼成 install-service 的参数。
    fn install(&self, s: &Snap, modify: bool) -> anyhow::Result<()> {
        let cfg = s.cfg.clone();
        let port_def = cfg.as_ref().map(|c| c.port).unwrap_or(galaxy_proto::DEFAULT_PORT).to_string();
        let port = loop {
            let Some(v) = self.t.input(t!("端口（TCP 与 UDP 同号）", "Port (TCP and UDP)"), &port_def, "", false)? else { return Ok(()) };
            match v.parse::<u16>() {
                Ok(p) if p != 0 => break p,
                _ => println!("{}", red(t!("  端口要是 1 到 65535 之间的数字", "  Port must be a number from 1 to 65535"))),
            }
        };
        let host_def = cfg.as_ref().and_then(|c| c.public_host.clone()).or_else(|| s.settings_host.clone()).unwrap_or_default();
        let Some(host) = self.t.input(
            t!("对外地址", "Public host"),
            &host_def,
            t!("客户端用来连接的域名或 IP（写进客户端地址与自签证书）；留空则只有本机能连", "Domain or IP clients connect to (goes into the client address and self-signed cert); empty = local only"),
            false,
        )?
        else {
            return Ok(());
        };
        let cert_def = cfg.as_ref().and_then(|c| c.tls_cert.as_ref()).map(|p| p.display().to_string()).unwrap_or_default();
        let Some(cert) = self.t.input(t!("TLS 证书文件", "TLS certificate"), &cert_def, t!("正规证书（PEM）的路径；留空使用自动生成的自签证书", "Path to a PEM certificate; empty = auto-generated self-signed certificate"), false)? else {
            return Ok(());
        };
        let mut key = String::new();
        if !cert.is_empty() {
            let key_def = cfg.as_ref().and_then(|c| c.tls_key.as_ref()).map(|p| p.display().to_string()).unwrap_or_default();
            let Some(k) = self.t.input(t!("TLS 私钥文件", "TLS private key"), &key_def, "", false)? else { return Ok(()) };
            anyhow::ensure!(!k.is_empty(), "{}", t!("证书与私钥要一起给", "Certificate and key must be given together"));
            key = k;
        }
        let mut args: Vec<String> = vec!["install-service".into(), "--port".into(), port.to_string(), "--public-host".into(), host.clone()];
        if !cert.is_empty() {
            args.extend(["--tls-cert".into(), cert.clone(), "--tls-key".into(), key]);
        } else if cfg.as_ref().map(|c| c.tls_cert.is_some()).unwrap_or(false) {
            // 空值表示清除，改回自签证书
            args.extend(["--tls-cert".into(), String::new(), "--tls-key".into(), String::new()]);
        }
        // 第一次安装，当前目录下有前台运行留下的数据：问要不要沿用
        let local = Path::new(crate::LOCAL_DATA_DIR);
        if !modify && cfg.is_none() && local.join("server.db").exists() {
            let abs = std::path::absolute(local).unwrap_or_else(|_| local.to_path_buf());
            if self.t.confirm(&tf!("当前目录下有前台运行留下的数据 {}，服务沿用这份数据（账号、设备）？", "Found data from a foreground run at {}. Use it for the service (accounts, devices)?", abs.display()))? {
                args.extend(["--data-dir".into(), abs.display().to_string()]);
            }
        }
        println!();
        println!("  {} {port}", dim(t!("端口", "Port")));
        println!("  {} {}", dim(t!("对外地址", "Public host")), if host.is_empty() { t!("（未设置，只有本机能连）", "(not set, local only)").to_string() } else { host });
        println!("  {} {}", dim(t!("TLS 证书", "TLS certificate")), if cert.is_empty() { t!("自签证书", "self-signed").to_string() } else { cert });
        let q = if modify { t!("用这些参数重新安装服务（服务会重启）？", "Reinstall the service with these settings (it will restart)?") } else { t!("安装为系统服务并启动？", "Install the system service and start it?") };
        if !self.t.confirm(q)? {
            return Ok(());
        }
        let refs: Vec<&str> = args.iter().map(|a| a.as_str()).collect();
        self.run_self(&refs, true)
    }

    /// 运行自己的子命令（输出直接显示），结束后等用户按回车。admin 为真时需要 root / 管理员。
    fn run_self(&self, args: &[&str], admin: bool) -> anyhow::Result<()> {
        let exe = std::env::current_exe()?;
        let mut cmd = if admin && !service::is_admin() {
            match self.elevated_cmd(&exe, args)? {
                Some(c) => c,
                None => return Ok(()),
            }
        } else {
            let mut c = Command::new(&exe);
            c.args(args);
            c
        };
        let shown: Vec<String> = std::iter::once(cmd.get_program().to_string_lossy().into_owned()).chain(cmd.get_args().map(|a| shell_quote(&a.to_string_lossy()))).collect();
        println!("{}", dim(&format!("  $ {}", shown.join(" "))));
        match cmd.status() {
            Ok(st) if st.success() => {}
            Ok(st) => println!("{}", red(&tf!("  命令没有成功（退出码 {}）", "  Command failed (exit code {})", st.code().unwrap_or(-1)))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !cfg!(windows) => {
                println!("{}", red(&tf!("  找不到 sudo，请切换到 root 后运行：{} {}", "  sudo not found; run as root: {} {}", exe.display(), args.join(" "))));
            }
            Err(e) => return Err(e.into()),
        }
        self.t.pause()
    }

    /// 需要 root 时经 sudo 运行子命令。
    #[cfg(not(windows))]
    fn elevated_cmd(&self, exe: &Path, args: &[&str]) -> anyhow::Result<Option<Command>> {
        let mut c = Command::new("sudo");
        c.arg(exe).args(args);
        Ok(Some(c))
    }

    /// Windows 上不是管理员：提示以管理员身份重新打开菜单（返回 None 表示回菜单）。
    #[cfg(windows)]
    fn elevated_cmd(&self, exe: &Path, args: &[&str]) -> anyhow::Result<Option<Command>> {
        println!("{}", yellow(t!("  这个操作需要管理员权限。", "  This needs administrator rights.")));
        if self.t.confirm(t!("以管理员身份重新打开菜单？", "Reopen this menu as administrator?"))? {
            return Err(self.elevate());
        }
        println!("{}", dim(&tf!("  也可以右键「以管理员身份运行」PowerShell，再运行：{} {}", "  Or run as administrator: {} {}", exe.display(), args.join(" "))));
        self.t.pause()?;
        Ok(None)
    }

    /// 以管理员身份在新窗口重新打开菜单。返回 Quit，让当前菜单退出。
    #[cfg(windows)]
    fn elevate(&self) -> anyhow::Error {
        let r = std::env::current_exe().map_err(anyhow::Error::from).and_then(|exe| shell_execute_runas(&exe));
        match r {
            Ok(()) => {
                println!("{}", dim(t!("  已在新窗口以管理员身份打开菜单。", "  Opened the menu as administrator in a new window.")));
                Quit.into()
            }
            Err(e) => e,
        }
    }

    /// 经 sudo 以 root 重新打开菜单，它退出后当前菜单也退出（返回 Quit）。
    #[cfg(not(windows))]
    fn elevate(&self) -> anyhow::Error {
        let exe = match std::env::current_exe() {
            Ok(e) => e,
            Err(e) => return e.into(),
        };
        // sudo 默认会清掉环境变量，菜单语言单独带过去
        let lang = if zh() { "LEMON_GALAXY_LANG=zh" } else { "LEMON_GALAXY_LANG=en" };
        println!("{}", dim(&format!("  $ sudo env {} {}", lang, shell_quote(&exe.to_string_lossy()))));
        match Command::new("sudo").arg("env").arg(lang).arg(&exe).status() {
            Ok(_) => Quit.into(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => anyhow::anyhow!("{}", t!("找不到 sudo，请切换到 root 后再运行", "sudo not found; switch to root and run again")),
            Err(e) => e.into(),
        }
    }

    /// 读不了数据目录时提示提升权限。返回 Ok(()) 表示用户不提升（回菜单）。
    fn offer_elevate(&self, dir: &Path) -> anyhow::Result<()> {
        println!("{}", yellow(&tf!("  没有权限读取数据目录 {}。", "  No permission to read the data directory {}.", dir.display())));
        if self.t.confirm(&elevate_label())? {
            return Err(self.elevate());
        }
        Ok(())
    }

    fn foreground(&self, s: &Snap) -> anyhow::Result<Option<ServeCtx>> {
        let mut ctx = crate::serve_ctx(self.cli, PathBuf::from(crate::LOCAL_DATA_DIR));
        let svc_port = s.cfg.as_ref().map(|c| c.port).filter(|_| s.running());
        if svc_port == Some(ctx.port) {
            println!("{}", yellow(&tf!("  系统服务正在使用端口 {}，前台运行换一个端口。", "  The system service is using port {}; pick another port.", ctx.port)));
            let def = (ctx.port.wrapping_add(1).max(1)).to_string();
            let Some(v) = self.t.input(t!("端口", "Port"), &def, "", false)? else { return Ok(None) };
            ctx.port = v.parse().ok().filter(|p| *p != 0).ok_or_else(|| anyhow::anyhow!("{}", t!("端口不对", "Invalid port")))?;
        }
        let abs = std::path::absolute(&ctx.data_dir).unwrap_or_else(|_| ctx.data_dir.clone());
        println!("{}", dim(&tf!("  数据目录 {}；按 Ctrl+C 或关闭窗口停止。", "  Data directory {}; press Ctrl+C or close the window to stop.", abs.display())));
        println!();
        Ok(Some(ctx))
    }

    fn show_status(&self) -> anyhow::Result<()> {
        let (ctx, _) = crate::resolve_manage_ctx(self.cli);
        println!();
        crate::print_status(&ctx);
        let log = service::log_file_in(&ctx.data_dir);
        println!();
        match std::fs::metadata(&log) {
            Ok(_) => {
                println!("{}", dim(&tf!("  最近日志 {}", "  Recent log {}", log.display())));
                for l in crate::logfile::tail(&log, 30).lines() {
                    println!("  {l}");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied || data_access(&ctx.data_dir) == Access::Denied => {
                if cfg!(windows) {
                    println!("{}", yellow(t!("  读日志需要管理员权限（以管理员身份重新打开菜单）。", "  Reading the log needs administrator rights.")));
                } else if self.t.confirm(t!("读日志需要 root，用 sudo 查看最近日志？", "Reading the log needs root. View it with sudo?"))? {
                    let _ = Command::new("sudo").args(["tail", "-n", "30"]).arg(&log).status();
                }
            }
            Err(_) => println!("{}", dim(t!("  还没有日志文件（前台运行时日志只在窗口里）。", "  No log file yet (foreground runs log to the window)."))),
        }
        println!();
        self.t.pause()
    }

    fn show_address(&self) -> anyhow::Result<()> {
        let (ctx, _) = crate::resolve_manage_ctx(self.cli);
        if data_access(&ctx.data_dir) == Access::Denied {
            return self.offer_elevate(&ctx.data_dir);
        }
        let admins = crate::admin_count(&ctx.data_dir);
        let run = service::read_run_info(&ctx.data_dir);
        let address = run.as_ref().map(|r| r.address.clone()).unwrap_or_else(|| crate::fallback_address(&ctx));
        println!();
        println!("  {}", dim(t!("客户端填写的服务器地址（# 后面是证书指纹，整行一起填）：", "Server address for clients (the part after # is the certificate fingerprint; enter the whole line):")));
        println!("  {}", bold(&address));
        if let Some(u) = run.as_ref().and_then(|r| r.setup_url.as_ref()).filter(|_| admins.unwrap_or(0) == 0) {
            println!();
            println!("  {}", dim(t!("还没有管理员。浏览器打开初始化网址创建管理员（会提示自签证书，核对指纹后继续）：", "No administrator yet. Open the setup URL in a browser to create one (accept the self-signed certificate after checking the fingerprint):")));
            println!("  {}", bold(u));
        } else if admins.unwrap_or(0) > 0 {
            // 地址形如 host:port#指纹 或 [v6]:port#指纹
            let host = address.split('#').next().and_then(|a| a.rsplit_once(':')).map(|(h, _)| h.to_string()).unwrap_or_else(|| "127.0.0.1".into());
            println!();
            println!("  {}", dim(t!("管理后台：", "Admin console:")));
            println!("  {}", bold(&format!("https://{host}:{}/", ctx.port)));
        }
        if address.starts_with("127.0.0.1:") {
            println!();
            println!("{}", yellow(t!("  没有设置对外地址，其他电脑连不上。到「系统服务 → 修改服务参数」里填域名或 IP。", "  No public host set, so other computers cannot connect. Set one in System service > Change service settings.")));
        }
        println!();
        println!("{}", dim(t!("  复制：在终端里用鼠标选中整行（Windows 控制台选中后按回车，macOS 按 Command+C）。", "  To copy: select the line with the mouse (Enter in Windows console, Command+C on macOS).")));
        println!();
        self.t.pause()
    }

    /// 打开数据库（读不了数据目录时提示提升权限，返回 None）。
    fn open_db(&self) -> anyhow::Result<Option<(ServeCtx, crate::db::Db, crate::state::Settings)>> {
        let (ctx, _) = crate::resolve_manage_ctx(self.cli);
        if data_access(&ctx.data_dir) == Access::Denied {
            self.offer_elevate(&ctx.data_dir)?;
            return Ok(None);
        }
        let (db, _, settings) = crate::open_data(&ctx.data_dir, ctx.public_host.as_deref())?;
        Ok(Some((ctx, db, settings)))
    }

    /// 输入两遍密码。取消时返回 None。
    fn new_password(&self) -> anyhow::Result<Option<String>> {
        loop {
            let Some(p1) = self.t.input(t!("密码（至少 8 位）", "Password (8+ characters)"), "", "", true)? else { return Ok(None) };
            if p1.chars().count() < 8 {
                println!("{}", red(t!("  密码至少 8 位", "  Password must be at least 8 characters")));
                continue;
            }
            let Some(p2) = self.t.input(t!("再输入一遍", "Repeat password"), "", "", true)? else { return Ok(None) };
            if p1 != p2 {
                println!("{}", red(t!("  两次输入的不一样", "  Passwords do not match")));
                continue;
            }
            return Ok(Some(p1));
        }
    }

    fn ask_username(&self, label: &str) -> anyhow::Result<Option<String>> {
        let mut hint = t!("3 到 32 位字母、数字或 ._-", "3 to 32 letters, digits or ._-");
        loop {
            let Some(u) = self.t.input(label, "", std::mem::take(&mut hint), false)? else { return Ok(None) };
            if u.is_empty() {
                return Ok(None);
            }
            if crate::admin::valid_username(&u) {
                return Ok(Some(u));
            }
            println!("{}", red(t!("  用户名需为 3 到 32 位字母、数字或 ._-", "  Username must be 3 to 32 letters, digits or ._-")));
        }
    }

    fn admins_menu(&self) -> anyhow::Result<()> {
        loop {
            let Some((_ctx, db, _)) = self.open_db()? else { return Ok(()) };
            let admins: Vec<(String, bool, i64)> = db.call_sync(|c| {
                let mut st = c.prepare("SELECT username, totp_enabled, last_login_at FROM admins ORDER BY id")?;
                let v = st.query_map([], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0, r.get(2)?)))?.collect::<Result<Vec<_>, _>>()?;
                Ok(v)
            })?;
            let mut items = vec![Item::new("create", t!("创建管理员", "Create administrator"), t!("首次登录后台时绑定两步验证", "binds two-factor auth at first login"))];
            for (i, (name, totp, last)) in admins.iter().enumerate() {
                let mut hint = if *totp { t!("已绑定两步验证", "2FA bound").to_string() } else { t!("未绑定两步验证", "2FA not bound").to_string() };
                if *last > 0 {
                    hint.push_str(&tf!(" · 上次登录 {}", " · last login {}", fmt_time(*last)));
                }
                items.push(Item::new(format!("a{i}"), name.clone(), hint));
            }
            let title = bold(&tf!("管理员（{} 个）· 选一个重置密码", "Administrators ({}) · pick one to reset its password", admins.len()));
            let Some(k) = self.t.choose(&title, &items, 0, false)? else { return Ok(()) };
            let r = if k == "create" {
                (|| -> anyhow::Result<()> {
                    let Some(u) = self.ask_username(t!("用户名", "Username"))? else { return Ok(()) };
                    let Some(p) = self.new_password()? else { return Ok(()) };
                    crate::admin_cli(&db, &AdminCmd::Create { username: u, password: p })?;
                    self.t.pause()
                })()
            } else {
                let name = k.strip_prefix('a').and_then(|i| i.parse::<usize>().ok()).and_then(|i| admins.get(i)).map(|a| a.0.clone()).unwrap_or_default();
                (|| -> anyhow::Result<()> {
                    if !self.t.confirm(&tf!("重置 {name} 的密码？同时解除两步验证并注销其登录。", "Reset the password of {name}? This also removes its 2FA and signs it out."))? {
                        return Ok(());
                    }
                    let Some(p) = self.new_password()? else { return Ok(()) };
                    crate::admin_cli(&db, &AdminCmd::ResetPassword { username: name.clone(), password: p })?;
                    self.t.pause()
                })()
            };
            self.after(r)?;
        }
    }

    fn users_menu(&self) -> anyhow::Result<()> {
        loop {
            let Some((ctx, db, settings)) = self.open_db()? else { return Ok(()) };
            let n: i64 = db.call_sync(|c| c.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0)))?;
            let items = vec![
                Item::new("create", t!("创建用户", "Create user"), t!("首次登录时必须改密码", "must change password at first login")),
                Item::new("list", t!("列出用户", "List users"), tf!("{n} 个", "{n}")),
            ];
            let title = format!("{}\n{}", bold(t!("用户", "Users")), dim(t!("  日常在 Web 管理后台管理；这里给没有图形界面的场景用。", "  Usually managed in the web admin console; this is for headless setups.")));
            let Some(k) = self.t.choose(&title, &items, 0, false)? else { return Ok(()) };
            let r = match k.as_str() {
                "create" => (|| -> anyhow::Result<()> {
                    let Some(u) = self.ask_username(t!("用户名", "Username"))? else { return Ok(()) };
                    let Some(dn) = self.t.input(t!("显示名（可留空）", "Display name (optional)"), "", "", false)? else { return Ok(()) };
                    let Some(p) = self.new_password()? else { return Ok(()) };
                    let keys = crate::keys::Keys::load(&ctx.data_dir, &settings.public_host, &ctx.tls_source())?;
                    crate::user_cli(&db, &keys, &UserCmd::Create { username: u, password: p, display_name: Some(dn).filter(|d| !d.is_empty()) })?;
                    self.t.pause()
                })(),
                "list" => (|| -> anyhow::Result<()> {
                    let rows: Vec<(i64, String, String, bool)> = db.call_sync(|c| {
                        let mut s = c.prepare("SELECT id, username, display_name, disabled FROM users ORDER BY id")?;
                        let v = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? != 0)))?.collect::<Result<Vec<_>, _>>()?;
                        Ok(v)
                    })?;
                    println!();
                    if rows.is_empty() {
                        println!("{}", dim(t!("  还没有用户。", "  No users yet.")));
                    } else {
                        let w = rows.iter().map(|r| tui::display_width(&r.1)).max().unwrap_or(0).max(tui::display_width(t!("用户名", "Username")));
                        println!("  {}", dim(&format!("{:>5}  {}  {}", "ID", pad(t!("用户名", "Username"), w), t!("显示名", "Display name"))));
                        for (id, u, d, dis) in rows {
                            let tag = if dis { format!("  {}", red(t!("已禁用", "disabled"))) } else { String::new() };
                            println!("  {id:>5}  {}  {d}{tag}", pad(&u, w));
                        }
                    }
                    println!();
                    self.t.pause()
                })(),
                _ => Ok(()),
            };
            self.after(r)?;
        }
    }

    fn update(&self, s: &Snap) -> anyhow::Result<()> {
        let repo = s.repo.clone().unwrap_or_else(|| crate::state::Settings::default().update_github_repo);
        println!("{}", dim(&tf!("  正在检查 github.com/{repo} 的最新发布…", "  Checking the latest release of github.com/{repo}...")));
        let r = tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(crate::update::self_update(&repo, true))?;
        match r {
            None => {
                println!("{}", green(&tf!("  已是最新版本 {}", "  Already up to date ({})", galaxy_proto::APP_VERSION)));
                self.t.pause()
            }
            Some(u) => {
                let q = tf!("有新版本 {}（当前 {}），现在升级？{}", "Version {} is available (current {}). Upgrade now?{}", u.version, galaxy_proto::APP_VERSION, if s.svc.is_some() { t!("系统服务会自动重启。", " The service restarts automatically.") } else { "" });
                if !self.t.confirm(&q)? {
                    return Ok(());
                }
                // 装了服务要替换服务用的那一份，需要 root / 管理员
                self.run_self(&["self-update"], s.svc.is_some())
            }
        }
    }
}

fn elevate_label() -> String {
    if cfg!(windows) {
        t!("以管理员身份重新打开菜单", "Reopen as administrator").into()
    } else {
        t!("用 sudo 重新打开菜单", "Reopen with sudo").into()
    }
}

/// 毫秒时间戳显示成 UTC 时间。
fn fmt_time(ms: i64) -> String {
    let fmt = time::macros::format_description!("[year]-[month]-[day] [hour]:[minute] UTC");
    time::OffsetDateTime::from_unix_timestamp(ms / 1000).ok().and_then(|t| t.format(&fmt).ok()).unwrap_or_default()
}

/// 给用户看的命令行：有空格等特殊字符的参数加引号。
fn shell_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "/._:@,+=-\\".contains(c)) {
        s.to_string()
    } else {
        format!("\"{}\"", s.replace('"', "\\\""))
    }
}

#[cfg(windows)]
fn shell_execute_runas(exe: &Path) -> anyhow::Result<()> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(hwnd: *mut c_void, op: *const u16, file: *const u16, params: *const u16, dir: *const u16, show: i32) -> isize;
    }
    let wide = |s: &std::ffi::OsStr| s.encode_wide().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let op = wide("runas".as_ref());
    let file = wide(exe.as_os_str());
    let dir = std::env::current_dir().map(|d| wide(d.as_os_str())).unwrap_or_else(|_| vec![0]);
    // 返回值大于 32 表示成功；用户在 UAC 里点「否」也会失败
    let r = unsafe { ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), file.as_ptr(), std::ptr::null(), dir.as_ptr(), 1) };
    anyhow::ensure!(r > 32, "{}", t!("没有以管理员身份打开（UAC 被取消？）", "Could not start as administrator (UAC cancelled?)"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("install-service"), "install-service");
        assert_eq!(shell_quote(""), "\"\"");
        assert_eq!(shell_quote("/a b/c"), "\"/a b/c\"");
    }
}
