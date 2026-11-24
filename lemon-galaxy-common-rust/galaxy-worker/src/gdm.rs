//! 登录界面使用 X11：GDM 默认用 Wayland，远程看不到登录界面。改 GDM 配置 `[daemon] WaylandEnable=false`
//! 后，登录界面（以及之后登录的桌面）都用 X11（docs/linux-desktop.md 第 4 节）。重启电脑或注销后生效。
//!
//! 只改这一行，文件里其他内容原样保留。
//!
//! GNOME 49 起（Ubuntu 25.10 / 26.04、Fedora 43 等）去掉了 X11 会话，这类系统上关掉 Wayland 可能让登录界面起不来：
//! 打开前先确认有 Xorg 和 X11 桌面会话，没有就拒绝。恢复 Wayland 永远允许。

use std::path::{Path, PathBuf};

/// Debian / Ubuntu 是 gdm3，Fedora / RHEL / Arch 是 gdm。
const CANDIDATES: [&str; 2] = ["/etc/gdm3/custom.conf", "/etc/gdm/custom.conf"];
/// Xorg 可执行文件可能的位置
pub const XORG_PATHS: [&str; 3] = ["/usr/lib/xorg/Xorg", "/usr/libexec/Xorg", "/usr/bin/Xorg"];
/// X11 桌面会话目录
pub const XSESSIONS_DIR: &str = "/usr/share/xsessions";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GdmState {
    /// 登录界面不是 GDM（或没装图形登录）
    NotGdm,
    /// 配置文件路径，是否已强制 X11，这个系统能不能用 X11（有 Xorg 和 X11 桌面会话）
    Gdm { path: PathBuf, x11: bool, x11_available: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GdmError {
    /// 登录界面不是 GDM
    NotGdm,
    /// 没有 Xorg 或 X11 桌面会话（GNOME 49 起已去掉）
    NoX11Session,
    Io(String),
}

impl GdmError {
    pub fn message(&self, zh: bool) -> String {
        match (self, zh) {
            (GdmError::NotGdm, true) => "登录界面不是 GDM，不需要修改。".into(),
            (GdmError::NotGdm, false) => "The login screen is not GDM; nothing to change.".into(),
            (GdmError::NoX11Session, true) => "这个系统没有 X11 桌面会话（GNOME 49 起已去掉），不能把登录界面切到 X11；没人登录时远程会自动使用虚拟桌面。".into(),
            (GdmError::NoX11Session, false) => {
                "This system has no X11 desktop session (removed in GNOME 49), so the login screen cannot be switched to X11. When nobody is signed in, remote access uses a virtual desktop automatically.".into()
            }
            (GdmError::Io(e), _) => e.clone(),
        }
    }
}

impl std::fmt::Display for GdmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message(crate::zh()))
    }
}

impl std::error::Error for GdmError {}

/// 这个系统能不能用 X11：有 Xorg 可执行文件，且 X11 会话目录里至少有一个 .desktop。
pub fn x11_available() -> bool {
    let xorg: Vec<&Path> = XORG_PATHS.iter().map(Path::new).collect();
    x11_available_in(&xorg, Path::new(XSESSIONS_DIR))
}

pub fn x11_available_in(xorg_paths: &[&Path], xsessions: &Path) -> bool {
    let has_xorg = xorg_paths.iter().any(|p| p.is_file());
    let has_session = std::fs::read_dir(xsessions).map(|rd| rd.flatten().any(|e| e.path().extension().is_some_and(|x| x == "desktop"))).unwrap_or(false);
    has_xorg && has_session
}

/// 当前的显示管理器是不是 GDM；是的话返回配置文件路径（文件可能还不存在）。
pub fn config_path() -> Option<PathBuf> {
    // systemd 系统上 display-manager.service 指向正在用的显示管理器
    if let Ok(t) = std::fs::read_link("/etc/systemd/system/display-manager.service") {
        let name = t.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if !name.starts_with("gdm") {
            return None;
        }
    }
    for c in CANDIDATES {
        if Path::new(c).exists() {
            return Some(PathBuf::from(c));
        }
    }
    CANDIDATES.iter().map(Path::new).find(|p| p.parent().is_some_and(|d| d.is_dir())).map(Path::to_path_buf)
}

