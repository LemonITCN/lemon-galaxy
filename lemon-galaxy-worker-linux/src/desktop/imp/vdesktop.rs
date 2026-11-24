//! 虚拟桌面（docs/linux-desktop.md 第 5 节）：没有可看的图形会话时，给账号绑定的系统用户开一个 Xvfb + 桌面环境。
//!
//! - 每个系统用户最多一个；断开远程后保留，下次接着用。服务重启时从状态文件认回还活着的虚拟桌面。
//! - 运行目录 /run/lemon-galaxy-worker/vd-<uid>/（普通用户运行时在 $XDG_RUNTIME_DIR 下）：Xauthority、日志、状态文件，
//!   也当作虚拟桌面里的 XDG_RUNTIME_DIR（用户没登录时系统不会建 /run/user/<uid>）。
//! - Xvfb 与桌面都以该用户身份运行；Xvfb 开访问控制（独立的 MIT-MAGIC-COOKIE），不监听 TCP。

use super::session::{GuiSession, Kind};
use galaxy_host::sysuser::SysUser;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Saved {
    user: String,
    uid: u32,
    display: String,
    xvfb_pid: u32,
    session_pid: u32,
    desktop: String,
    width: u32,
    height: u32,
}

struct VDesktop {
    saved: Saved,
    xvfb: Option<Child>,
    session: Option<Child>,
    dir: PathBuf,
}

static DESKTOPS: Mutex<Option<HashMap<u32, VDesktop>>> = Mutex::new(None);

fn base_dir() -> PathBuf {
    if nix::unistd::getuid().is_root() {
        PathBuf::from("/run/lemon-galaxy-worker")
    } else {
        std::env::var_os("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join("lemon-galaxy-worker")).unwrap_or_else(|| std::env::temp_dir().join(format!("lemon-galaxy-worker-{}", nix::unistd::getuid())))
    }
}

/// 这个显示号是不是我们开的虚拟桌面（找真屏幕时跳过它们）
pub fn is_ours(display: &str) -> bool {
    let d = display.trim();
    with(|m| m.values().any(|v| v.saved.display == d))
}

fn with<R>(f: impl FnOnce(&mut HashMap<u32, VDesktop>) -> R) -> R {
    let mut g = DESKTOPS.lock();
    if g.is_none() {
        *g = Some(adopt());
    }
    f(g.as_mut().unwrap())
}

fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        // 第三列是状态，Z 是僵尸
        Ok(s) => s.rsplit_once(')').map(|(_, r)| !r.trim_start().starts_with('Z')).unwrap_or(false),
        Err(_) => false,
    }
}

fn alive(v: &mut VDesktop) -> bool {
    let x = match v.xvfb.as_mut() {
        Some(c) => matches!(c.try_wait(), Ok(None)),
        None => pid_alive(v.saved.xvfb_pid),
    };
    let s = match v.session.as_mut() {
        Some(c) => matches!(c.try_wait(), Ok(None)),
        None => pid_alive(v.saved.session_pid),
    };
    x && s
}

/// 服务重启后认回上次开的、还活着的虚拟桌面
fn adopt() -> HashMap<u32, VDesktop> {
    let mut m = HashMap::new();
    let Ok(rd) = std::fs::read_dir(base_dir()) else { return m };
    for e in rd.flatten() {
        if !e.file_name().to_string_lossy().starts_with("vd-") {
            continue;
        }
        let dir = e.path();
        let Ok(b) = std::fs::read(dir.join("state.json")) else { continue };
        let Ok(saved) = serde_json::from_slice::<Saved>(&b) else { continue };
        let mut v = VDesktop { saved, xvfb: None, session: None, dir };
        if alive(&mut v) {
            tracing::info!("认回虚拟桌面 {}（用户 {}）", v.saved.display, v.saved.user);
            m.insert(v.saved.uid, v);
        } else {
            kill(&mut v);
        }
    }
    m
}

