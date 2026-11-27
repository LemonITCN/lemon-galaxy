//! 更新进度：数据目录下 `update/state.json`。被控服务（新旧两个版本）与更新守护进程都读写它，
//! 改之前拿 `update/state.lock`（独占新建文件，进程没了留下的锁 30 秒后作废），读改写不会互相覆盖。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 最多记几个起不来的版本
const MAX_BAD: usize = 20;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// 没有进行中的更新
    #[default]
    Idle,
    /// 包管理器正在装（Linux deb / rpm，由更新守护进程执行，装完由包脚本重启服务）
    Installing,
    /// 新版本已换上，等它连上服务器、稳定运行一段时间
    Trial,
    /// 守护进程正在换回旧版本
    RollingBack,
    /// 新版本稳定运行，已确认
    Committed,
    /// 新版本起不来或一直连不上服务器，已换回旧版本（这个版本记进 bad，不再自动安装）
    RolledBack,
    /// 没装成（包管理器失败、版本被手动换掉等），已恢复或放弃；不算坏版本
    Failed,
}

impl Phase {
    /// 更新还没有结论（守护进程要接着看着）
    pub fn pending(self) -> bool {
        matches!(self, Phase::Installing | Phase::Trial | Phase::RollingBack)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// 只换核心程序（只装了命令行，或服务器没有桌面端的包）
    #[default]
    Core,
    /// Windows 桌面端：安装目录里的界面、被控服务、资源整套换
    WindowsDesktop,
    /// Linux deb / rpm：交给包管理器
    LinuxPackage,
}

/// 被换掉的一个文件。`backup` 为空表示这是新版本新加的文件，回滚时删掉。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replaced {
    pub target: PathBuf,
    #[serde(default)]
    pub backup: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub phase: Phase,
    #[serde(default)]
    pub kind: Kind,
    /// 换之前的版本（守护进程就是这个版本的程序）
    #[serde(default)]
    pub from: String,
    /// 要换上的版本
    #[serde(default)]
    pub to: String,
    /// 开始换的时间（毫秒）
    #[serde(default)]
    pub applied_at: i64,
    /// 新版本要在这之前连上服务器并稳定运行，否则换回（毫秒；0 = 包还在装，还没开始计时）
    #[serde(default)]
    pub deadline: i64,
    /// 试运行时长（秒），包装好后守护进程按它定 deadline
    #[serde(default)]
    pub trial_secs: u64,
    /// 新版本启动了几次（每次启动最早的时候加一）
    #[serde(default)]
    pub starts: u32,
    #[serde(default)]
    pub files: Vec<Replaced>,
    /// Linux：要装的 deb / rpm
    #[serde(default)]
    pub package: Option<PathBuf>,
    /// Linux：deb 或 rpm
    #[serde(default)]
    pub package_format: String,
    /// 更新守护进程的程序（旧版本的副本）；守护进程不在时新版本据此再拉起它
    #[serde(default)]
    pub guard: Option<PathBuf>,
    /// 回滚 / 失败的原因
    #[serde(default)]
    pub reason: String,
    /// 正在换回的这个版本要不要记成坏版本（换回做到一半守护进程没了，接手的守护进程照此做完）
    #[serde(default)]
    pub rollback_bad: bool,
    /// 结论的时间（毫秒）
    #[serde(default)]
    pub finished_at: i64,
    /// 最近确认过的桌面端版本（换整套后界面文件的版本读不出来时，用它判断界面是不是已是这个版本）
    #[serde(default)]
    pub desktop_version: String,
    /// 装上后起不来的版本，不再自动安装（有更新的版本时照常装更新的）
    #[serde(default)]
    pub bad: Vec<String>,
}

pub fn update_dir(data: &Path) -> PathBuf {
    data.join("update")
}

fn state_path(data: &Path) -> PathBuf {
    update_dir(data).join("state.json")
}

impl State {
    pub fn load(data: &Path) -> State {
        std::fs::read(state_path(data)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn save(&self, data: &Path) -> std::io::Result<()> {
        let p = state_path(data);
        std::fs::create_dir_all(update_dir(data))?;
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap_or_default())?;
        std::fs::rename(&tmp, &p)
    }

    pub fn is_bad(&self, version: &str) -> bool {
        self.bad.iter().any(|v| v == version)
    }

    pub fn mark_bad(&mut self, version: &str) {
        if !self.is_bad(version) {
            self.bad.push(version.to_string());
        }
        let n = self.bad.len();
        if n > MAX_BAD {
            self.bad.drain(..n - MAX_BAD);
        }
    }

    /// 拿着锁读、改、写。`f` 返回 false 时不写。返回 `f` 的结果（拿不到锁、写失败时为 false）。
    pub fn update(data: &Path, f: impl FnOnce(&mut State) -> bool) -> bool {
        let Some(_lock) = Lock::acquire(data) else {
            tracing::warn!("更新：拿不到 state.lock");
            return false;
        };
        let mut s = State::load(data);
        if !f(&mut s) {
            return false;
        }
        match s.save(data) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("更新：写 state.json 失败：{e}");
                false
            }
        }
    }
}

