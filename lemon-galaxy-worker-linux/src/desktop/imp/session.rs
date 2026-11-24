//! 找「现在屏幕上是谁」：docs/linux-desktop.md 第 3 节。
//!
//! 顺序：logind 的 seat0 活动图形会话 → 没有 logind 时扫 X 服务器进程 → 我们自己开的虚拟桌面。
//! 会话的环境变量（DISPLAY、XAUTHORITY、WAYLAND_DISPLAY、XDG_RUNTIME_DIR、DBUS_SESSION_BUS_ADDRESS）
//! 从这个用户的桌面进程里取；以 root 运行才读得到别的用户的进程。

use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    X11,
    Wayland,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuiSession {
    /// 区分会话用：logind 会话号、`x:<显示号>` 或 `virtual:<用户名>`
    pub id: String,
    pub kind: Kind,
    pub uid: u32,
    pub user: String,
    /// 登录界面（GDM、LightDM、SDDM）
    pub greeter: bool,
    /// 我们开的虚拟桌面
    pub virtual_desktop: bool,
    /// 桌面环境（XDG_CURRENT_DESKTOP 小写，如 gnome、kde、xfce），不知道为空
    pub desktop: String,
    /// 代理要带的环境变量
    pub env: Vec<(String, String)>,
}

impl GuiSession {
    pub fn describe(&self) -> String {
        let kind = match self.kind {
            Kind::X11 => "X11",
            Kind::Wayland => "Wayland",
        };
        let what = if self.virtual_desktop {
            "虚拟桌面"
        } else if self.greeter {
            "登录界面"
        } else {
            "桌面"
        };
        let de = if self.desktop.is_empty() { String::new() } else { format!(" {}", self.desktop) };
        format!("{what}（{kind}{de}，用户 {}，{}）", self.user, self.id)
    }
    pub fn is_gnome(&self) -> bool {
        self.desktop.contains("gnome") || self.desktop.contains("ubuntu") || self.desktop.contains("unity") || self.desktop.contains("pop")
    }
}

/// logind 会话文件 /run/systemd/sessions/<id>
#[derive(Debug, Default, Clone)]
struct LogindSession {
    id: String,
    uid: u32,
    user: String,
    active: bool,
    seat: String,
    kind: String,
    class: String,
    state: String,
    display: String,
    leader: u32,
    remote: bool,
}

fn parse_kv(s: &str) -> HashMap<String, String> {
    s.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect()
}

fn logind_sessions() -> Vec<LogindSession> {
    let Ok(rd) = std::fs::read_dir("/run/systemd/sessions") else { return vec![] };
    let mut v = vec![];
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        // 同目录下还有 <id>.ref 之类的管道文件
        if name.contains('.') {
            continue;
        }
        let Ok(s) = std::fs::read_to_string(e.path()) else { continue };
        let kv = parse_kv(&s);
        let g = |k: &str| kv.get(k).cloned().unwrap_or_default();
        v.push(LogindSession {
            id: name,
            uid: g("UID").parse().unwrap_or(u32::MAX),
            user: g("USER"),
            active: g("ACTIVE") == "1",
            seat: g("SEAT"),
            kind: g("TYPE"),
            class: g("CLASS"),
            state: g("STATE"),
            display: g("DISPLAY"),
            leader: g("LEADER").parse().unwrap_or(0),
            remote: g("REMOTE") == "1",
        });
    }
    v
}

/// 进程信息：名字、属主、环境变量、命令行
struct Proc {
    pid: u32,
    uid: u32,
    comm: String,
    env: HashMap<String, String>,
    cmdline: Vec<String>,
}

fn proc_uid(pid: u32) -> Option<u32> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    s.lines().find_map(|l| l.strip_prefix("Uid:")).and_then(|r| r.split_whitespace().next()).and_then(|x| x.parse().ok())
}