fn kill(v: &mut VDesktop) {
    for pid in [v.saved.session_pid, v.saved.xvfb_pid] {
        if pid > 0 {
            // 桌面会话是新会话的首进程：杀整个进程组
            unsafe {
                libc::kill(-(pid as i32), libc::SIGTERM);
                libc::kill(pid as i32, libc::SIGTERM);
            }
        }
    }
    for c in [v.session.as_mut(), v.xvfb.as_mut()].into_iter().flatten() {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(2) {
            if !matches!(c.try_wait(), Ok(None)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = c.kill();
        let _ = c.wait();
    }
    let _ = std::fs::remove_dir_all(&v.dir);
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var("PATH").unwrap_or_default();
    let found = path.split(':').chain(["/usr/bin", "/usr/local/bin", "/bin", "/usr/sbin"]).map(|d| Path::new(d).join(name)).find(|p| p.is_file());
    found
}

/// 可用的桌面会话：(名字, 启动命令, DesktopNames)
fn sessions() -> Vec<(String, String, String)> {
    let mut v = vec![];
    for dir in ["/usr/share/xsessions", "/usr/local/share/xsessions"] {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x != "desktop").unwrap_or(true) {
                continue;
            }
            let Ok(s) = std::fs::read_to_string(&p) else { continue };
            let get = |k: &str| s.lines().find_map(|l| l.strip_prefix(k).map(|v| v.trim().to_string()));
            if get("Hidden=").as_deref() == Some("true") || get("NoDisplay=").as_deref() == Some("true") {
                continue;
            }
            let Some(exec) = get("Exec=") else { continue };
            // 程序得真的装了
            let prog = exec.split_whitespace().find(|w| !w.contains('=') && *w != "env").unwrap_or("").to_string();
            if prog.is_empty() || (!prog.starts_with('/') && which(&prog).is_none()) || (prog.starts_with('/') && !Path::new(&prog).exists()) {
                continue;
            }
            let name = p.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
            v.push((name, exec, get("DesktopNames=").unwrap_or_default().trim_end_matches(';').replace(';', ":")));
        }
    }
    // Xfce、MATE 等轻量桌面在 Xvfb 上最稳；GNOME（Xorg 版）排后面
    let rank = |n: &str| {
        let n = n.to_lowercase();
        const ORDER: &[&str] = &["xfce", "mate", "cinnamon", "lxqt", "lxde", "plasma", "kde", "budgie", "ubuntu-xorg", "gnome-xorg", "gnome", "ubuntu", "openbox", "icewm", "fluxbox", "i3"];
        ORDER.iter().position(|o| n.starts_with(o)).unwrap_or(ORDER.len())
    };
    v.sort_by_key(|(n, _, _)| (rank(n), n.clone()));
    v
}

/// 能不能开虚拟桌面；不能时返回原因（含安装命令）
pub fn availability() -> Result<String, String> {
    let xvfb = which("Xvfb").ok_or_else(|| "没有可用的虚拟桌面：请安装 Xvfb 和一个桌面环境，例如 Ubuntu / Debian：sudo apt install xvfb xfce4；CentOS / RHEL / Fedora：sudo dnf install xorg-x11-server-Xvfb @xfce-desktop-environment".to_string())?;
    let s = sessions();
    let first = s.first().ok_or_else(|| "没有可用的虚拟桌面：没有装桌面环境，例如 Ubuntu / Debian：sudo apt install xfce4；CentOS / RHEL / Fedora：sudo dnf groupinstall Xfce".to_string())?;
    Ok(format!("{}，{}", xvfb.display(), first.0))
}

fn random_cookie() -> [u8; 16] {
    let mut b = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = f.read_exact(&mut b);
    }
    b
}

/// 写 Xauthority 文件：一条通配地址（任何主机名都匹配）的 MIT-MAGIC-COOKIE-1
fn write_xauth(path: &Path, display_num: &str, cookie: &[u8]) -> std::io::Result<()> {
    let mut v = Vec::new();
    let mut field = |b: &[u8]| {
        v.extend_from_slice(&(b.len() as u16).to_be_bytes());
        v.extend_from_slice(b);
    };
    let mut out = 0xffffu16.to_be_bytes().to_vec();
    field(b"");
    field(display_num.as_bytes());
    field(b"MIT-MAGIC-COOKIE-1");
    field(cookie);
    out.extend_from_slice(&v);
    std::fs::write(path, out)
}

fn chown(p: &Path, u: &SysUser) {
    let _ = nix::unistd::chown(p, Some(u.uid.into()), Some(u.gid.into()));
}

