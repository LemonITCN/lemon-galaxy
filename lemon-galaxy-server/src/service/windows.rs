//! Windows：LocalSystem 系统服务（开机自启、失败自动重启）、防火墙规则、服务入口、双击运行。

use super::SvcConfig;
use std::ffi::OsString;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;
use windows_service::service::{
    Service, ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo, ServiceStartType, ServiceState,
    ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

pub const SERVICE_NAME: &str = "LemonGalaxyServer";
const DISPLAY_NAME: &str = "LemonGalaxy Server";
const DESCRIPTION: &str = "LemonGalaxy 服务器：账号、设备、信令、中转、Web 管理后台";
/// 防火墙入站规则（名称, 协议）。
const FW_RULES: [(&str, &str); 2] = [("LemonGalaxy Server TCP", "TCP"), ("LemonGalaxy Server UDP", "UDP")];

const ERROR_ACCESS_DENIED: i32 = 5;
const ERROR_SERVICE_DOES_NOT_EXIST: i32 = 1060;

fn install_dir() -> PathBuf {
    let pf = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Program Files"));
    pf.join("LemonGalaxy").join("Server")
}

pub fn installed_exe() -> PathBuf {
    install_dir().join("lemon-galaxy-server.exe")
}

pub fn default_data_dir() -> Option<PathBuf> {
    let pd = std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\ProgramData"));
    Some(pd.join("LemonGalaxy").join("server"))
}

pub fn config_path() -> Option<PathBuf> {
    Some(install_dir().join("service.json"))
}

fn os_code(e: &windows_service::Error) -> Option<i32> {
    match e {
        windows_service::Error::Winapi(io) => io.raw_os_error(),
        _ => None,
    }
}

fn werr(e: windows_service::Error) -> anyhow::Error {
    if os_code(&e) == Some(ERROR_ACCESS_DENIED) {
        return anyhow::anyhow!("需要以管理员身份运行（右键「以管理员身份运行」命令提示符或 PowerShell）");
    }
    match e {
        windows_service::Error::Winapi(io) => anyhow::anyhow!(io),
        other => anyhow::anyhow!(other),
    }
}

pub fn require_admin() -> anyhow::Result<()> {
    ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE).map_err(werr)?;
    Ok(())
}

/// 数据目录只给 SYSTEM 与管理员组（用 SID，与系统语言无关）。
pub fn secure_dir(dir: &Path) {
    let _ = Command::new("icacls").arg(dir).args(["/inheritance:r", "/grant:r", "*S-1-5-18:(OI)(CI)F", "*S-1-5-32-544:(OI)(CI)F", "/q"]).output();
}

fn wait_stopped(s: &Service) {
    for _ in 0..100 {
        if s.query_status().map(|st| st.current_state == ServiceState::Stopped).unwrap_or(true) {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn stop_service(s: &Service) {
    if s.query_status().map(|st| st.current_state != ServiceState::Stopped).unwrap_or(false) {
        let _ = s.stop();
        wait_stopped(s);
    }
}

fn stop_existing(manager: &ServiceManager) {
    if let Ok(s) = manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP) {
        stop_service(&s);
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

/// 服务刚停下时进程可能还没完全退出，文件仍被占用，重试几次。
fn retry<T>(mut f: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let mut last = None;
    for _ in 0..25 {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("retry")))
}

fn netsh(args: &str) -> std::io::Result<std::process::Output> {
    // 规则名里有空格，按 netsh 文档的写法原样传参
    Command::new("netsh").raw_arg(args).output()
}

fn firewall_remove() {
    for (name, _) in FW_RULES {
        let _ = netsh(&format!("advfirewall firewall delete rule name=\"{name}\""));
    }
}

fn firewall_add(port: u16) -> Result<bool, String> {
    firewall_remove();
    for (name, proto) in FW_RULES {
        let o = netsh(&format!("advfirewall firewall add rule name=\"{name}\" dir=in action=allow protocol={proto} localport={port} profile=any enable=yes")).map_err(|e| format!("无法运行 netsh：{e}"))?;
        if !o.status.success() {
            return Err(format!("netsh 退出码 {}", o.status.code().unwrap_or(-1)));
        }
    }
    Ok(true)
}

/// 复制程序、注册或更新服务、加防火墙规则、启动。返回防火墙处理结果。
pub fn install(cfg: &SvcConfig) -> anyhow::Result<Result<bool, String>> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE).map_err(werr)?;
    // 复制到固定位置，服务不依赖下载目录里的那一份
    let src = std::env::current_exe()?;
    let target = installed_exe();
    if !same_file(&src, &target) {
        stop_existing(&manager);
        std::fs::create_dir_all(install_dir())?;
        let _ = std::fs::remove_file(target.with_extension("old"));
        retry(|| std::fs::copy(&src, &target)).map_err(|e| anyhow::anyhow!("无法复制到 {}：{e}", target.display()))?;
    }
    let mut launch_arguments: Vec<OsString> = cfg.args().into_iter().map(OsString::from).collect();
    launch_arguments.push(OsString::from("service"));
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: target.clone(),
        launch_arguments,
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };
    let access = ServiceAccess::QUERY_STATUS | ServiceAccess::START | ServiceAccess::STOP | ServiceAccess::CHANGE_CONFIG;
    let service = match manager.open_service(SERVICE_NAME, access) {
        Ok(s) => {
            stop_service(&s);
            s.change_config(&info).map_err(werr)?;
            s
        }
        Err(_) => manager.create_service(&info, access).map_err(|e| anyhow::anyhow!("创建服务失败：{}", werr(e)))?,
    };
    let _ = service.set_description(DESCRIPTION);
    let _ = service.update_failure_actions(ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(86400)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![
            ServiceAction { action_type: ServiceActionType::Restart, delay: Duration::from_secs(3) },
            ServiceAction { action_type: ServiceActionType::Restart, delay: Duration::from_secs(10) },
            ServiceAction { action_type: ServiceActionType::Restart, delay: Duration::from_secs(30) },
        ]),
    });
    // 出错退出（不是崩溃）也按上面的规则重启
    let _ = service.set_failure_actions_on_non_crash_failures(true);
    let fw = firewall_add(cfg.port);
    service.start::<&str>(&[]).map_err(|e| anyhow::anyhow!("服务已安装但启动失败：{}", werr(e)))?;
    Ok(fw)
}

