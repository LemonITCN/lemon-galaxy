//! 和桌面打交道的零碎：登录后显示托盘（XDG 自启动）、找内核程序、装服务、托盘宿主在不在。

use std::path::{Path, PathBuf};

pub const AUTOSTART_FILE: &str = "lemon-galaxy-worker-gui.desktop";

fn user_autostart() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
    base.join("autostart").join(AUTOSTART_FILE)
}

/// 包装的系统级自启动项（/etc/xdg/autostart，XDG_CONFIG_DIRS 里的第一个有的）
fn system_autostart() -> Option<PathBuf> {
    let dirs = std::env::var("XDG_CONFIG_DIRS").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/etc/xdg".into());
    dirs.split(':').map(|d| Path::new(d).join("autostart").join(AUTOSTART_FILE)).find(|p| p.exists())
}

fn hidden(text: &str) -> bool {
    text.lines().any(|l| {
        let l = l.trim();
        l.strip_prefix("Hidden").map(|r| r.trim_start().trim_start_matches('=').trim().eq_ignore_ascii_case("true")).unwrap_or(false)
    })
}

/// 登录后是否显示托盘
pub fn tray_autostart() -> bool {
    match std::fs::read_to_string(user_autostart()) {
        Ok(t) => !hidden(&t),
        Err(_) => system_autostart().is_some(),
    }
}

pub fn set_tray_autostart(on: bool) -> std::io::Result<()> {
    let p = user_autostart();
    let system = system_autostart();
    if on {
        if system.is_some() {
            // 包里已经装了系统级的，删掉自己的覆盖就行
            match std::fs::remove_file(&p) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
            return Ok(());
        }
        let exe = std::env::current_exe().map(|e| e.display().to_string()).unwrap_or_else(|_| "lemon-galaxy-worker-gui".into());
        write(&p, &format!("[Desktop Entry]\nType=Application\nName=LemonGalaxy\nComment=LemonGalaxy tray\nExec={exe} --tray\nIcon=lemon-galaxy-worker\nTerminal=false\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n"))
    } else if system.is_some() {
        // 系统级的删不了（也不该删），写一份 Hidden=true 盖住它
        write(&p, "[Desktop Entry]\nType=Application\nName=LemonGalaxy\nExec=lemon-galaxy-worker-gui --tray\nHidden=true\n")
    } else {
        match std::fs::remove_file(&p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

fn write(p: &Path, s: &str) -> std::io::Result<()> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(p, s)
}

fn in_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(name)).find(|f| f.is_file()))
}

/// 内核程序：和界面同目录的、PATH 里的、/usr/bin、/usr/local/bin
pub fn kernel_path() -> Option<PathBuf> {
    const NAME: &str = "lemon-galaxy-worker";
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        let p = d.join(NAME);
        if p.is_file() {
            return Some(p);
        }
    }
    in_path(NAME).or_else(|| ["/usr/bin", "/usr/local/bin"].iter().map(|d| Path::new(d).join(NAME)).find(|p| p.is_file()))
}

pub fn pkexec() -> Option<PathBuf> {
    in_path("pkexec").or_else(|| ["/usr/bin/pkexec", "/bin/pkexec"].iter().map(PathBuf::from).find(|p| p.is_file()))
}

/// 以管理员身份安装并启动服务（pkexec 弹密码框）。返回 (成功, 输出)
pub fn install_service(pkexec: &Path, kernel: &Path) -> (bool, String) {
    match std::process::Command::new(pkexec).arg(kernel).arg("install-service").output() {
        Ok(o) => {
            // 先错误输出（原因），再标准输出（命令行给的提示）
            let mut s = String::from_utf8_lossy(&o.stderr).trim().to_string();
            let out = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !out.is_empty() {
                if !s.is_empty() {
                    s.push('\n');
                }
                s.push_str(&out);
            }
            // pkexec：126 = 用户取消，127 = 没授权
            (o.status.success(), if matches!(o.status.code(), Some(126) | Some(127)) && s.is_empty() { "cancelled".into() } else { s })
        }
        Err(e) => (false, e.to_string()),
    }
}

/// 已安装的图标（hicolor）里有没有这个名字；没有就用内嵌的
pub fn icon_installed(name: &str) -> bool {
    let dirs = std::env::var("XDG_DATA_DIRS").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs.split(':').any(|d| {
        let base = Path::new(d).join("icons/hicolor");
        ["scalable/status", "scalable/apps", "symbolic/status", "symbolic/apps"].iter().any(|sub| base.join(sub).join(format!("{name}.svg")).exists())
    })
}

#[cfg(test)]
mod tests {
    use super::hidden;
    #[test]
    fn hidden_key() {
        assert!(hidden("[Desktop Entry]\nHidden=true\n"));
        assert!(hidden("[Desktop Entry]\nHidden = True\n"));
        assert!(!hidden("[Desktop Entry]\nHidden=false\n"));
        assert!(!hidden("[Desktop Entry]\nExec=x\n"));
    }
}
