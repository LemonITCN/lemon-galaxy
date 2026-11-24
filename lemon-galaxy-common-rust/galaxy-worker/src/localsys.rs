//! 本机控制接口鉴权要用的系统信息：屏幕前的图形会话（logind）、管理员组、系统用户列表、polkit。

use std::path::Path;

/// logind 会话文件 `/run/systemd/sessions/<id>` 里用得到的键。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LogindSession {
    pub uid: u32,
    pub user: String,
    pub seat: String,
    pub active: bool,
    pub kind: String,
    pub class: String,
}

impl LogindSession {
    /// seat0 上亮着的图形会话（登录界面不算）
    pub fn is_graphical_foreground(&self) -> bool {
        self.seat == "seat0" && self.active && matches!(self.kind.as_str(), "x11" | "wayland" | "mir") && self.class != "greeter"
    }
}

pub fn parse_logind_session(text: &str) -> Option<LogindSession> {
    let mut s = LogindSession::default();
    let mut has_uid = false;
    for l in text.lines() {
        let Some((k, v)) = l.split_once('=') else { continue };
        let v = v.trim();
        match k.trim() {
            "UID" => {
                s.uid = v.parse().ok()?;
                has_uid = true;
            }
            "USER" => s.user = v.into(),
            "SEAT" => s.seat = v.into(),
            "ACTIVE" => s.active = v == "1" || v == "yes",
            "TYPE" => s.kind = v.into(),
            "CLASS" => s.class = v.into(),
            _ => {}
        }
    }
    has_uid.then_some(s)
}

/// 屏幕前图形会话的用户（logind）。没有 logind 时为空。
pub fn graphical_uids() -> Vec<u32> {
    graphical_uids_in(Path::new("/run/systemd/sessions"))
}

pub fn graphical_uids_in(dir: &Path) -> Vec<u32> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut v = vec![];
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        // 目录里还有 *.ref 管道文件
        if name.contains('.') {
            continue;
        }
        if let Some(s) = std::fs::read_to_string(e.path()).ok().and_then(|t| parse_logind_session(&t)) {
            if s.is_graphical_foreground() && !v.contains(&s.uid) {
                v.push(s.uid);
            }
        }
    }
    v
}

/// `/proc/<pid>/stat` 第 22 列：进程启动时间（开机后的时钟滴答数）。polkit 用它防进程号复用。
pub fn parse_proc_start_time(stat: &str) -> Option<u64> {
    // 第 2 列是带括号的进程名，里面可能有空格和括号：从最后一个 ')' 之后数
    let rest = &stat[stat.rfind(')')? + 1..];
    // rest 从第 3 列开始，第 22 列是其中第 20 个（下标 19）
    rest.split_whitespace().nth(19)?.parse().ok()
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn proc_start_time(pid: u32) -> Option<u64> {
    parse_proc_start_time(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// 管理员组
pub const ADMIN_GROUPS: [&str; 3] = ["sudo", "wheel", "admin"];

#[cfg(unix)]
pub fn user_name(uid: u32) -> String {
    nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid)).ok().flatten().map(|u| u.name).unwrap_or_else(|| uid.to_string())
}

/// 用户是否在 sudo / wheel / admin 组里。
#[cfg(unix)]
pub fn in_admin_group(uid: u32) -> bool {
    let Some(u) = galaxy_host::sysuser::SysUser::lookup(&user_name(uid)) else { return false };
    for g in ADMIN_GROUPS {
        if let Ok(Some(gr)) = nix::unistd::Group::from_name(g) {
            if u.groups.contains(&gr.gid.as_raw()) || u.gid == gr.gid.as_raw() || gr.mem.iter().any(|m| m == &u.name) {
                return true;
            }
        }
    }
    false
}

/// 能登录的系统用户（root 与 uid 1000 起、有登录 shell 的），给权限里的「以哪个系统用户运行」选。
pub fn login_users() -> Vec<String> {
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    let mut v = parse_login_users(&passwd);
    #[cfg(unix)]
    {
        // macOS 等 /etc/passwd 里没有普通用户：至少有当前用户
        let me = galaxy_host::sysuser::SysUser::current().name;
        if !v.contains(&me) {
            v.insert(0, me);
        }
    }
    v
}

pub fn parse_login_users(passwd: &str) -> Vec<String> {
    let mut users = vec![];
    let mut has_root = false;
    for l in passwd.lines() {
        let f: Vec<&str> = l.split(':').collect();
        if f.len() < 7 || l.starts_with('#') {
            continue;
        }
        let Ok(uid) = f[2].parse::<u32>() else { continue };
        let shell = f[6].trim();
        let nologin = shell.is_empty() || shell.ends_with("nologin") || shell.ends_with("/false");
        if uid == 0 {
            has_root = true;
        } else if (1000..60000).contains(&uid) && !nologin && !users.iter().any(|u| u == f[0]) {
            users.push(f[0].to_string());
        }
    }
    users.sort();
    if has_root {
        users.push("root".into());
    }
    users
}

/// polkit 授权结果
#[derive(Debug)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub enum PolkitResult {
    Authorized,
    /// 没授权；`dismissed` = 用户取消了密码框
    Denied { dismissed: bool, challenge: bool },
    /// 没有 polkit（没有系统总线、没装、动作文件没装）
    Unavailable(String),
}

