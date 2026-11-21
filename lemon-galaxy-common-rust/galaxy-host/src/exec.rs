//! 非交互命令执行（AI 的 run_command 与用户的快速命令）。

use crate::sysuser::SysUser;
use galaxy_proto::peer::{ExecDone, ExecIn, ExecOut, ExecRequest};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

/// 执行命令，输出经 `out` 送出，控制消息从 `ctl` 读。
pub async fn run(req: ExecRequest, user: &SysUser, out: mpsc::Sender<ExecOut>, mut ctl: mpsc::Receiver<ExecIn>) {
    let started = Instant::now();
    let shell = req.shell.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| user.shell.clone());
    let base = shell.rsplit(['/', '\\']).next().unwrap_or("sh").to_ascii_lowercase();
    let mut cmd = tokio::process::Command::new(&shell);
    if base.starts_with("powershell") || base.starts_with("pwsh") {
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", &req.command]);
    } else if base == "cmd" || base == "cmd.exe" {
        cmd.args(["/C", &req.command]);
    } else if matches!(base.as_str(), "bash" | "zsh" | "fish") {
        cmd.args(["-l", "-c", &req.command]);
    } else {
        cmd.args(["-c", &req.command]);
    }
    cmd.env_clear();
    for (k, v) in crate::sysuser::child_env(user) {
        cmd.env(k, v);
    }
    cmd.env("TERM", "dumb");
    cmd.env("PAGER", "cat");
    cmd.env("GIT_PAGER", "cat");
    cmd.env("SYSTEMD_PAGER", "");
    cmd.env("DEBIAN_FRONTEND", "noninteractive");
    for (k, v) in &req.env {
        if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !matches!(k.as_str(), "LD_PRELOAD" | "LD_LIBRARY_PATH" | "DYLD_INSERT_LIBRARIES") {
            cmd.env(k, v);
        }
    }
    let cwd = req.cwd.clone().filter(|c| !c.is_empty()).map(|c| if let Some(r) = c.strip_prefix("~/") { format!("{}/{r}", user.home) } else if c == "~" { user.home.clone() } else { c });
    cmd.current_dir(cwd.unwrap_or_else(|| user.home.clone()));
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    #[cfg(unix)]
    {
        let switch = user.needs_switch();
        let (uid, gid) = (user.uid, user.gid);
        let groups: Vec<libc::gid_t> = user.groups.iter().map(|g| *g as libc::gid_t).collect();
        unsafe {
            cmd.pre_exec(move || {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if switch {
                    #[cfg(not(target_os = "macos"))]
                    let n = groups.len() as libc::size_t;
                    #[cfg(target_os = "macos")]
                    let n = groups.len() as libc::c_int;
                    if libc::setgroups(n, groups.as_ptr()) < 0 || libc::setgid(gid) < 0 || libc::setuid(uid) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = out.send(ExecOut::Error(galaxy_proto::peer::PeerError::new("spawn", format!("无法启动 {shell}: {e}")))).await;
            return;
        }
    };
    let pid = child.id();
    let _ = out.send(ExecOut::Started).await;
    let mut stdin = child.stdin.take();
    let so = child.stdout.take().expect("stdout");
    let se = child.stderr.take().expect("stderr");
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
    let h1 = pump(Box::new(so), false);
    let h2 = pump(Box::new(se), true);

    let timeout = Duration::from_millis(req.timeout_ms.clamp(1000, 30 * 60 * 1000));
    let deadline = tokio::time::Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        tokio::select! {
            s = child.wait() => break s.ok(),
            _ = tokio::time::sleep_until(deadline) => {
                timed_out = true;
                kill_tree(pid);
                break child.wait().await.ok();
            }
            m = ctl.recv() => match m {
                Some(ExecIn::Stdin(d)) => { if let Some(s) = stdin.as_mut() { let _ = s.write_all(&d).await; } }
                Some(ExecIn::CloseStdin) => { stdin = None; }
                Some(ExecIn::Cancel) | None => {
                    kill_tree(pid);
                    break child.wait().await.ok();
                }
            }
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(2), h1).await;
    let _ = tokio::time::timeout(Duration::from_secs(2), h2).await;
    #[cfg(unix)]
    let signal = status.and_then(|s| std::os::unix::process::ExitStatusExt::signal(&s));
    #[cfg(not(unix))]
    let signal = None;
    let _ = out
        .send(ExecOut::Done(ExecDone {
            code: status.and_then(|s| s.code()),
            signal,
            timed_out,
            truncated: truncated.load(std::sync::atomic::Ordering::Relaxed),
            duration_ms: started.elapsed().as_millis() as u64,
        }))
        .await;
}

fn kill_tree(pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(p) = pid {
        unsafe {
            libc::kill(-(p as i32), libc::SIGTERM);
        }
        std::thread::sleep(Duration::from_millis(300));
        unsafe {
            libc::kill(-(p as i32), libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
}