fn free_display() -> Option<u32> {
    (90..200).find(|n| !Path::new(&format!("/tmp/.X{n}-lock")).exists() && !Path::new(&format!("/tmp/.X11-unix/X{n}")).exists())
}

/// 已有的虚拟桌面会话（还活着）
pub fn session_for(uid: u32) -> Option<GuiSession> {
    with(|m| {
        let v = m.get_mut(&uid)?;
        if !alive(v) {
            tracing::info!("虚拟桌面 {} 已结束（用户 {}）", v.saved.display, v.saved.user);
            let mut v = m.remove(&uid)?;
            kill(&mut v);
            return None;
        }
        Some(to_session(v))
    })
}

/// 任意一个活着的虚拟桌面（没有指定用户时用）
pub fn any_session() -> Option<GuiSession> {
    let uids: Vec<u32> = with(|m| m.keys().copied().collect());
    uids.into_iter().find_map(session_for)
}

fn to_session(v: &VDesktop) -> GuiSession {
    let s = &v.saved;
    let env = vec![
        ("DISPLAY".to_string(), s.display.clone()),
        ("XAUTHORITY".to_string(), v.dir.join("Xauthority").to_string_lossy().to_string()),
        ("XDG_RUNTIME_DIR".to_string(), v.dir.join("run").to_string_lossy().to_string()),
        ("XDG_SESSION_TYPE".to_string(), "x11".to_string()),
        ("XDG_CURRENT_DESKTOP".to_string(), s.desktop.clone()),
    ];
    GuiSession { id: format!("virtual:{}", s.user), kind: Kind::X11, uid: s.uid, user: s.user.clone(), greeter: false, virtual_desktop: true, desktop: s.desktop.to_lowercase(), env }
}

/// 关掉某个用户的虚拟桌面（该用户在真屏幕上登录了，或者设置关掉了）
pub fn stop(uid: u32) {
    let v = with(|m| m.remove(&uid));
    if let Some(mut v) = v {
        tracing::info!("关闭虚拟桌面 {}（用户 {}）", v.saved.display, v.saved.user);
        kill(&mut v);
    }
}

/// 现有虚拟桌面的用户
pub fn users() -> Vec<u32> {
    with(|m| m.keys().copied().collect())
}

