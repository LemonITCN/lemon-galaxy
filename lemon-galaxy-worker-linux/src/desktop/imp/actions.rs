//! 各后端共用的桌面动作：锁屏、打开任务管理器。画面代理带着会话的环境变量，直接起命令即可。

use std::process::{Command, Stdio};

fn try_run(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

fn try_spawn(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok()
}

/// 锁屏：logind（会话号由服务通过 LG_SESSION_ID 传进来）→ 屏保接口 → 各桌面自己的命令。
pub fn lock_screen() {
    if let Ok(id) = std::env::var("LG_SESSION_ID") {
        if !id.is_empty() && !id.contains(':') && try_run("loginctl", &["lock-session", &id]) {
            return;
        }
    }
    for (c, a) in [
        ("dbus-send", &["--session", "--type=method_call", "--dest=org.freedesktop.ScreenSaver", "/org/freedesktop/ScreenSaver", "org.freedesktop.ScreenSaver.Lock"][..]),
        ("xdg-screensaver", &["lock"][..]),
        ("gnome-screensaver-command", &["--lock"][..]),
        ("xflock4", &[][..]),
        ("mate-screensaver-command", &["--lock"][..]),
        ("cinnamon-screensaver-command", &["--lock"][..]),
        ("dm-tool", &["lock"][..]),
        ("light-locker-command", &["--lock"][..]),
    ] {
        if try_run(c, a) {
            return;
        }
    }
}

/// 打开系统监视器；都没有返回 false（调用方再按 Ctrl+Alt+Del）。
pub fn task_manager() -> bool {
    for c in ["gnome-system-monitor", "plasma-systemmonitor", "ksysguard", "xfce4-taskmanager", "mate-system-monitor", "lxtask", "qps"] {
        if try_spawn(c, &[]) {
            return true;
        }
    }
    false
}
