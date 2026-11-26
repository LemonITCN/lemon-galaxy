//! 非交互命令执行（AI 的 run_command 与用户的快速命令）。

use crate::sysuser::SysUser;
use galaxy_proto::peer::{ExecDone, ExecIn, ExecOut, ExecRequest};
#[cfg(unix)]
use std::process::Stdio;
#[cfg(windows)]
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

/// 执行命令，输出经 `out` 送出，控制消息从 `ctl` 读。
pub async fn run(req: ExecRequest, user: &SysUser, out: mpsc::Sender<ExecOut>, mut ctl: mpsc::Receiver<ExecIn>) {
    let started = Instant::now();
    let shell = req.shell.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| user.shell.clone());
    let spawned = match spawn(&req, user, &shell) {
        Ok(x) => x,
        Err(e) => {
            let _ = out.send(ExecOut::Error(galaxy_proto::peer::PeerError::new("spawn", format!("无法启动 {shell}: {e}")))).await;
            return;
        }
    };
    let Spawned { mut waiter, killer, stdin, stdout: so, stderr: se } = spawned;
    let mut stdin = stdin;
    let _ = out.send(ExecOut::Started).await;
    let max = req.max_output.clamp(4096, 64 * 1024 * 1024) as usize;
    let total = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let truncated = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let pump = |mut r: Box<dyn tokio::io::AsyncRead + Unpin + Send>, err: bool| {
        let out = out.clone();
        let total = total.clone();
        let truncated = truncated.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                match r.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let t = total.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
                        if t >= max {
                            truncated.store(true, std::sync::atomic::Ordering::Relaxed);
                            continue;
                        }
                        let take = n.min(max - t);
                        let d = buf[..take].to_vec();
                        let _ = out.send(if err { ExecOut::Stderr(d) } else { ExecOut::Stdout(d) }).await;
                    }
                }
            }
        })
    };
    let h1 = pump(so, false);
    let h2 = pump(se, true);

    let timeout = Duration::from_millis(req.timeout_ms.clamp(1000, 30 * 60 * 1000));
    let deadline = tokio::time::Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        tokio::select! {
            s = &mut waiter => break s,
            _ = tokio::time::sleep_until(deadline) => {
                timed_out = true;
                killer();
                break waiter.await;
            }
            m = ctl.recv() => match m {
                Some(ExecIn::Stdin(d)) => { if let Some(s) = stdin.as_mut() { let _ = s.write_all(&d).await; } }
                Some(ExecIn::CloseStdin) => { stdin = None; }
                Some(ExecIn::Cancel) | None => {
                    killer();
                    break waiter.await;
                }
            }
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(2), h1).await;
    let _ = tokio::time::timeout(Duration::from_secs(2), h2).await;
    let (code, signal) = status.unwrap_or((None, None));
    let _ = out
        .send(ExecOut::Done(ExecDone {
            code,
            signal,
            timed_out,
            truncated: truncated.load(std::sync::atomic::Ordering::Relaxed),
            duration_ms: started.elapsed().as_millis() as u64,
        }))
        .await;
}

type Waiter = std::pin::Pin<Box<dyn std::future::Future<Output = Option<(Option<i32>, Option<i32>)>> + Send>>;

struct Spawned {
    /// 等进程结束：(退出码, 信号)
    waiter: Waiter,
    /// 结束整棵进程树
    killer: Box<dyn Fn() + Send + Sync>,
    stdin: Option<Box<dyn tokio::io::AsyncWrite + Unpin + Send>>,
    stdout: Box<dyn tokio::io::AsyncRead + Unpin + Send>,
    stderr: Box<dyn tokio::io::AsyncRead + Unpin + Send>,
}

fn expand_cwd(req: &ExecRequest, user: &SysUser) -> String {
    let home = user.home.clone();
    match req.cwd.clone().filter(|c| !c.is_empty()) {
        None => home,
        Some(c) if c == "~" => home,
        Some(c) => match c.strip_prefix("~/") {
            Some(r) => format!("{home}/{r}"),
            None => c,
        },
    }
}