/// 返回是否真的卸载了服务。
pub fn uninstall() -> anyhow::Result<bool> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).map_err(werr)?;
    let had = match manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE) {
        Ok(s) => {
            stop_service(&s);
            s.delete().map_err(werr)?;
            true
        }
        Err(e) if os_code(&e) == Some(ERROR_SERVICE_DOES_NOT_EXIST) => false,
        Err(e) => return Err(werr(e)),
    };
    firewall_remove();
    let exe = installed_exe();
    if exe.exists() {
        let running_self = std::env::current_exe().map(|c| same_file(&c, &exe)).unwrap_or(false);
        if running_self {
            println!("程序文件 {} 正在使用，请退出后手动删除。", exe.display());
        } else if let Err(e) = retry(|| std::fs::remove_file(&exe)) {
            println!("程序文件 {} 删不掉（{e}），可稍后手动删除。", exe.display());
        }
    }
    let _ = std::fs::remove_file(exe.with_extension("old"));
    if let Some(c) = config_path() {
        let _ = std::fs::remove_file(c);
    }
    let dir = install_dir();
    let _ = std::fs::remove_dir(&dir);
    if let Some(p) = dir.parent() {
        let _ = std::fs::remove_dir(p);
    }
    Ok(had)
}

fn query() -> Option<ServiceStatus> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).ok()?;
    manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS).ok()?.query_status().ok()
}

pub fn state() -> Option<String> {
    let st = query()?;
    Some(match st.current_state {
        ServiceState::Running => format!("运行中（进程 {}）", st.process_id.unwrap_or(0)),
        ServiceState::StartPending => "正在启动".into(),
        ServiceState::StopPending => "正在停止".into(),
        ServiceState::Stopped => "已停止".into(),
        _ => "已暂停".into(),
    })
}

pub fn is_stopped() -> bool {
    query().map(|s| s.current_state == ServiceState::Stopped).unwrap_or(true)
}

pub fn update_target() -> Option<PathBuf> {
    let exe = installed_exe();
    exe.exists().then_some(exe)
}

pub fn restart_if_installed() -> anyhow::Result<bool> {
    if query().is_none() {
        return Ok(false);
    }
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).map_err(werr)?;
    let s = manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::START).map_err(werr)?;
    stop_service(&s);
    s.start::<&str>(&[]).map_err(werr)?;
    Ok(true)
}

// ---------------- 服务入口 ----------------

static CTX: OnceLock<crate::ServeCtx> = OnceLock::new();

define_windows_service!(ffi_service_main, service_main);

/// `service` 子命令：交给服务管理器，直到服务停止。
pub fn run_dispatcher(ctx: crate::ServeCtx) -> anyhow::Result<()> {
    let _ = CTX.set(ctx);
    service_dispatcher::start(SERVICE_NAME, ffi_service_main).map_err(|e| anyhow::anyhow!("service 命令只给 Windows 服务管理器用，前台运行请用 lemon-galaxy-server run（{}）", werr(e)))
}

fn set_status(h: ServiceStatusHandle, state: ServiceState, code: u32) {
    let _ = h.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: if state == ServiceState::Running { ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN } else { ServiceControlAccept::empty() },
        exit_code: if code == 0 { ServiceExitCode::Win32(0) } else { ServiceExitCode::ServiceSpecific(code) },
        checkpoint: if state == ServiceState::StopPending { 1 } else { 0 },
        wait_hint: Duration::from_secs(20),
        process_id: None,
    });
}

fn service_main(_args: Vec<OsString>) {
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let stop_tx = std::sync::Mutex::new(Some(stop_tx));
    let handler = move |ev| -> ServiceControlHandlerResult {
        match ev {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                if let Some(tx) = stop_tx.lock().ok().and_then(|mut g| g.take()) {
                    let _ = tx.send(());
                }
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };
    let h = match service_control_handler::register(SERVICE_NAME, handler) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("注册服务控制失败：{}", werr(e));
            return;
        }
    };
    set_status(h, ServiceState::Running, 0);
    let Some(ctx) = CTX.get().cloned() else {
        set_status(h, ServiceState::Stopped, 1);
        return;
    };
    let shutdown = async move {
        let _ = stop_rx.await;
        tracing::info!("收到服务管理器的停止请求");
        set_status(h, ServiceState::StopPending, 0);
    };
    let code = match crate::serve_blocking(ctx, shutdown) {
        Ok(()) => 0,
        Err(e) => {
            tracing::error!("服务器退出：{e:#}");
            1
        }
    };
    set_status(h, ServiceState::Stopped, code);
}

// ---------------- 双击运行 ----------------

#[link(name = "kernel32")]
extern "system" {
    fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32;
}

/// 是否从资源管理器双击启动（控制台窗口是专门为本进程开的，关掉窗口就看不到输出了）。
pub fn launched_from_explorer() -> bool {
    let mut buf = [0u32; 4];
    unsafe { GetConsoleProcessList(buf.as_mut_ptr(), buf.len() as u32) == 1 }
}
