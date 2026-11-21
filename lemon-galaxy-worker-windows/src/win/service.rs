//! Windows 服务：安装（复制到 Program Files、开机自启、失败自动重启）、卸载、服务入口。

use super::WindowsOs;
use galaxy_worker::t;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use windows_service::service::{
    ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo, ServiceStartType, ServiceState,
    ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

pub const SERVICE_NAME: &str = "LemonGalaxyWorker";
const DISPLAY_NAME: &str = "LemonGalaxy Worker";

static DATA_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

pub fn install_dir() -> PathBuf {
    let pf = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Program Files"));
    pf.join("LemonGalaxy").join("Worker")
}

fn stop_existing(manager: &ServiceManager) {
    if let Ok(s) = manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP) {
        if s.query_status().map(|st| st.current_state != ServiceState::Stopped).unwrap_or(false) {
            let _ = s.stop();
            for _ in 0..50 {
                std::thread::sleep(Duration::from_millis(200));
                if s.query_status().map(|st| st.current_state == ServiceState::Stopped).unwrap_or(true) {
                    break;
                }
            }
        }
    }
}

pub fn install(dir: &Path) -> i32 {
    if !galaxy_host::sysuser::is_admin() {
        eprintln!("{}", t!("需要以管理员身份运行（右键「以管理员身份运行」命令提示符或 PowerShell）。", "Please run as administrator."));
        return 1;
    }
    let manager = match ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{}: {e}", t!("无法连接服务管理器", "Cannot open service manager"));
            return 1;
        }
    };
    // 复制到固定位置，服务不依赖下载目录里的那一份
    let src = std::env::current_exe().unwrap_or_default();
    let target_dir = install_dir();
    let target = target_dir.join("lemon-galaxy-worker.exe");
    if !same_file(&src, &target) {
        stop_existing(&manager);
        if let Err(e) = std::fs::create_dir_all(&target_dir).and_then(|_| std::fs::copy(&src, &target).map(|_| ())) {
            eprintln!("{} {}: {e}", t!("无法复制到", "Cannot copy to"), target.display());
            return 1;
        }
    }
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: target.clone(),
        launch_arguments: vec![OsString::from("--data-dir"), dir.as_os_str().to_owned(), OsString::from("service")],
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };
    let access = ServiceAccess::QUERY_STATUS | ServiceAccess::START | ServiceAccess::STOP | ServiceAccess::CHANGE_CONFIG;
    let service = match manager.open_service(SERVICE_NAME, access) {
        Ok(s) => {
            stop_existing(&manager);
            if let Err(e) = s.change_config(&info) {
                eprintln!("{e}");
                return 1;
            }
            s
        }
        Err(_) => match manager.create_service(&info, access) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: {e}", t!("创建服务失败", "Failed to create service"));
                return 1;
            }
        },
    };
    let _ = service.set_description(if galaxy_worker::zh() { "LemonGalaxy 被控端：远程终端、文件、桌面" } else { "LemonGalaxy worker: remote terminal, files and desktop" });
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
    let _ = service.set_failure_actions_on_non_crash_failures(true);
    // 允许服务发送 Ctrl+Alt+Del（远程解锁、登录界面要用）
    let _ = std::process::Command::new("reg")
        .args(["add", "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Policies\\System", "/v", "SoftwareSASGeneration", "/t", "REG_DWORD", "/d", "1", "/f"])
        .output();
    match service.start::<&str>(&[]) {
        Ok(()) => {
            println!("{}", t!("已安装并启动服务 LemonGalaxyWorker（开机自启，出错自动重启）。", "Service LemonGalaxyWorker installed and started."));
            println!("{} {}", t!("程序位置：", "Program:"), target.display());
            0
        }
        Err(e) => {
            eprintln!("{}: {e}", t!("服务已安装但启动失败", "Service installed but failed to start"));
            1
        }
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

pub fn uninstall() -> i32 {
    if !galaxy_host::sysuser::is_admin() {
        eprintln!("{}", t!("需要以管理员身份运行。", "Please run as administrator."));
        return 1;
    }
    let Ok(manager) = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT) else { return 1 };
    stop_existing(&manager);
    match manager.open_service(SERVICE_NAME, ServiceAccess::DELETE) {
        Ok(s) => {
            let _ = s.delete();
            println!("{}", t!("已卸载服务。数据目录与程序文件保留，可手动删除。", "Service removed."));
            0
        }
        Err(_) => {
            println!("{}", t!("没有安装服务。", "Service not installed."));
            0
        }
    }
}

define_windows_service!(ffi_service_main, service_main);

/// 由服务管理器启动时的入口。
pub fn run_dispatcher(dir: &Path) -> i32 {
    let _ = DATA_DIR.set(dir.to_path_buf());
    match service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{}: {e}", t!("只能由服务管理器启动；前台运行请用 lemon-galaxy-worker run", "This command is for the service manager; use `run` for foreground"));
            1
        }
    }
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
    let Ok(status) = service_control_handler::register(SERVICE_NAME, handler) else { return };
    let set = |state: ServiceState, code: u32| {
        let _ = status.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: if state == ServiceState::Running { ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN } else { ServiceControlAccept::empty() },
            exit_code: ServiceExitCode::Win32(code),
            checkpoint: 0,
            wait_hint: Duration::from_secs(10),
            process_id: None,
        });
    };
    set(ServiceState::Running, 0);
    let dir = DATA_DIR.get().cloned().unwrap_or_else(|| WindowsOs.default_dir(true));
    let code = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt.block_on(galaxy_worker::run_host(Arc::new(WindowsOs), &dir, async move {
            let _ = stop_rx.await;
        })),
        Err(_) => 1,
    };
    set(ServiceState::Stopped, code as u32);
}
