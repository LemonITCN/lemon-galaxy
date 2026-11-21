//! 终端会话：PTY、断线后保留、重连接续、读取屏幕文字（AI 用）。

use crate::sysuser::SysUser;
use galaxy_proto::peer::TerminalSessionInfo;
use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::mpsc;

const RING: usize = 512 * 1024;

pub struct TermSession {
    pub id: String,
    pub title: String,
    pub account: u64,
    pub created_at: i64,
    inner: Mutex<TermInner>,
    #[cfg(unix)]
    master: Arc<tokio::io::unix::AsyncFd<std::os::fd::OwnedFd>>,
    #[cfg(windows)]
    pty: Arc<crate::win::PtyProc>,
    /// 以其他账号运行时的令牌，会话结束才释放（卸载用户配置）
    #[cfg(windows)]
    _token: Option<Arc<crate::win::UserToken>>,
    #[allow(dead_code)]
    pid: i32,
}

struct TermInner {
    ring: VecDeque<u8>,
    parser: vt100::Parser,
    sub: Option<mpsc::Sender<TermEvent>>,
    exited: Option<Option<i32>>,
}

pub enum TermEvent {
    Output(Vec<u8>),
    Exited(Option<i32>),
}

#[derive(Default)]
pub struct TermManager {
    sessions: Mutex<HashMap<String, Arc<TermSession>>>,
}

impl TermManager {
    pub fn list(&self, account: u64) -> Vec<TerminalSessionInfo> {
        let mut v: Vec<_> = self
            .sessions
            .lock()
            .values()
            .filter(|s| s.account == account && s.inner.lock().exited.is_none())
            .map(|s| TerminalSessionInfo { id: s.id.clone(), title: s.title.clone(), created_at: s.created_at, attached: s.inner.lock().sub.is_some() })
            .collect();
        v.sort_by_key(|s| s.created_at);
        v
    }

    pub fn get(&self, id: &str, account: u64) -> Option<Arc<TermSession>> {
        self.sessions.lock().get(id).filter(|s| s.account == account).cloned()
    }

    pub fn kill(&self, id: &str, account: u64) -> bool {
        let s = self.sessions.lock().get(id).filter(|s| s.account == account).cloned();
        match s {
            Some(s) => {
                s.kill();
                self.sessions.lock().remove(id);
                true
            }
            None => false,
        }
    }

    pub fn kill_all(&self) {
        let all: Vec<_> = self.sessions.lock().drain().map(|(_, s)| s).collect();
        for s in all {
            s.kill();
        }
    }

    #[cfg(unix)]
    pub fn spawn(self: &Arc<Self>, user: &SysUser, shell: Option<String>, cols: u16, rows: u16, env: &[(String, String)], account: u64) -> anyhow::Result<Arc<TermSession>> {
        use nix::pty::{openpty, Winsize};
        use std::os::fd::AsRawFd;
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};