/// 给 `user` 开虚拟桌面（已有就直接返回），分辨率 `w`×`h`。
pub fn ensure(user: &str, w: u32, h: u32) -> Result<GuiSession, String> {
    let u = SysUser::lookup(user).ok_or_else(|| format!("系统用户不存在：{user}"))?;
    if u.uid == 0 && std::env::var("LEMON_GALAXY_ALLOW_ROOT_DESKTOP").map(|v| v != "1").unwrap_or(true) {
        return Err("不给 root 开虚拟桌面（远程拿到 root 图形桌面风险太高）；请把账号绑定到普通系统用户".into());
    }
    let me = nix::unistd::getuid().as_raw();
    if me != 0 && me != u.uid {
        return Err("普通用户运行时只能给自己开虚拟桌面".into());
    }
    if let Some(s) = session_for(u.uid) {
        return Ok(s);
    }
    availability()?;
    let xvfb = which("Xvfb").ok_or("没有 Xvfb")?;
    let (sname, exec, names) = sessions().into_iter().next().ok_or("没有桌面环境")?;
    let num = free_display().ok_or("没有空闲的显示号")?;
    let display = format!(":{num}");
    let dir = base_dir().join(format!("vd-{}", u.uid));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("run")).map_err(|e| format!("建运行目录失败：{e}"))?;
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(base_dir(), std::fs::Permissions::from_mode(0o755));
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        let _ = std::fs::set_permissions(dir.join("run"), std::fs::Permissions::from_mode(0o700));
    }
    let xauth = dir.join("Xauthority");
    write_xauth(&xauth, &num.to_string(), &random_cookie()).map_err(|e| format!("写 Xauthority 失败：{e}"))?;
    for p in [dir.as_path(), &dir.join("run"), &xauth] {
        chown(p, &u);
    }
    let log = |name: &str| std::fs::File::create(dir.join(name)).map(|f| {
        chown(&dir.join(name), &u);
        Stdio::from(f)
    });
    // Xvfb
    let mut cmd = Command::new(&xvfb);
    cmd.args([&display, "-screen", "0", &format!("{w}x{h}x24"), "-nolisten", "tcp", "-auth", &xauth.to_string_lossy(), "-dpi", "96", "+extension", "RANDR", "+extension", "GLX"]);
    cmd.env_clear().envs(galaxy_host::sysuser::child_env(&u)).current_dir(&u.home).stdin(Stdio::null()).stdout(log("xvfb.log").unwrap_or(Stdio::null())).stderr(Stdio::null());
    galaxy_host::sysuser::prepare_command(&mut cmd, &u, false);
    let mut xchild = cmd.spawn().map_err(|e| format!("启动 Xvfb 失败：{e}"))?;
    let sock = format!("/tmp/.X11-unix/X{num}");
    let t = Instant::now();
    while !Path::new(&sock).exists() {
        if t.elapsed() > Duration::from_secs(10) || !matches!(xchild.try_wait(), Ok(None)) {
            let _ = xchild.kill();
            let _ = xchild.wait();
            let detail = std::fs::read_to_string(dir.join("xvfb.log")).unwrap_or_default();
            return Err(format!("Xvfb 没起来：{}", detail.lines().last().unwrap_or("")));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // 桌面会话：有 dbus-run-session 就包一层，给会话一条独立的会话总线
    let wrapped = if which("dbus-run-session").is_some() {
        format!("exec dbus-run-session -- {exec}")
    } else if which("dbus-launch").is_some() {
        format!("exec dbus-launch --exit-with-session {exec}")
    } else {
        format!("exec {exec}")
    };
    let mut cmd = Command::new("/bin/sh");
    cmd.args(["-c", &wrapped]);
    let desktop = if names.is_empty() { sname.clone() } else { names.clone() };
    let mut env = galaxy_host::sysuser::child_env(&u);
    env.retain(|(k, _)| k != "TERM" && k != "COLORTERM" && k != "LEMON_GALAXY");
    env.extend([
        ("DISPLAY".to_string(), display.clone()),
        ("XAUTHORITY".to_string(), xauth.to_string_lossy().to_string()),
        ("XDG_RUNTIME_DIR".to_string(), dir.join("run").to_string_lossy().to_string()),
        ("XDG_SESSION_TYPE".to_string(), "x11".to_string()),
        ("XDG_CURRENT_DESKTOP".to_string(), desktop.clone()),
        ("XDG_SESSION_DESKTOP".to_string(), sname.clone()),
        ("DESKTOP_SESSION".to_string(), sname.clone()),
        ("XDG_SESSION_CLASS".to_string(), "user".to_string()),
    ]);
    cmd.env_clear().envs(env).current_dir(&u.home).stdin(Stdio::null()).stdout(log("session.log").unwrap_or(Stdio::null())).stderr(log("session.err.log").unwrap_or(Stdio::null()));
    galaxy_host::sysuser::prepare_command(&mut cmd, &u, false);
    let schild = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = xchild.kill();
            let _ = xchild.wait();
            return Err(format!("启动桌面会话失败：{e}"));
        }
    };
    let saved = Saved { user: u.name.clone(), uid: u.uid, display: display.clone(), xvfb_pid: xchild.id(), session_pid: schild.id(), desktop: desktop.clone(), width: w, height: h };
    let _ = std::fs::write(dir.join("state.json"), serde_json::to_vec(&saved).unwrap_or_default());
    let msg = format!("已为用户 {} 开虚拟桌面 {}（{}，{}x{}）", u.name, display, sname, w, h);
    tracing::info!("{msg}");
    let v = VDesktop { saved, xvfb: Some(xchild), session: Some(schild), dir };
    let s = to_session(&v);
    with(|m| m.insert(u.uid, v));
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xauth_format() {
        let d = std::env::temp_dir().join(format!("lg-xauth-{}", std::process::id()));
        write_xauth(&d, "91", &[1u8; 16]).unwrap();
        let b = std::fs::read(&d).unwrap();
        let _ = std::fs::remove_file(&d);
        assert_eq!(&b[0..2], &[0xff, 0xff]);
        assert_eq!(&b[2..4], &[0, 0]);
        assert_eq!(&b[4..8], &[0, 2, b'9', b'1']);
        assert_eq!(&b[8..10], &[0, 18]);
        assert_eq!(b.len(), 2 + 2 + 4 + 20 + 18);
    }
}
