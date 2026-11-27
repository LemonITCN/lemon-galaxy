//! 系统相关部分（galaxy_worker::WorkerOs）：数据目录、服务安装、版本信息。画面能力在 desktop 模块。

use galaxy_proto::Platform;
use galaxy_worker::{t, WorkerOs};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct LinuxOs;

fn is_root() -> bool {
    galaxy_host::sysuser::is_root()
}

impl WorkerOs for LinuxOs {
    fn platform(&self) -> Platform {
        if cfg!(target_os = "macos") {
            Platform::Macos
        } else {
            Platform::Linux
        }
    }

    fn os_version(&self) -> String {
        if let Ok(s) = std::fs::read_to_string("/etc/os-release") {
            for l in s.lines() {
                if let Some(v) = l.strip_prefix("PRETTY_NAME=") {
                    return v.trim_matches('"').to_string();
                }
            }
        }
        if cfg!(target_os = "macos") {
            return "macOS".into();
        }
        std::process::Command::new("uname").arg("-sr").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_else(|| "Linux".into())
    }

    fn default_data_dir(&self, admin: bool) -> PathBuf {
        if admin {
            PathBuf::from("/var/lib/lemon-galaxy-worker")
        } else {
            let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| {
                let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
                home.join(".local/share")
            });
            base.join("lemon-galaxy-worker")
        }
    }

    fn default_system_user(&self) -> String {
        if is_root() {
            std::env::var("SUDO_USER").unwrap_or_else(|_| "root".into())
        } else {
            galaxy_host::sysuser::SysUser::current().name
        }
    }

    fn superuser_name(&self) -> &'static str {
        "root"
    }

    fn host_platform(&self) -> Option<Arc<dyn galaxy_host::platform::HostPlatform>> {
        crate::desktop::host_platform()
    }

    fn extra_command(&self, cmd: &str, rest: &[String], dir: &Path) -> Option<i32> {
        crate::desktop::extra_command(cmd, rest, dir)
    }

    fn extra_help(&self) -> Vec<(&'static str, &'static str, &'static str)> {
        crate::desktop::extra_help()
    }

    fn install_service(&self, dir: &Path) -> i32 {
        install_service(dir)
    }

    fn uninstall_service(&self) -> i32 {
        uninstall_service()
    }

    fn update_target(&self) -> Option<PathBuf> {
        let installed = PathBuf::from("/usr/local/bin/lemon-galaxy-worker");
        (is_root() && installed.exists()).then_some(installed)
    }

    fn restart_service(&self) -> bool {
        systemctl(&["restart", "lemon-galaxy-worker"])
    }

    fn stop_service(&self) -> bool {
        systemctl(&["stop", "lemon-galaxy-worker"])
    }

    /// 换回旧版本后启动：新版本崩溃太频繁时 systemd 会拒绝再启动（start-limit-hit），先清掉失败状态
    fn start_service(&self) -> bool {
        let _ = systemctl(&["reset-failed", "lemon-galaxy-worker"]);
        systemctl(&["start", "lemon-galaxy-worker"])
    }

    /// deb / rpm 装的（包管理器查得到、程序就是 /usr/bin 下那份）：自动更新由包管理器装整个新包（界面 + 被控服务）
    fn desktop_install(&self) -> Option<galaxy_worker::upgrade::DesktopInstall> {
        galaxy_worker::upgrade::linux_package()
    }
}