        let ws = Winsize { ws_row: rows.max(2), ws_col: cols.max(10), ws_xpixel: 0, ws_ypixel: 0 };
        let pty = openpty(Some(&ws), None)?;
        let shell = shell.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| user.shell.clone());
        let base = shell.rsplit('/').next().unwrap_or("sh").to_string();
        let mut cmd = Command::new(&shell);
        cmd.arg0(format!("-{base}"));
        cmd.env_clear();
        for (k, v) in crate::sysuser::child_env(user) {
            cmd.env(k, v);
        }
        for (k, v) in env {
            if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && k != "PATH" && k != "LD_PRELOAD" && k != "LD_LIBRARY_PATH" && k != "DYLD_INSERT_LIBRARIES" {
                cmd.env(k, v);
            }
        }
        cmd.current_dir(if std::path::Path::new(&user.home).is_dir() { user.home.as_str() } else { "/" });
        let slave_fd = pty.slave;
        cmd.stdin(Stdio::from(slave_fd.try_clone()?));
        cmd.stdout(Stdio::from(slave_fd.try_clone()?));
        cmd.stderr(Stdio::from(slave_fd));
        crate::sysuser::prepare_command(&mut cmd, user, true);
        let child = cmd.spawn()?;
        let pid = child.id() as i32;
        drop(cmd);

        let master = pty.master;
        unsafe {
            let fl = libc::fcntl(master.as_raw_fd(), libc::F_GETFL);
            libc::fcntl(master.as_raw_fd(), libc::F_SETFL, fl | libc::O_NONBLOCK);
        }
        let master = Arc::new(tokio::io::unix::AsyncFd::new(master)?);
        let id: String = format!("{:08x}", rand::random::<u32>());
        let title = format!("{}@{}", user.name, crate::hostname());
        let s = Arc::new(TermSession {
            id: id.clone(),
            title,
            account,
            created_at: galaxy_proto::now_ms(),
            inner: Mutex::new(TermInner { ring: VecDeque::with_capacity(RING), parser: vt100::Parser::new(rows.max(2), cols.max(10), 2000), sub: None, exited: None }),
            master: master.clone(),
            pid,
        });
        self.sessions.lock().insert(id.clone(), s.clone());

        // 读取循环
        let s2 = s.clone();
        let mgr = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut child = child;
            let mut buf = vec![0u8; 32 * 1024];
            loop {
                let mut guard = match s2.master.readable().await {
                    Ok(g) => g,
                    Err(_) => break,
                };
                let r = guard.try_io(|fd| {
                    let n = unsafe { libc::read(fd.get_ref().as_raw_fd(), buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
                    if n < 0 {
                        Err(std::io::Error::last_os_error())
                    } else {
                        Ok(n as usize)
                    }
                });
                match r {
                    Ok(Ok(0)) | Ok(Err(_)) => break,
                    Ok(Ok(n)) => s2.on_output(&buf[..n]).await,
                    Err(_would_block) => continue,
                }
            }
            let code = tokio::task::spawn_blocking(move || child.wait().ok().and_then(|st| st.code())).await.ok().flatten();
            let sub = {
                let mut i = s2.inner.lock();
                i.exited = Some(code);
                i.sub.take()
            };
            if let Some(tx) = sub {
                let _ = tx.send(TermEvent::Exited(code)).await;
            }
            if let Some(m) = mgr.upgrade() {
                m.sessions.lock().remove(&s2.id);
            }
        });
        Ok(s)
    }

    #[cfg(windows)]
    pub fn spawn(self: &Arc<Self>, user: &SysUser, shell: Option<String>, cols: u16, rows: u16, env: &[(String, String)], account: u64) -> anyhow::Result<Arc<TermSession>> {
        let token = if user.needs_switch() {
            let acc = crate::win::Account { name: user.name.clone(), domain: user.domain.clone(), sid: user.sid.clone() };
            Some(Arc::new(crate::win::token_for(&acc).map_err(|e| anyhow::anyhow!("无法以 {} 身份启动终端：{e}", user.display_name()))?))
        } else {
            None
        };
        let shell = shell.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| user.shell.clone());
        let base = shell.rsplit(['/', '\\']).next().unwrap_or("").to_ascii_lowercase();
        let cmdline = if base.starts_with("powershell") || base.starts_with("pwsh") {
            format!("{} -NoLogo", crate::win::quote_arg(&shell))
        } else {
            crate::win::quote_arg(&shell)
        };
        let mut extra: Vec<(String, String)> = vec![("LEMON_GALAXY".into(), "1".into()), ("TERM".into(), "xterm-256color".into())];
        for (k, v) in env {
            if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !k.eq_ignore_ascii_case("PATH") && !k.eq_ignore_ascii_case("PATHEXT") && !k.eq_ignore_ascii_case("COMSPEC") {
                extra.push((k.clone(), v.clone()));
            }
        }
        let (pty, rx) = crate::win::PtyProc::spawn(cmdline, Some(user.home.clone()), token.as_deref(), &extra, cols, rows)?;
        let id: String = format!("{:08x}", rand::random::<u32>());
        let title = format!("{}@{}", user.name, crate::hostname());
        let pid = pty.child.pid as i32;
        let s = Arc::new(TermSession {
            id: id.clone(),
            title,
            account,
            created_at: galaxy_proto::now_ms(),
            inner: Mutex::new(TermInner { ring: VecDeque::with_capacity(RING), parser: vt100::Parser::new(rows.max(2), cols.max(10), 2000), sub: None, exited: None }),
            pty: pty.clone(),
            _token: token,
            pid,
        });
        self.sessions.lock().insert(id.clone(), s.clone());
        let s2 = s.clone();
        let mgr = Arc::downgrade(self);
        let rt = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            while let Ok(data) = rx.recv() {
                rt.block_on(s2.on_output(&data));
            }
            let code = s2.pty.child.wait_blocking();
            let sub = {
                let mut i = s2.inner.lock();
                i.exited = Some(code);
                i.sub.take()
            };
            if let Some(tx) = sub {
                let _ = rt.block_on(tx.send(TermEvent::Exited(code)));
            }
            if let Some(m) = mgr.upgrade() {
                m.sessions.lock().remove(&s2.id);
            }
        });
        Ok(s)
    }

    #[cfg(not(any(unix, windows)))]
    pub fn spawn(self: &Arc<Self>, _user: &SysUser, _shell: Option<String>, _cols: u16, _rows: u16, _env: &[(String, String)], _account: u64) -> anyhow::Result<Arc<TermSession>> {
        anyhow::bail!("此平台的终端尚未实现")
    }
}