/// 跨进程的锁：独占新建 `update/state.lock`，用完删掉。
struct Lock(PathBuf);

impl Lock {
    fn acquire(data: &Path) -> Option<Lock> {
        let dir = update_dir(data);
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("state.lock");
        for i in 0..200 {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&p) {
                Ok(_) => return Some(Lock(p)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // 拿着锁的进程没了：30 秒前的锁作废
                    let stale = std::fs::metadata(&p).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d > Duration::from_secs(30));
                    if stale {
                        let _ = std::fs::remove_file(&p);
                        continue;
                    }
                }
                Err(_) if i > 3 => return None,
                Err(_) => {}
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        None
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// 更新守护进程的心跳：`update/guard.alive` 里写「进程号 毫秒时间」，每两秒一次。
pub fn heartbeat(data: &Path) {
    let _ = std::fs::create_dir_all(update_dir(data));
    let p = update_dir(data).join("guard.alive");
    let tmp = p.with_extension("tmp");
    if std::fs::write(&tmp, format!("{} {}", std::process::id(), galaxy_proto::now_ms())).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

/// 有没有别的更新守护进程在跑（15 秒内有心跳，且不是自己）
pub fn guard_alive(data: &Path) -> bool {
    let Ok(s) = std::fs::read_to_string(update_dir(data).join("guard.alive")) else { return false };
    let mut it = s.split_whitespace();
    let pid: u32 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let at: i64 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    pid != std::process::id() && (galaxy_proto::now_ms() - at).abs() < 15_000
}

pub fn clear_heartbeat(data: &Path) {
    let p = update_dir(data).join("guard.alive");
    // 只删自己的
    if std::fs::read_to_string(&p).ok().and_then(|s| s.split_whitespace().next().and_then(|v| v.parse::<u32>().ok())) == Some(std::process::id()) {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lg-upd-state-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn roundtrip_and_update() {
        let d = tmp("rt");
        assert_eq!(State::load(&d), State::default());
        assert!(State::update(&d, |s| {
            s.phase = Phase::Trial;
            s.from = "0.1.0.1".into();
            s.to = "0.1.0.2".into();
            true
        }));
        let s = State::load(&d);
        assert_eq!((s.phase, s.from.as_str(), s.to.as_str()), (Phase::Trial, "0.1.0.1", "0.1.0.2"));
        // f 返回 false 不写
        assert!(!State::update(&d, |s| {
            s.phase = Phase::Committed;
            false
        }));
        assert_eq!(State::load(&d).phase, Phase::Trial);
        // 锁用完就删了
        assert!(!update_dir(&d).join("state.lock").exists());
        // 坏文件当作空
        std::fs::write(update_dir(&d).join("state.json"), b"{broken").unwrap();
        assert_eq!(State::load(&d).phase, Phase::Idle);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn stale_lock_is_broken() {
        let d = tmp("lock");
        std::fs::create_dir_all(update_dir(&d)).unwrap();
        let lock = update_dir(&d).join("state.lock");
        std::fs::write(&lock, b"").unwrap();
        // 新鲜的锁：等不到
        let t = std::time::Instant::now();
        assert!(Lock::acquire(&d).is_none());
        assert!(t.elapsed() >= Duration::from_secs(4));
        // 改成 1 分钟前的：作废
        let old = std::time::SystemTime::now() - Duration::from_secs(60);
        std::fs::File::options().write(true).open(&lock).unwrap().set_modified(old).unwrap();
        assert!(Lock::acquire(&d).is_some());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn bad_versions_are_bounded() {
        let mut s = State::default();
        for i in 0..30 {
            s.mark_bad(&format!("0.1.0.{i}"));
            s.mark_bad(&format!("0.1.0.{i}"));
        }
        assert_eq!(s.bad.len(), MAX_BAD);
        assert!(s.is_bad("0.1.0.29") && !s.is_bad("0.1.0.0"));
    }

    #[test]
    fn heartbeat_of_self_is_not_another_guard() {
        let d = tmp("hb");
        assert!(!guard_alive(&d));
        heartbeat(&d);
        // 自己的心跳不算「别的守护进程」
        assert!(!guard_alive(&d));
        std::fs::write(update_dir(&d).join("guard.alive"), format!("{} {}", std::process::id() + 1, galaxy_proto::now_ms())).unwrap();
        assert!(guard_alive(&d));
        std::fs::write(update_dir(&d).join("guard.alive"), format!("{} {}", std::process::id() + 1, galaxy_proto::now_ms() - 60_000)).unwrap();
        assert!(!guard_alive(&d));
        let _ = std::fs::remove_dir_all(&d);
    }
}