#[cfg(unix)]
fn spawn(req: &ExecRequest, user: &SysUser, shell: &str) -> std::io::Result<Spawned> {
    let base = shell.rsplit(['/', '\\']).next().unwrap_or("sh").to_ascii_lowercase();
    let mut cmd = tokio::process::Command::new(shell);
    if base.starts_with("powershell") || base.starts_with("pwsh") {
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", &req.command]);
    } else if matches!(base.as_str(), "bash" | "zsh" | "fish") {
        cmd.args(["-l", "-c", &req.command]);
    } else {
        cmd.args(["-c", &req.command]);
    }
    cmd.env_clear();
    for (k, v) in crate::sysuser::child_env(user) {
        cmd.env(k, v);
    }
    for (k, v) in quiet_env() {
        cmd.env(k, v);
    }
    for (k, v) in &req.env {
        if safe_env_key(k) {
            cmd.env(k, v);
        }
    }
    cmd.current_dir(expand_cwd(req, user));
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    let switch = user.needs_switch();
    let (uid, gid) = (user.uid, user.gid);
    let groups: Vec<libc::gid_t> = user.groups.iter().map(|g| *g as libc::gid_t).collect();
    unsafe {
        cmd.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if switch {
                #[cfg(not(target_vendor = "apple"))]
                let n = groups.len() as libc::size_t;
                #[cfg(target_vendor = "apple")]
                let n = groups.len() as libc::c_int;
                if libc::setgroups(n, groups.as_ptr()) < 0 || libc::setgid(gid) < 0 || libc::setuid(uid) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn()?;
    let pid = child.id();
    let stdin = child.stdin.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncWrite + Unpin + Send>);
    let stdout = Box::new(child.stdout.take().expect("stdout"));
    let stderr = Box::new(child.stderr.take().expect("stderr"));
    let waiter: Waiter = Box::pin(async move {
        use std::os::unix::process::ExitStatusExt;
        child.wait().await.ok().map(|s| (s.code(), s.signal()))
    });
    let killer = Box::new(move || {
        if let Some(p) = pid {
            unsafe {
                libc::kill(-(p as i32), libc::SIGTERM);
            }
            std::thread::sleep(Duration::from_millis(300));
            unsafe {
                libc::kill(-(p as i32), libc::SIGKILL);
            }
        }
    });
    Ok(Spawned { waiter, killer, stdin, stdout, stderr })
}

#[cfg(windows)]
fn spawn(req: &ExecRequest, user: &SysUser, shell: &str) -> std::io::Result<Spawned> {
    use crate::win;
    let base = shell.rsplit(['/', '\\']).next().unwrap_or("").to_ascii_lowercase();
    let cmdline = if base.starts_with("powershell") || base.starts_with("pwsh") {
        let args = win::powershell_args(&req.command);
        let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        win::cmdline(shell, &refs)
    } else if (base == "cmd" || base == "cmd.exe") && win::cmd_lacks_utf8() {
        // XP 的 cmd 切不到 UTF-8：不切，输出按控制台代码页转（见下面 oem_output）
        format!("{} /D /S /C \"{}\"", win::quote_arg(shell), req.command)
    } else if base == "cmd" || base == "cmd.exe" {
        // /S：去掉最外层引号后原样执行；先切到 UTF-8 代码页
        format!("{} /D /S /C \"chcp 65001>nul & {}\"", win::quote_arg(shell), req.command)
    } else {
        // 其他 shell（如 Git Bash）按 -c 传
        win::cmdline(shell, &["-c", &req.command])
    };
    let token = if user.needs_switch() {
        let acc = win::Account { name: user.name.clone(), domain: user.domain.clone(), sid: user.sid.clone() };
        Some(Arc::new(win::token_for(&acc)?))
    } else {
        None
    };
    let mut extra: Vec<(String, String)> = vec![("LEMON_GALAXY".into(), "1".into())];
    extra.extend(quiet_env());
    for (k, v) in &req.env {
        if safe_env_key(k) && !k.eq_ignore_ascii_case("PATH") {
            extra.push((k.clone(), v.clone()));
        }
    }
    let cwd = crate::files::wire_to_native(&expand_cwd(req, user)).map(|p| p.to_string_lossy().to_string());
    let oem_output = (base == "cmd" || base == "cmd.exe") && win::cmd_lacks_utf8();
    let (child, i, o, e) = win::spawn_piped_ex(cmdline, cwd, token.as_deref(), &extra, oem_output)?;
    let c2 = child.clone();
    let waiter: Waiter = Box::pin(async move {
        let code = c2.wait().await;
        drop(token);
        Some((code, None))
    });
    let killer = Box::new(move || child.kill());
    Ok(Spawned { waiter, killer, stdin: Some(Box::new(i)), stdout: Box::new(o), stderr: Box::new(e) })
}

#[cfg(not(any(unix, windows)))]
fn spawn(_req: &ExecRequest, _user: &SysUser, _shell: &str) -> std::io::Result<Spawned> {
    Err(std::io::Error::other("此平台不支持执行命令"))
}

fn quiet_env() -> Vec<(String, String)> {
    [("TERM", "dumb"), ("PAGER", "cat"), ("GIT_PAGER", "cat"), ("SYSTEMD_PAGER", ""), ("DEBIAN_FRONTEND", "noninteractive")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn safe_env_key(k: &str) -> bool {
    k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !matches!(k, "LD_PRELOAD" | "LD_LIBRARY_PATH" | "DYLD_INSERT_LIBRARIES")
}