impl TermSession {
    async fn on_output(&self, data: &[u8]) {
        let sub = {
            let mut i = self.inner.lock();
            i.parser.process(data);
            for b in data {
                if i.ring.len() >= RING {
                    i.ring.pop_front();
                }
                i.ring.push_back(*b);
            }
            i.sub.clone()
        };
        if let Some(tx) = sub {
            // 主控端处理不过来时阻塞读取，形成背压
            let _ = tx.send(TermEvent::Output(data.to_vec())).await;
        }
    }

    /// 接上会话：返回需要先发给主控端的重放数据，之后的输出经 `tx` 送出。
    pub fn attach(&self, tx: mpsc::Sender<TermEvent>) -> Vec<u8> {
        let mut i = self.inner.lock();
        i.sub = Some(tx);
        if i.parser.screen().alternate_screen() {
            let mut v = b"\x1b[?1049h\x1b[H\x1b[2J".to_vec();
            v.extend_from_slice(&i.parser.screen().contents_formatted());
            v
        } else {
            i.ring.iter().copied().collect()
        }
    }

    pub fn detach(&self) {
        self.inner.lock().sub = None;
    }

    pub fn snapshot(&self) -> String {
        self.inner.lock().parser.screen().contents()
    }

    pub async fn write(&self, data: &[u8]) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let mut off = 0;
            while off < data.len() {
                let mut g = self.master.writable().await?;
                match g.try_io(|fd| {
                    let n = unsafe { libc::write(fd.get_ref().as_raw_fd(), data[off..].as_ptr() as *const libc::c_void, data.len() - off) };
                    if n < 0 {
                        Err(std::io::Error::last_os_error())
                    } else {
                        Ok(n as usize)
                    }
                }) {
                    Ok(Ok(n)) => off += n,
                    Ok(Err(e)) => return Err(e),
                    Err(_) => continue,
                }
            }
            Ok(())
        }
        #[cfg(windows)]
        {
            let p = self.pty.clone();
            let d = data.to_vec();
            tokio::task::spawn_blocking(move || p.write(&d)).await.map_err(std::io::Error::other)?
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = data;
            Ok(())
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let ws = libc::winsize { ws_row: rows.max(2), ws_col: cols.max(10), ws_xpixel: 0, ws_ypixel: 0 };
            unsafe {
                libc::ioctl(self.master.get_ref().as_raw_fd(), libc::TIOCSWINSZ, &ws);
            }
        }
        #[cfg(windows)]
        self.pty.resize(cols, rows);
        self.inner.lock().parser.set_size(rows.max(2), cols.max(10));
    }

    pub fn kill(&self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-self.pid, libc::SIGHUP);
            libc::kill(-self.pid, libc::SIGKILL);
        }
        #[cfg(windows)]
        self.pty.kill();
    }

    pub fn exited(&self) -> Option<Option<i32>> {
        self.inner.lock().exited
    }
}
