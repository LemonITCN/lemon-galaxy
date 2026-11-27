//! 自动更新装了新包（deb / rpm 把 /usr/bin/lemon-galaxy-worker-gui 换成了新版本）后，界面自己换成新版本：
//! 发现正在运行的程序文件已被换掉，先试跑新程序的 `--version`，再以同样的方式（窗口开着 / 只在托盘）原地 exec 成新程序。
//! 原地 exec 进程号不变：登录自启动的 systemd 单元、桌面会话都不会以为程序退出了。不弹任何提示。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// 换版本重启时加的参数：新进程据此在会话总线上的名字还没释放时多等一会
pub const RESTARTED: &str = "--restarted";

/// 正在运行的程序文件已被换掉（包管理器是「写新文件再改名」，旧文件已删）：返回那个位置上现在的程序
pub fn replaced_exe() -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let link = std::fs::read_link("/proc/self/exe").ok()?;
    let s = link.to_string_lossy();
    let path = PathBuf::from(s.strip_suffix(" (deleted)").unwrap_or(&s));
    let running = std::fs::metadata("/proc/self/exe").ok()?;
    let disk = std::fs::metadata(&path).ok()?;
    (disk.is_file() && (running.ino() != disk.ino() || running.dev() != disk.dev())).then_some(path)
}

/// 新程序能不能跑：`--version` 十秒内正常退出，返回它报的版本
pub fn probe(exe: &Path) -> Option<String> {
    let mut child = Command::new(exe).arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let t0 = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if t0.elapsed() < Duration::from_secs(10) => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    let v = out.split_whitespace().last()?.to_string();
    status.success().then_some(v)
}

/// 重启用的参数：原来的参数去掉 --tray 与 --restarted；窗口没开着就加 --tray（只放托盘），再加 --restarted
pub fn restart_args(orig: &[String], window_visible: bool) -> Vec<String> {
    let mut v: Vec<String> = orig.iter().filter(|a| *a != "--tray" && *a != RESTARTED).cloned().collect();
    if !window_visible {
        v.push("--tray".into());
    }
    v.push(RESTARTED.into());
    v
}

/// 原地换成新程序（成功不返回）。先把所有打开的文件设成 exec 时关闭：会话总线的连接跟着关掉，
/// 名字马上释放，新程序才能注册成唯一实例。
pub fn exec(exe: &Path, args: &[String]) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    if let Ok(rd) = std::fs::read_dir("/proc/self/fd") {
        let fds: Vec<i32> = rd.flatten().filter_map(|e| e.file_name().to_str().and_then(|s| s.parse().ok())).filter(|&fd| fd > 2).collect();
        for fd in fds {
            unsafe {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags >= 0 {
                    libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
                }
            }
        }
    }
    Command::new(exe).args(args).exec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_keep_mode() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        // 托盘里自启动、窗口没开：还是只放托盘
        assert_eq!(restart_args(&s(&["--tray"]), false), s(&["--tray", RESTARTED]));
        // 托盘里自启动、后来打开了窗口：新版本把窗口也打开
        assert_eq!(restart_args(&s(&["--tray"]), true), s(&[RESTARTED]));
        // 从菜单打开、窗口藏到了托盘
        assert_eq!(restart_args(&[], false), s(&["--tray", RESTARTED]));
        // 已经重启过一次的再换：不会越加越多
        assert_eq!(restart_args(&s(&["--tray", RESTARTED]), true), s(&[RESTARTED]));
    }

    #[test]
    fn running_exe_not_replaced() {
        assert_eq!(replaced_exe(), None);
        assert!(probe(Path::new("/nonexistent/lemon-galaxy-worker-gui")).is_none());
    }
}
