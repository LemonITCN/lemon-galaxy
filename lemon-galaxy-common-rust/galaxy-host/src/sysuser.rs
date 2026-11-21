//! 系统用户：远端操作（终端、文件、命令）以被控端为账号指定的系统用户身份运行。

#[derive(Clone, Debug)]
pub struct SysUser {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: String,
    pub shell: String,
    /// 附加组（在父进程里算好，子进程只做系统调用）
    pub groups: Vec<u32>,
    /// Windows：账号所在域（本机账号为计算机名）与 SID；其他系统为空
    pub domain: String,
    pub sid: String,
}

impl SysUser {
    /// 当前进程用户。
    pub fn current() -> Self {
        #[cfg(unix)]
        {
            let uid = nix::unistd::getuid();
            if let Ok(Some(u)) = nix::unistd::User::from_uid(uid) {
                return Self::from_nix(u);
            }
        }
        #[cfg(windows)]
        {
            if let Some(a) = crate::win::current_account() {
                return Self::from_win(a);
            }
        }
        Self {
            name: std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "user".into()),
            uid: 0,
            gid: 0,
            home: std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| "/".into()),
            shell: default_shell(),
            groups: vec![],
            domain: String::new(),
            sid: String::new(),
        }
    }

    #[cfg(windows)]
    fn from_win(a: crate::win::Account) -> Self {
        let home = crate::win::profile_dir(&a.sid).or_else(|| std::env::var("USERPROFILE").ok()).unwrap_or_else(|| "C:\\".into());
        Self { name: a.name, uid: 0, gid: 0, home, shell: default_shell(), groups: vec![], domain: a.domain, sid: a.sid }
    }

    pub fn lookup(name: &str) -> Option<Self> {
        #[cfg(unix)]
        {
            if let Ok(Some(u)) = nix::unistd::User::from_name(name) {
                return Some(Self::from_nix(u));
            }
            None
        }
        #[cfg(windows)]
        {
            crate::win::lookup_account(name).map(Self::from_win)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let c = Self::current();
            if c.name.eq_ignore_ascii_case(name) {
                Some(c)
            } else {
                None
            }
        }
    }

    #[cfg(unix)]
    fn from_nix(u: nix::unistd::User) -> Self {
        let groups = std::ffi::CString::new(u.name.clone())
            .ok()
            .and_then(|c| {
                #[cfg(not(target_vendor = "apple"))]
                {
                    nix::unistd::getgrouplist(&c, u.gid).ok()
                }
                #[cfg(target_vendor = "apple")]
                {
                    let _ = c;
                    None
                }
            })
            .map(|v: Vec<nix::unistd::Gid>| v.into_iter().map(|g| g.as_raw()).collect::<Vec<u32>>())
            .unwrap_or_else(|| vec![u.gid.as_raw()]);
        let shell = u.shell.to_string_lossy().to_string();
        Self {
            name: u.name,
            uid: u.uid.as_raw(),
            gid: u.gid.as_raw(),
            home: u.dir.to_string_lossy().to_string(),
            shell: if shell.is_empty() || shell.ends_with("nologin") || shell.ends_with("false") { default_shell() } else { shell },
            groups,
            domain: String::new(),
            sid: String::new(),
        }
    }

    /// 当前进程是否需要切换到该用户。
    pub fn needs_switch(&self) -> bool {
        #[cfg(unix)]
        {
            nix::unistd::getuid().as_raw() != self.uid
        }
        #[cfg(windows)]
        {
            crate::win::current_account().map(|c| c.sid != self.sid).unwrap_or(false)
        }
        #[cfg(not(any(unix, windows)))]
        {
            false
        }
    }

    /// Windows：「域\用户名」；其他系统就是用户名。
    pub fn display_name(&self) -> String {
        if self.domain.is_empty() || self.domain.eq_ignore_ascii_case("NT AUTHORITY") {
            self.name.clone()
        } else {
            format!("{}\\{}", self.domain, self.name)
        }
    }
}

pub fn default_shell() -> String {
    #[cfg(windows)]
    {
        crate::win::powershell_exe()
    }
    #[cfg(not(windows))]
    if std::path::Path::new("/bin/bash").exists() {
        "/bin/bash".into()
    } else if std::path::Path::new("/bin/zsh").exists() {
        "/bin/zsh".into()
    } else {
        "/bin/sh".into()
    }
}

/// 有管理员权限（Linux/macOS 为 root；Windows 为已提升的管理员或 SYSTEM）。
pub fn is_admin() -> bool {
    #[cfg(windows)]
    {
        crate::win::is_elevated() || crate::win::is_system()
    }
    #[cfg(not(windows))]
    {
        is_root()
    }
}

/// 以最高权限账号运行（Linux/macOS 为 root，Windows 为 SYSTEM），能替其他账号启动进程。
pub fn is_root() -> bool {
    #[cfg(unix)]
    {
        nix::unistd::getuid().is_root()
    }
    #[cfg(windows)]
    {
        crate::win::is_system()
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

/// 子进程环境变量。
pub fn child_env(u: &SysUser) -> Vec<(String, String)> {
    let path = if cfg!(target_os = "macos") {
        "/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
    } else {
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/snap/bin"
    };
    let lang = std::env::var("LANG").ok().filter(|l| l.contains("UTF-8") || l.contains("utf8")).unwrap_or_else(|| "C.UTF-8".into());
    vec![
        ("HOME".into(), u.home.clone()),
        ("USER".into(), u.name.clone()),
        ("LOGNAME".into(), u.name.clone()),
        ("SHELL".into(), u.shell.clone()),
        ("PATH".into(), path.into()),
        ("LANG".into(), lang),
        ("TERM".into(), "xterm-256color".into()),
        ("COLORTERM".into(), "truecolor".into()),
        ("LEMON_GALAXY".into(), "1".into()),
    ]
}

/// 给 Command 加上「新会话 + 切换用户」，只在子进程里做系统调用。
#[cfg(unix)]
pub fn prepare_command(cmd: &mut std::process::Command, u: &SysUser, controlling_tty: bool) {
    use std::os::unix::process::CommandExt;
    let switch = u.needs_switch();
    let (uid, gid) = (u.uid, u.gid);
    let groups: Vec<libc::gid_t> = u.groups.iter().map(|g| *g as libc::gid_t).collect();
    unsafe {
        cmd.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if controlling_tty {
                // 标准输入已是从端，设为控制终端
                #[allow(clippy::useless_conversion)]
                if libc::ioctl(0, libc::TIOCSCTTY.into(), 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            if switch {
                #[cfg(not(target_vendor = "apple"))]
                let n = groups.len() as libc::size_t;
                #[cfg(target_vendor = "apple")]
                let n = groups.len() as libc::c_int;
                if libc::setgroups(n, groups.as_ptr()) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::setgid(gid) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::setuid(uid) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
}