pub fn state() -> GdmState {
    match config_path() {
        None => GdmState::NotGdm,
        Some(path) => {
            let x11 = std::fs::read_to_string(&path).map(|s| x11_forced(&s)).unwrap_or(false);
            GdmState::Gdm { path, x11, x11_available: x11_available() }
        }
    }
}

/// 改配置文件（要 root），返回配置文件路径。打开（关掉 Wayland）前先确认系统有 X11 会话。
pub fn set(on: bool) -> Result<PathBuf, GdmError> {
    let path = config_path().ok_or(GdmError::NotGdm)?;
    if on && !x11_available() {
        return Err(GdmError::NoX11Session);
    }
    write_config(&path, on).map_err(GdmError::Io)?;
    Ok(path)
}

fn write_config(path: &Path, on: bool) -> Result<(), String> {
    let old = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    let new = set_x11(&old, on);
    if new == old {
        return Ok(());
    }
    let tmp = path.with_extension("conf.lemon-galaxy.tmp");
    std::fs::write(&tmp, new.as_bytes()).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path).map(|m| m.permissions().mode() & 0o7777).unwrap_or(0o644);
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode));
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })?;
    Ok(())
}

fn section_of(line: &str) -> Option<String> {
    let t = line.trim();
    (t.starts_with('[') && t.ends_with(']')).then(|| t[1..t.len() - 1].trim().to_ascii_lowercase())
}

/// 这一行是 `WaylandEnable=…`（不算注释），返回值。
fn active_value(line: &str) -> Option<String> {
    let (k, v) = line.trim().split_once('=')?;
    (k.trim() == "WaylandEnable").then(|| v.trim().to_ascii_lowercase())
}

/// 这一行是被注释掉的 `#WaylandEnable=…`。
fn is_commented(line: &str) -> bool {
    let t = line.trim_start();
    let Some(rest) = t.strip_prefix('#').or_else(|| t.strip_prefix(';')) else { return false };
    rest.trim_start().split_once('=').is_some_and(|(k, _)| k.trim() == "WaylandEnable")
}

fn is_false(v: &str) -> bool {
    matches!(v, "false" | "0" | "no" | "off")
}

/// 配置里是否已关掉 Wayland（登录界面用 X11）。
pub fn x11_forced(conf: &str) -> bool {
    let mut sec = String::new();
    let mut forced = false;
    for l in conf.lines() {
        if let Some(s) = section_of(l) {
            sec = s;
            continue;
        }
        if sec == "daemon" {
            if let Some(v) = active_value(l) {
                // 同一个键写了多次时，GKeyFile 取最后一个
                forced = is_false(&v);
            }
        }
    }
    forced
}

