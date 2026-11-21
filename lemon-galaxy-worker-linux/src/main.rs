//! LemonGalaxy Linux 被控端（命令行）。命令与说明见 galaxy-worker。

use galaxy_proto::Platform;
use galaxy_worker::{t, WorkerOs};
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct LinuxOs;

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

    fn install_service(&self, dir: &Path) -> i32 {
        install_service(dir)
    }

    fn uninstall_service(&self) -> i32 {
        uninstall_service()
    }
}

fn main() {
    // 命令行输出接到 head 等程序时，对方先关管道就安静退出，不报错
    if std::env::args().nth(1).map(|c| c != "run").unwrap_or(true) {
        unsafe {
            signal(13, 0);
        }
    }
    galaxy_worker::main(Arc::new(LinuxOs));
}

extern "C" {
    /// signal(SIGPIPE=13, SIG_DFL=0)
    fn signal(sig: i32, handler: usize) -> usize;
}

fn install_service(dir: &Path) -> i32 {
    let exe = std::env::current_exe().ok().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
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
            "[Unit]\nDescription=LemonGalaxy worker\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\nExecStart={target} run --data-dir {}\nRestart=always\nRestartSec=3\nKillMode=process\n\n[Install]\nWantedBy=multi-user.target\n",
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
            "[Unit]\nDescription=LemonGalaxy worker (user)\nAfter=network-online.target\n\n[Service]\nExecStart={exe} run --data-dir {}\nRestart=always\nRestartSec=3\n\n[Install]\nWantedBy=default.target\n",
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