/// systemctl（系统服务；不是 root 时是用户服务）
fn systemctl(args: &[&str]) -> bool {
    let mut c = std::process::Command::new("systemctl");
    if !is_root() {
        c.arg("--user");
    }
    c.args(args).stdin(std::process::Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

/// 装 .deb / .rpm 时自带的系统服务
const PACKAGED_UNIT: &str = "/usr/lib/systemd/system/lemon-galaxy-worker.service";

/// 是不是安装包装的（程序在 /usr/bin、包带了服务文件）
fn packaged() -> bool {
    let exe = std::env::current_exe().ok().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
    exe == "/usr/bin/lemon-galaxy-worker" && Path::new(PACKAGED_UNIT).exists()
}

fn install_service(dir: &Path) -> i32 {
    let exe = std::env::current_exe().ok().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
    // 安装包已经带了服务：只启用它，不再复制程序、不另写服务文件
    if packaged() && is_root() {
        let ok = std::process::Command::new("systemctl").args(["enable", "--now", "lemon-galaxy-worker"]).status().map(|s| s.success()).unwrap_or(false);
        println!("{}", if ok { t!("已启用并启动系统服务 lemon-galaxy-worker。", "System service enabled and started.") } else { t!("启用服务失败。", "Failed to enable service.") });
        let _ = dir;
        return if ok { 0 } else { 1 };
    }
    let has_systemd = std::path::Path::new("/run/systemd/system").exists();
    if !has_systemd {
        eprintln!("{}", t!("没有检测到 systemd。请用你的系统的方式让下面的命令开机自动运行：", "systemd not found. Make this command start at boot:"));
        println!("{exe} run --data-dir {}", dir.display());
        return 1;
    }
    if is_root() {
        let target = "/usr/local/bin/lemon-galaxy-worker";
        if exe != target {
            if let Err(e) = std::fs::copy(&exe, target) {
                eprintln!("{} {target}: {e}", t!("无法复制到", "Cannot copy to"));
                return 1;
            }
        }
        let unit = format!(
            "[Unit]\nDescription=LemonGalaxy worker\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\nExecStart={target} run --data-dir {}\nEnvironment=LEMON_GALAXY_SUPERVISED=1\nRestart=always\nRestartSec=3\nKillMode=process\n\n[Install]\nWantedBy=multi-user.target\n",
            dir.display()
        );
        if let Err(e) = std::fs::write("/etc/systemd/system/lemon-galaxy-worker.service", unit) {
            eprintln!("{e}");
            return 1;
        }
        let ok = std::process::Command::new("systemctl").args(["daemon-reload"]).status().map(|s| s.success()).unwrap_or(false)
            && std::process::Command::new("systemctl").args(["enable", "--now", "lemon-galaxy-worker"]).status().map(|s| s.success()).unwrap_or(false);
        println!("{}", if ok { t!("已安装并启动系统服务 lemon-galaxy-worker。", "System service installed and started.") } else { t!("安装服务失败。", "Failed to install service.") });
        if ok {
            0
        } else {
            1
        }
    } else {
        let home = std::env::var("HOME").unwrap_or_default();
        let unit_dir = PathBuf::from(&home).join(".config/systemd/user");
        let _ = std::fs::create_dir_all(&unit_dir);
        let unit = format!(
            "[Unit]\nDescription=LemonGalaxy worker (user)\nAfter=network-online.target\n\n[Service]\nExecStart={exe} run --data-dir {}\nEnvironment=LEMON_GALAXY_SUPERVISED=1\nRestart=always\nRestartSec=3\n\n[Install]\nWantedBy=default.target\n",
            dir.display()
        );
        if let Err(e) = std::fs::write(unit_dir.join("lemon-galaxy-worker.service"), unit) {
            eprintln!("{e}");
            return 1;
        }
        let ok = std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).status().map(|s| s.success()).unwrap_or(false)
            && std::process::Command::new("systemctl").args(["--user", "enable", "--now", "lemon-galaxy-worker"]).status().map(|s| s.success()).unwrap_or(false);
        println!("{}", if ok { t!("已安装并启动用户服务。", "User service installed and started.") } else { t!("安装用户服务失败。", "Failed to install user service.") });
        println!("{}", t!("提示：要在你没登录时也保持运行，需要管理员执行一次 loginctl enable-linger 你的用户名", "Tip: ask an admin to run `loginctl enable-linger <you>` to keep it running after logout."));
        if ok {
            0
        } else {
            1
        }
    }
}

fn uninstall_service() -> i32 {
    if packaged() && is_root() {
        // 安装包的服务文件归包管理器管：只停掉、取消开机自启（卸载软件用 apt / dnf）
        let _ = std::process::Command::new("systemctl").args(["disable", "--now", "lemon-galaxy-worker"]).status();
        println!("{}", t!("已停止服务并取消开机自启。卸载软件请用 apt remove / dnf remove lemon-galaxy-worker。", "Service stopped and disabled. Use your package manager to remove the software."));
        return 0;
    }
    if is_root() {
        let _ = std::process::Command::new("systemctl").args(["disable", "--now", "lemon-galaxy-worker"]).status();
        let _ = std::fs::remove_file("/etc/systemd/system/lemon-galaxy-worker.service");
        let _ = std::process::Command::new("systemctl").args(["daemon-reload"]).status();
    } else {
        let _ = std::process::Command::new("systemctl").args(["--user", "disable", "--now", "lemon-galaxy-worker"]).status();
        let home = std::env::var("HOME").unwrap_or_default();
        let _ = std::fs::remove_file(PathBuf::from(home).join(".config/systemd/user/lemon-galaxy-worker.service"));
    }
    println!("{}", t!("已卸载服务。", "Service removed."));
    0
}