/// 改写配置：`on` 时保证 `[daemon]` 里有 `WaylandEnable=false`，关时把它注释掉（恢复系统默认）。其他行原样保留。
pub fn set_x11(conf: &str, on: bool) -> String {
    let mut lines: Vec<String> = if conf.is_empty() { vec![] } else { conf.split('\n').map(String::from).collect() };
    // split('\n') 在结尾有换行时多出一个空串，最后再拼回去
    let trailing_newline = conf.ends_with('\n');
    if trailing_newline {
        lines.pop();
    }
    let mut appended = false;
    let mut sec = String::new();
    let mut daemon_header: Option<usize> = None;
    let mut actives = vec![];
    let mut commented: Option<usize> = None;
    for (i, l) in lines.iter().enumerate() {
        if let Some(s) = section_of(l) {
            sec = s;
            if sec == "daemon" && daemon_header.is_none() {
                daemon_header = Some(i);
            }
            continue;
        }
        if sec != "daemon" {
            continue;
        }
        if active_value(l).is_some() {
            actives.push(i);
        } else if commented.is_none() && is_commented(l) {
            commented = Some(i);
        }
    }
    if on {
        if !actives.is_empty() {
            for i in actives {
                lines[i] = "WaylandEnable=false".into();
            }
        } else if let Some(i) = commented {
            lines[i] = "WaylandEnable=false".into();
        } else if let Some(h) = daemon_header {
            lines.insert(h + 1, "WaylandEnable=false".into());
        } else {
            if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("[daemon]".into());
            lines.push("WaylandEnable=false".into());
            appended = true;
        }
    } else {
        for i in actives {
            if active_value(&lines[i]).is_some_and(|v| is_false(&v)) {
                lines[i] = "#WaylandEnable=false".into();
            }
        }
    }
    let mut out = lines.join("\n");
    if trailing_newline || appended {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ubuntu 24.04 自带的 /etc/gdm3/custom.conf
    const UBUNTU: &str = "# GDM configuration storage\n#\n# See /usr/share/gdm/gdm.schemas for a list of available options.\n\n[daemon]\nAutomaticLoginEnable=true\nAutomaticLogin=alice\n\n# Uncomment the line below to force the login screen to use Xorg\n#WaylandEnable=false\n\n# Enabling automatic login\n\n[security]\n\n[xdmcp]\n\n[chooser]\n\n[debug]\n# Uncomment the line below to turn on debugging\n# More verbose logs\n# Additionally lets the X server dump core if it crashes\n#Enable=true\n";

    #[test]
    fn uncomments_stock_line_and_keeps_everything_else() {
        assert!(!x11_forced(UBUNTU));
        let on = set_x11(UBUNTU, true);
        assert!(x11_forced(&on));
        assert_eq!(on, UBUNTU.replace("\n#WaylandEnable=false\n", "\nWaylandEnable=false\n"));
        // 关掉恢复原样
        let off = set_x11(&on, false);
        assert!(!x11_forced(&off));
        assert_eq!(off, UBUNTU);
        // 重复设置不变
        assert_eq!(set_x11(&on, true), on);
        assert_eq!(set_x11(UBUNTU, false), UBUNTU);
    }

    #[test]
    fn inserts_after_daemon_header() {
        let c = "[security]\nDisallowTCP=true\n\n[daemon]\nTimedLoginEnable=false\n";
        let on = set_x11(c, true);
        assert_eq!(on, "[security]\nDisallowTCP=true\n\n[daemon]\nWaylandEnable=false\nTimedLoginEnable=false\n");
        assert!(x11_forced(&on));
    }

    #[test]
    fn adds_daemon_section_when_missing() {
        assert_eq!(set_x11("", true), "[daemon]\nWaylandEnable=false\n");
        assert_eq!(set_x11("[security]\n", true), "[security]\n\n[daemon]\nWaylandEnable=false\n");
        assert_eq!(set_x11("[security]", true), "[security]\n\n[daemon]\nWaylandEnable=false\n");
    }

    #[test]
    fn ignores_key_in_other_sections_and_true_values() {
        let c = "[debug]\nWaylandEnable=false\n[daemon]\nWaylandEnable=true\n";
        assert!(!x11_forced(c));
        let on = set_x11(c, true);
        assert_eq!(on, "[debug]\nWaylandEnable=false\n[daemon]\nWaylandEnable=false\n");
        // 关时只注释掉 [daemon] 里值为 false 的行
        assert_eq!(set_x11(&on, false), "[debug]\nWaylandEnable=false\n[daemon]\n#WaylandEnable=false\n");
        assert_eq!(set_x11(c, false), c);
    }

    #[test]
    fn x11_session_check() {
        let d = std::env::temp_dir().join(format!("lg-gdm-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let xs = d.join("xsessions");
        std::fs::create_dir_all(&xs).unwrap();
        let xorg = d.join("Xorg");
        let missing = d.join("nope");
        // 没有 Xorg、没有会话
        assert!(!x11_available_in(&[&xorg], &xs));
        std::fs::write(&xorg, "").unwrap();
        // 有 Xorg、会话目录空（GNOME 49 起的样子）
        assert!(!x11_available_in(&[&missing, &xorg], &xs));
        std::fs::write(xs.join("README"), "").unwrap();
        assert!(!x11_available_in(&[&xorg], &xs));
        std::fs::write(xs.join("xfce.desktop"), "[Desktop Entry]\n").unwrap();
        assert!(x11_available_in(&[&missing, &xorg], &xs));
        // 有会话、没有 Xorg
        assert!(!x11_available_in(&[&missing], &xs));
        // 会话目录不存在
        assert!(!x11_available_in(&[&xorg], &d.join("none")));
        let _ = std::fs::remove_dir_all(&d);
        assert!(GdmError::NoX11Session.message(true).contains("GNOME 49"));
        assert!(GdmError::NoX11Session.message(false).contains("GNOME 49"));
    }

    #[test]
    fn keeps_crlf_free_spacing_and_comments() {
        let c = "[daemon]\n  WaylandEnable = False  \n# note\n";
        assert!(x11_forced(c));
        assert_eq!(set_x11(c, false), "[daemon]\n#WaylandEnable=false\n# note\n");
    }
}