/// 问 polkit：这个进程能不能做「管理 LemonGalaxy 被控端」。允许弹密码框（会等到用户输完）。
#[cfg(target_os = "linux")]
pub async fn polkit_check(pid: u32, uid: u32, action: &str) -> PolkitResult {
    use std::collections::HashMap;
    use zbus::zvariant::Value;
    let Some(start) = proc_start_time(pid) else {
        return PolkitResult::Unavailable(format!("/proc/{pid}/stat"));
    };
    let conn = match zbus::Connection::system().await {
        Ok(c) => c,
        Err(e) => return PolkitResult::Unavailable(e.to_string()),
    };
    let proxy = match zbus::Proxy::new(&conn, "org.freedesktop.PolicyKit1", "/org/freedesktop/PolicyKit1/Authority", "org.freedesktop.PolicyKit1.Authority").await {
        Ok(p) => p,
        Err(e) => return PolkitResult::Unavailable(e.to_string()),
    };
    let mut sd: HashMap<&str, Value> = HashMap::new();
    sd.insert("pid", Value::U32(pid));
    sd.insert("start-time", Value::U64(start));
    sd.insert("uid", Value::I32(uid as i32));
    let subject = ("unix-process", sd);
    let details: HashMap<&str, &str> = HashMap::new();
    // flags 1 = AllowUserInteraction
    let r: zbus::Result<(bool, bool, HashMap<String, String>)> = proxy.call("CheckAuthorization", &(subject, action, details, 1u32, "")).await;
    match r {
        Ok((true, _, _)) => PolkitResult::Authorized,
        Ok((false, challenge, d)) => PolkitResult::Denied { dismissed: d.get("polkit.dismissed").is_some_and(|v| v == "true"), challenge },
        Err(e) => PolkitResult::Unavailable(e.to_string()),
    }
}

#[cfg(not(target_os = "linux"))]
pub async fn polkit_check(_pid: u32, _uid: u32, _action: &str) -> PolkitResult {
    PolkitResult::Unavailable("polkit is Linux only".into())
}

/// 系统总线在不在（粗判 polkit 能不能用）
pub fn system_bus_present() -> bool {
    cfg!(target_os = "linux") && (Path::new("/run/dbus/system_bus_socket").exists() || Path::new("/var/run/dbus/system_bus_socket").exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logind_session_file() {
        let t = "# This is private data. Do not parse.\nUID=1000\nUSER=alice\nACTIVE=1\nIS_DISPLAY=1\nSTATE=active\nREMOTE=0\nTYPE=wayland\nORIGINAL_TYPE=wayland\nCLASS=user\nSEAT=seat0\nVTNR=2\nDISPLAY=:0\nLEADER=1234\n";
        let s = parse_logind_session(t).unwrap();
        assert_eq!(s.uid, 1000);
        assert_eq!(s.user, "alice");
        assert!(s.is_graphical_foreground());
        let tty = t.replace("TYPE=wayland", "TYPE=tty");
        assert!(!parse_logind_session(&tty).unwrap().is_graphical_foreground());
        let bg = t.replace("ACTIVE=1", "ACTIVE=0");
        assert!(!parse_logind_session(&bg).unwrap().is_graphical_foreground());
        let ssh = "UID=1001\nUSER=bob\nACTIVE=1\nTYPE=tty\nCLASS=user\nREMOTE=1\n";
        assert!(!parse_logind_session(ssh).unwrap().is_graphical_foreground());
        let greeter = "UID=120\nUSER=gdm\nACTIVE=1\nTYPE=wayland\nCLASS=greeter\nSEAT=seat0\n";
        assert!(!parse_logind_session(greeter).unwrap().is_graphical_foreground());
        assert!(parse_logind_session("USER=x\n").is_none());
    }

    #[test]
    fn logind_dir() {
        let d = std::env::temp_dir().join(format!("lg-logind-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("2"), "UID=1000\nACTIVE=1\nTYPE=x11\nCLASS=user\nSEAT=seat0\n").unwrap();
        std::fs::write(d.join("3"), "UID=1001\nACTIVE=1\nTYPE=tty\nCLASS=user\n").unwrap();
        std::fs::write(d.join("2.ref"), "").unwrap();
        assert_eq!(graphical_uids_in(&d), vec![1000]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn proc_stat_start_time() {
        let stat = "4242 (my (odd) prog) S 1 4242 4242 0 -1 4194560 1234 0 0 0 10 5 0 0 20 0 3 0 987654 123456789 456 18446744073709551615 1 1 0 0 0 0 0 4096 0 0 0 0 17 3 0 0 0 0 0";
        assert_eq!(parse_proc_start_time(stat), Some(987654));
        assert_eq!(parse_proc_start_time("garbage"), None);
    }

    #[test]
    fn passwd_users() {
        let p = "root:x:0:0:root:/root:/bin/bash\ndaemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin\nalice:x:1000:1000:Alice:/home/alice:/bin/bash\nnobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin\nbob:x:1001:1001::/home/bob:/bin/zsh\nsvc:x:1002:1002::/srv:/bin/false\n";
        assert_eq!(parse_login_users(p), vec!["alice", "bob", "root"]);
    }
}