fn read_proc(pid: u32, want_env: bool) -> Option<Proc> {
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?.trim().to_string();
    let uid = proc_uid(pid)?;
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).ok().map(|b| b.split(|c| *c == 0).filter(|x| !x.is_empty()).map(|x| String::from_utf8_lossy(x).to_string()).collect()).unwrap_or_default();
    let env = if want_env {
        std::fs::read(format!("/proc/{pid}/environ"))
            .ok()
            .map(|b| {
                b.split(|c| *c == 0)
                    .filter_map(|kv| {
                        let s = String::from_utf8_lossy(kv);
                        s.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        HashMap::new()
    };
    Some(Proc { pid, uid, comm, env, cmdline })
}

fn all_pids() -> Vec<u32> {
    std::fs::read_dir("/proc").map(|rd| rd.flatten().filter_map(|e| e.file_name().to_str().and_then(|s| s.parse().ok())).collect()).unwrap_or_default()
}

/// 桌面进程名的优先级：越像「桌面本体」越靠前（它们的环境变量最全）。
fn desktop_rank(comm: &str) -> u32 {
    const NAMES: &[&str] = &["gnome-shell", "kwin_wayland", "kwin_x11", "plasmashell", "ksmserver", "xfce4-session", "xfwm4", "mate-session", "marco", "cinnamon", "cinnamon-sessio", "lxqt-session", "lxsession", "openbox", "budgie-wm", "gnome-session-b", "gnome-session-c", "sway", "Hyprland", "labwc", "weston", "xdg-desktop-por", "dbus-daemon"];
    NAMES.iter().position(|n| comm == *n).map(|p| p as u32).unwrap_or(1000)
}

/// 从这个用户的进程里找桌面环境变量；`display` 非空时要求 DISPLAY 对得上（X11 会话）。
fn session_env(uid: u32, kind: Kind, display: &str) -> Option<(HashMap<String, String>, String)> {
    let mut best: Option<(u32, Proc)> = None;
    for pid in all_pids() {
        if proc_uid(pid) != Some(uid) {
            continue;
        }
        let Some(p) = read_proc(pid, true) else { continue };
        let ok = match kind {
            Kind::Wayland => p.env.contains_key("WAYLAND_DISPLAY"),
            Kind::X11 => p.env.get("DISPLAY").is_some_and(|d| display.is_empty() || same_display(d, display)) && !p.env.contains_key("WAYLAND_DISPLAY"),
        };
        if !ok {
            continue;
        }
        let mut rank = desktop_rank(&p.comm);
        // 有会话总线地址的更好
        if !p.env.contains_key("DBUS_SESSION_BUS_ADDRESS") {
            rank += 500;
        }
        if best.as_ref().map(|(r, _)| rank < *r).unwrap_or(true) {
            best = Some((rank, p));
        }
    }
    best.map(|(_, p)| {
        let de = p.env.get("XDG_CURRENT_DESKTOP").cloned().unwrap_or_default().to_lowercase();
        (p.env, de)
    })
}

/// ":0" 与 ":0.0" 是同一个显示
fn same_display(a: &str, b: &str) -> bool {
    let n = |s: &str| s.trim().split('.').next().unwrap_or("").to_string();
    n(a) == n(b)
}

/// X 服务器进程：(显示号, -auth 文件, 进程属主, 是否 Xvfb)
fn x_servers() -> Vec<(String, Option<String>, u32, String)> {
    let mut v = vec![];
    for pid in all_pids() {
        let Some(p) = read_proc(pid, false) else { continue };
        let exe_name = p.cmdline.first().map(|s| s.rsplit('/').next().unwrap_or("").to_string()).unwrap_or_default();
        if !matches!(p.comm.as_str(), "Xorg" | "X" | "Xvfb" | "Xvnc" | "Xephyr") && !matches!(exe_name.as_str(), "Xorg" | "X" | "Xvfb" | "Xvnc" | "Xorg.bin") {
            continue;
        }
        let display = p.cmdline.iter().skip(1).find(|a| a.starts_with(':') && a[1..].chars().all(|c| c.is_ascii_digit() || c == '.')).cloned();
        let auth = p.cmdline.iter().position(|a| a == "-auth").and_then(|i| p.cmdline.get(i + 1)).cloned();
        if let Some(d) = display {
            v.push((d, auth, p.uid, p.comm.clone()));
        }
        let _ = p.pid;
    }
    v.sort();
    v
}

fn user_name(uid: u32) -> String {
    nix::unistd::User::from_uid(uid.into()).ok().flatten().map(|u| u.name).unwrap_or_else(|| uid.to_string())
}

fn user_home(uid: u32) -> String {
    nix::unistd::User::from_uid(uid.into()).ok().flatten().map(|u| u.dir.to_string_lossy().to_string()).unwrap_or_default()
}

const KEEP: &[&str] = &["DISPLAY", "XAUTHORITY", "WAYLAND_DISPLAY", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS", "XDG_CURRENT_DESKTOP", "XDG_SESSION_TYPE", "XDG_SESSION_DESKTOP", "DESKTOP_SESSION", "LANG", "LANGUAGE", "LC_ALL", "PULSE_SERVER", "PIPEWIRE_REMOTE", "GNOME_SHELL_SESSION_MODE"];

fn filtered(env: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = KEEP.iter().filter_map(|k| env.get(*k).map(|x| (k.to_string(), x.clone()))).collect();
    v.sort();
    v
}

fn set(v: &mut Vec<(String, String)>, k: &str, val: String) {
    v.retain(|(a, _)| a != k);
    v.push((k.into(), val));
}

/// X11 会话还缺 XAUTHORITY 时找一个：X 服务器命令行的 -auth、用户家目录的 .Xauthority、GDM 的位置。
fn guess_xauthority(uid: u32, display: &str) -> Option<String> {
    for (d, auth, _, _) in x_servers() {
        if same_display(&d, display) {
            if let Some(a) = auth {
                if Path::new(&a).exists() {
                    return Some(a);
                }
            }
        }
    }
    let candidates = [format!("/run/user/{uid}/gdm/Xauthority"), format!("{}/.Xauthority", user_home(uid))];
    candidates.into_iter().find(|p| Path::new(p).exists())
}

fn from_logind(s: &LogindSession) -> Option<GuiSession> {
    let kind = match s.kind.as_str() {
        "x11" => Kind::X11,
        "wayland" => Kind::Wayland,
        _ => return None,
    };
    let greeter = s.class == "greeter";
    let found = session_env(s.uid, kind, if kind == Kind::X11 { &s.display } else { "" });
    let (env, desktop) = found.unwrap_or_default();
    let mut v = filtered(&env);
    let run = format!("/run/user/{}", s.uid);
    if !v.iter().any(|(k, _)| k == "XDG_RUNTIME_DIR") && Path::new(&run).exists() {
        set(&mut v, "XDG_RUNTIME_DIR", run.clone());
    }
    if !v.iter().any(|(k, _)| k == "DBUS_SESSION_BUS_ADDRESS") && Path::new(&format!("{run}/bus")).exists() {
        set(&mut v, "DBUS_SESSION_BUS_ADDRESS", format!("unix:path={run}/bus"));
    }
    match kind {
        Kind::X11 => {
            let display = v.iter().find(|(k, _)| k == "DISPLAY").map(|(_, d)| d.clone()).filter(|d| !d.is_empty()).or_else(|| (!s.display.is_empty()).then(|| s.display.clone()))?;
            set(&mut v, "DISPLAY", display.clone());
            if !v.iter().any(|(k, _)| k == "XAUTHORITY") {
                if let Some(a) = guess_xauthority(s.uid, &display) {
                    set(&mut v, "XAUTHORITY", a);
                }
            }
        }
        Kind::Wayland => {
            if !v.iter().any(|(k, _)| k == "WAYLAND_DISPLAY") {
                set(&mut v, "WAYLAND_DISPLAY", "wayland-0".into());
            }
        }
    }
    let desktop = if desktop.is_empty() && greeter { "gnome".into() } else { desktop };
    Some(GuiSession { id: s.id.clone(), kind, uid: s.uid, user: s.user.clone(), greeter, virtual_desktop: false, desktop, env: v })
}

/// 现在 seat0 上的图形会话（真屏幕）。没有返回 None。
pub fn find_seat() -> Option<GuiSession> {
    let mine = nix::unistd::getuid().as_raw();
    let root = mine == 0;
    let sessions = logind_sessions();
    if !sessions.is_empty() {
        let mut cands: Vec<&LogindSession> = sessions.iter().filter(|s| s.active && s.seat == "seat0" && !s.remote && matches!(s.kind.as_str(), "x11" | "wayland") && s.state != "closing").collect();
        // 普通用户只能看自己的会话
        if !root {
            cands.retain(|s| s.uid == mine);
        }
        // 用户会话优先于登录界面（切换用户时两者可能同时标为活动）
        cands.sort_by_key(|s| (s.class == "greeter", s.id.clone()));
        if let Some(s) = cands.first() {
            if let Some(g) = from_logind(s) {
                let _ = s.leader;
                return Some(g);
            }
        }
        if root && sessions.iter().any(|s| s.seat == "seat0" && s.active) {
            // seat0 上是文字终端之类：没有图形会话
            return None;
        }
    }
    // 没有 logind：直接找 X 服务器（容器、非 systemd 发行版、用户手动 startx）
    for (display, auth, owner, comm) in x_servers() {
        if crate::desktop::imp::vdesktop::is_ours(&display) {
            continue;
        }
        let uid = if root { owner } else { mine };
        if !root && owner != mine && owner != 0 {
            continue;
        }
        let mut v = vec![("DISPLAY".to_string(), display.clone())];
        // 用户自己的 X 会话：从进程里拿完整环境
        let (env, desktop) = session_env(uid, Kind::X11, &display).unwrap_or_default();
        if !env.is_empty() {
            v = filtered(&env);
            set(&mut v, "DISPLAY", display.clone());
        }
        if !v.iter().any(|(k, _)| k == "XAUTHORITY") {
            if let Some(a) = auth.filter(|a| Path::new(a).exists()) {
                set(&mut v, "XAUTHORITY", a);
            }
        }
        let user = user_name(uid);
        return Some(GuiSession { id: format!("x:{display}"), kind: Kind::X11, uid, user, greeter: false, virtual_desktop: false, desktop: if comm == "Xvfb" && desktop.is_empty() { String::new() } else { desktop }, env: v });
    }
    None
}

/// 普通用户运行时：当前进程自己的图形环境（从桌面里启动的界面、`run` 前台运行）。
pub fn from_own_env() -> Option<GuiSession> {
    let env: HashMap<String, String> = std::env::vars().collect();
    let kind = if env.contains_key("WAYLAND_DISPLAY") {
        Kind::Wayland
    } else if env.contains_key("DISPLAY") {
        Kind::X11
    } else {
        return None;
    };
    let uid = nix::unistd::getuid().as_raw();
    let desktop = env.get("XDG_CURRENT_DESKTOP").cloned().unwrap_or_default().to_lowercase();
    Some(GuiSession { id: "own".into(), kind, uid, user: user_name(uid), greeter: false, virtual_desktop: false, desktop, env: filtered(&env) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kv_and_display() {
        let kv = parse_kv("UID=1000\nUSER=alice\nACTIVE=1\nTYPE=wayland\n");
        assert_eq!(kv["USER"], "alice");
        assert!(same_display(":0", ":0.0"));
        assert!(!same_display(":0", ":1"));
        assert!(desktop_rank("gnome-shell") < desktop_rank("dbus-daemon"));
    }
}
