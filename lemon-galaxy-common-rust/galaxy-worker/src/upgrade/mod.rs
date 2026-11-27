//! 被控端自动更新：检查、下载、换版本、重启、确认、回滚（docs/auto-update.md）。
//!
//! 定时自动更新（服务模式，默认开）与界面「立即更新」（本机控制接口 ApplyUpdate）走同一条路，全程不需要任何人操作：
//!
//! 1. 检查：装了桌面端的机器问 `worker-desktop`（Windows 压缩包 / Linux deb、rpm），服务器没有时退回只问 `worker`
//!    （只换核心程序，界面下次再换）；以前装上后起不来的版本（state.json 的 bad）跳过。
//! 2. 下载到数据目录 `update/staging`（galaxy-client 验发布签名与 SHA-256），解开，先跑一下新程序的 `version`：起不来的不装。
//! 3. 备份要换掉的文件（`update/backup`），把当前（旧）程序复制成更新守护进程（`update/guard`），写 `state.json`
//!    （试运行、截止时间），在独立的进程里启动守护进程（Linux 放进单独的 systemd 单元，Windows 是脱离的子进程）。
//! 4. 换文件（Windows 上运行中的程序改名挪开）后以退出码 75 退出，由服务管理器拉起新版本；
//!    Linux 安装包则由守护进程调用 dpkg / rpm 装，包脚本重启服务。
//! 5. 新版本在启动最早的时候把启动次数加一；连上服务器并稳定运行一段时间后确认，删掉备份。
//! 6. 守护进程看着：启动了 4 次还没确认、或到了截止时间还没确认，就停服务、按备份换回、记下坏版本、再启动服务。

pub mod files;
pub mod pkg;
pub(crate) mod state;
pub(crate) mod zip;

use crate::{t, WorkerOs, VERSION};
use galaxy_client::update::UpdateInfo;
use galaxy_proto::update::{this_arch, version_newer, WORKER_DESKTOP};
use sha2::{Digest, Sha256};
use state::{Kind, Phase, Replaced, State};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub use state::update_dir;

/// 试运行多久内要连上服务器并稳定运行（秒），环境变量 LEMON_GALAXY_UPDATE_TRIAL_SECS 可改（测试用）
const TRIAL_SECS: u64 = 600;
/// 连上服务器后稳定多久算成功（秒），环境变量 LEMON_GALAXY_UPDATE_HEALTHY_SECS 可改
const HEALTHY_SECS: u64 = 60;
/// 新版本启动超过这么多次还没确认就换回（崩溃后被服务管理器一次次拉起）
const MAX_STARTS: u32 = 3;
/// 服务启动后多久第一次检查
const FIRST_CHECK: Duration = Duration::from_secs(600);
/// 定时检查的间隔
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// 有人正在远程时隔多久再看一次
const BUSY_RETRY: Duration = Duration::from_secs(600);
/// 有人正在远程时最多推迟多久（之后照样更新：被控端自动回连，主控端自动重连）
const MAX_DEFER: Duration = Duration::from_secs(24 * 3600);
/// 退出后让服务管理器拉起新版本的退出码
pub const EXIT_RESTART: i32 = 75;

fn env_secs(name: &str, default: u64, min: u64, max: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.trim().parse().ok()).map(|v: u64| v.clamp(min, max)).unwrap_or(default)
}

fn trial_secs() -> u64 {
    env_secs("LEMON_GALAXY_UPDATE_TRIAL_SECS", TRIAL_SECS, 30, 86400)
}

fn healthy_secs() -> u64 {
    env_secs("LEMON_GALAXY_UPDATE_HEALTHY_SECS", HEALTHY_SECS, 5, 3600)
}

fn now_ms() -> i64 {
    galaxy_proto::now_ms()
}

/// 本机装的桌面端（界面 + 被控服务一起的安装包，见 [`WorkerOs::desktop_install`]）。
#[derive(Clone, Debug)]
pub enum DesktopInstall {
    /// Windows：安装目录（Program Files\LemonGalaxy\Worker），里面有界面、被控服务与资源；`gui_version` 是界面程序的文件版本
    WindowsDir { dir: PathBuf, gui_version: Option<String> },
    /// Linux：deb / rpm 安装包；`version` 是包管理器记的版本
    LinuxPackage { format: pkg::Format, version: Option<String> },
}

/// Linux：本机是不是 deb / rpm 装的（以 root 运行、程序就是包里的 /usr/bin/lemon-galaxy-worker、包管理器查得到）。
#[cfg(unix)]
pub fn linux_package() -> Option<DesktopInstall> {
    if !galaxy_host::sysuser::is_root() {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    if exe != Path::new(pkg::BINARIES[0]) {
        return None;
    }
    let (format, version) = pkg::detect()?;
    Some(DesktopInstall::LinuxPackage { format, version })
}

/// 怎么换
#[derive(Clone, Debug)]
pub(crate) enum Plan {
    /// 只换核心程序
    Core { target: PathBuf },
    /// Windows：安装目录里的文件整套换
    WindowsDesktop { dir: PathBuf },
    /// Linux：交给包管理器
    LinuxPackage { format: pkg::Format },
}

/// 找到的新版本
#[derive(Clone, Debug)]
pub(crate) struct Found {
    pub info: UpdateInfo,
    plan: Plan,
    /// 当初的查询条件 (组件, 平台, 当前版本)：下载到的文件与检查时不一致（服务器换了新版本）时据此重查
    query: (&'static str, &'static str, String),
}

/// 桌面端按哪个版本问服务器：被控服务与界面里较旧的那个（界面落后时，同版本的桌面端包也要装，把界面补齐）。
/// 界面的版本读不出来、或比上次确认过的桌面端版本还旧（读到的不对）时，按上次确认的算。
fn desktop_current(core: &str, gui: Option<&str>, confirmed: &str) -> String {
    let gui = match gui.filter(|g| !g.is_empty()) {
        Some(g) if confirmed.is_empty() || !version_newer(confirmed, g) => g,
        _ if !confirmed.is_empty() => confirmed,
        _ => core,
    };
    if version_newer(core, gui) { gui } else { core }.to_string()
}

/// 有没有能装的新版本（跳过以前装上后起不来的版本）。
pub(crate) async fn check(os: &dyn WorkerOs, dir: &Path) -> Result<Option<Found>, String> {
    let api = crate::api_for(dir)?;
    let st = State::load(dir);
    let arch = this_arch();
    let skip = |v: &str| {
        let bad = st.is_bad(v);
        if bad {
            tracing::info!("新版本 {v} 以前装上后没能稳定运行，已自动换回，不再安装；等更新的版本");
        }
        bad
    };
    if let Some(d) = os.desktop_install() {
        let (platform, plan, installed) = match d {
            DesktopInstall::WindowsDir { dir, gui_version } => (crate::platform_name(os), Plan::WindowsDesktop { dir }, gui_version),
            DesktopInstall::LinuxPackage { format, version } => (format.platform(), Plan::LinuxPackage { format }, version),
        };
        let current = desktop_current(VERSION, installed.as_deref(), &st.desktop_version);
        match galaxy_client::update::check(&api, WORKER_DESKTOP, platform, arch, &current).await.map_err(|e| e.to_string())? {
            Some(info) if skip(&info.version) => return Ok(None),
            Some(info) => return Ok(Some(Found { info, plan, query: (WORKER_DESKTOP, platform, current) })),
            // 服务器没有桌面端的包（还没升级的服务器）或已是最新：看核心程序
            None => {}
        }
    }
    let target = match os.update_target() {
        Some(p) => p,
        None => std::env::current_exe().map_err(|e| e.to_string())?,
    };
    let platform = crate::platform_name(os);
    match galaxy_client::update::check(&api, "worker", platform, arch, VERSION).await.map_err(|e| e.to_string())? {
        Some(info) if skip(&info.version) => Ok(None),
        Some(info) => Ok(Some(Found { info, plan: Plan::Core { target }, query: ("worker", platform, VERSION.to_string()) })),
        None => Ok(None),
    }
}

/// 换好了
#[derive(Clone, Debug)]
pub(crate) struct Applied {
    pub version: String,
    /// 调用方要以退出码 75 退出，由服务管理器拉起新版本（Linux 安装包由包脚本重启，不用退出）
    pub exit: bool,
    /// 服务马上会重启
    pub restarting: bool,
}

static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 检查、下载、换上新版本（定时自动更新与「立即更新」共用）。`supervised`：有服务管理器负责拉起新版本。
pub(crate) async fn update_now(os: Arc<dyn WorkerOs>, dir: &Path, supervised: bool) -> Result<Option<Applied>, String> {
    let _g = LOCK.lock().await;
    let st = State::load(dir);
    if st.phase.pending() && state::guard_alive(dir) {
        return Err(t!("上一次更新还在进行中，请稍后再试。", "The previous update is still in progress. Try again later.").into());
    }
    let Some(found) = check(os.as_ref(), dir).await? else { return Ok(None) };
    tracing::info!("开始更新：{VERSION} → {}（{}）", found.info.version, found.info.file_name);
    let (found, file) = download(dir, found).await?;
    let dir2 = dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let staged = stage(&dir2, &found, &file)?;
        apply(&dir2, &found, &staged, supervised)
    })
    .await
    .map_err(|e| e.to_string())?
    .map(Some)
    .inspect_err(|e| tracing::warn!("更新没有完成：{e}"))
}

fn sha256_file(p: &Path) -> Result<String, String> {
    let mut f = std::fs::File::open(p).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h).map_err(|e| e.to_string())?;
    Ok(hex::encode(h.finalize()))
}

/// 下载到 update/staging。返回（实际下载到的版本，文件）。
async fn download(dir: &Path, mut found: Found) -> Result<(Found, PathBuf), String> {
    let api = crate::api_for(dir)?;
    let staging = update_dir(dir).join("staging");
    files::remove_dir_best_effort(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| format!("{}：{e}", staging.display()))?;
    let name: String = found.info.file_name.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')).collect();
    let file = staging.join(if name.is_empty() { "update.bin".to_string() } else { name });
    galaxy_client::update::download(&api, &found.info, &file, &|_, _| {}).await.map_err(|e| e.to_string())?;
    // 检查之后服务器清掉了那个版本、又有了更新的版本时，download 会改下最新的：按文件的 SHA-256 认出实际是哪个版本
    let got = sha256_file(&file)?;
    if !got.eq_ignore_ascii_case(&found.info.sha256) {
        let (component, platform, current) = found.query.clone();
        let fresh = galaxy_client::update::check(&api, component, platform, this_arch(), &current).await.map_err(|e| e.to_string())?;
        match fresh {
            Some(i) if i.sha256.eq_ignore_ascii_case(&got) => {
                tracing::info!("服务器上的新版本换成了 {}", i.version);
                if State::load(dir).is_bad(&i.version) {
                    return Err(format!("新版本 {} 以前装上后没能稳定运行，不再安装", i.version));
                }
                found.info = i;
            }
            _ => return Err(t!("下载的更新包与服务器上的不一致，请稍后再试。", "The downloaded package does not match the server. Try again later.").into()),
        }
    }
    Ok((found, file))
}

/// 跑一下新程序的 `version`：起不来（缺系统函数、架构不对、文件坏了）或报的版本不对就不装。
fn try_run(exe: &Path, version: &str) -> Result<(), String> {
    let mut c = Command::new(exe);
    c.arg("version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).env("LEMON_GALAXY_LANG", "en");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let name = exe.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut child = c.spawn().map_err(|e| format!("{}（{name}）：{e}", t!("新版本程序起不来", "The new program cannot start")))?;
    let t0 = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if t0.elapsed() > Duration::from_secs(60) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(t!("新版本程序没有响应", "The new program does not respond").into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e.to_string()),
        }
    };
    let mut text = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = std::io::Read::read_to_string(&mut o, &mut text);
    }
    if let Some(mut o) = child.stderr.take() {
        let _ = std::io::Read::read_to_string(&mut o, &mut text);
    }
    if !status.success() {
        return Err(format!("{}（{status}）：{}", t!("新版本程序起不来", "The new program cannot start"), text.trim()));
    }
    if !text.split_whitespace().any(|w| w == version) {
        return Err(format!("{}：{}", t!("新版本程序报的版本不对", "The new program reports a different version"), text.trim()));
    }
    Ok(())
}

/// 下载好的文件 → 准备换上的东西：核心程序文件 / 桌面端解开后的目录 / 安装包文件。先试跑新程序。
fn stage(dir: &Path, found: &Found, file: &Path) -> Result<PathBuf, String> {
    let version = &found.info.version;
    let staging = update_dir(dir).join("staging");
    match &found.plan {
        Plan::Core { .. } => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
            }
            try_run(file, version)?;
            Ok(file.to_path_buf())
        }
        Plan::WindowsDesktop { .. } => {
            let data = std::fs::read(file).map_err(|e| e.to_string())?;
            let out = staging.join("desktop");
            let _ = std::fs::remove_dir_all(&out);
            let names = zip::extract_all(&data, &out)?;
            drop(data);
            // 压缩包里是同名文件夹：只有一个顶层目录时以它为根
            let tops: std::collections::BTreeSet<_> = names.iter().filter_map(|p| p.components().next()).map(|c| c.as_os_str().to_owned()).collect();
            let root = match (tops.len(), names.iter().all(|p| p.components().count() > 1)) {
                (1, true) => out.join(tops.into_iter().next().unwrap_or_default()),
                _ => out,
            };
            for need in [DESKTOP_CORE_EXE, DESKTOP_GUI_EXE] {
                if !root.join(need).is_file() {
                    return Err(format!("{}：{need}", t!("桌面端的包里缺少", "The desktop package is missing")));
                }
            }
            try_run(&root.join(DESKTOP_CORE_EXE), version)?;
            Ok(root)
        }
        Plan::LinuxPackage { format } => {
            match pkg::unpack_core(*format, file, &staging.join("unpacked"))? {
                Some(core) => try_run(&core, version)?,
                None => tracing::info!("没有 rpm2cpio / cpio，装之前不试跑新程序"),
            }
            let _ = std::fs::remove_dir_all(staging.join("unpacked"));
            Ok(file.to_path_buf())
        }
    }
}

/// Windows 桌面端的界面程序与被控服务（压缩包的根目录里）
pub const DESKTOP_GUI_EXE: &str = "LemonGalaxyWorker.exe";
pub const DESKTOP_CORE_EXE: &str = "lemon-galaxy-worker.exe";

/// 当前正在运行的程序（Linux 上用 /proc/self/exe：程序文件被换掉后它仍指向正在运行的那份）
fn running_exe() -> Result<PathBuf, String> {
    #[cfg(target_os = "linux")]
    if Path::new("/proc/self/exe").exists() {
        return Ok(PathBuf::from("/proc/self/exe"));
    }
    std::env::current_exe().map_err(|e| e.to_string())
}

/// 把当前（旧）程序复制一份给更新守护进程用（换文件之后它仍是旧版本，回滚时用它）。
fn prepare_guard(dir: &Path) -> Result<PathBuf, String> {
    let root = update_dir(dir).join("guard");
    files::remove_dir_best_effort(&root);
    // 每次一个新目录：Windows 上一次的守护进程可能还没退出，它的程序删不掉也盖不了
    let g = root.join(now_ms().to_string());
    std::fs::create_dir_all(&g).map_err(|e| format!("{}：{e}", g.display()))?;
    let dst = g.join(crate::CORE_EXE);
    std::fs::copy(running_exe()?, &dst).map_err(|e| format!("{}：{e}", dst.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dst, std::fs::Permissions::from_mode(0o700));
    }
    Ok(dst)
}

fn wait_guard(dir: &Path, secs: u64) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(secs) {
        if state::guard_alive(dir) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

/// 在独立的进程里启动更新守护进程，等它的第一次心跳。
fn spawn_guard(dir: &Path, exe: &Path) -> Result<(), String> {
    let dir_s = dir.as_os_str();
    // Linux：放进单独的 systemd 单元，被控服务重启、包脚本重启服务都不会连带结束它
    #[cfg(target_os = "linux")]
    if Path::new("/run/systemd/system").exists() {
        let mut c = Command::new("systemd-run");
        if !galaxy_host::sysuser::is_root() {
            c.arg("--user");
        }
        c.arg(format!("--unit=lemon-galaxy-worker-update-{}", now_ms()))
            .arg("--description=LemonGalaxy worker update guard")
            .arg(exe)
            .arg("--data-dir")
            .arg(dir_s)
            .arg("__lg-update-guard")
            .stdin(Stdio::null());
        match c.output() {
            Ok(o) if o.status.success() => {
                if wait_guard(dir, 15) {
                    tracing::info!("更新守护进程已在单独的 systemd 单元里启动");
                    return Ok(());
                }
                tracing::warn!("systemd-run 启动了更新守护进程，但它没有心跳，改为直接启动");
            }
            Ok(o) => tracing::warn!("systemd-run 失败：{}", String::from_utf8_lossy(&o.stderr).trim()),
            Err(e) => tracing::warn!("systemd-run 失败：{e}"),
        }
    }
    // 直接起子进程。复制出来的程序跑不起来（数据目录所在分区不让执行等）时，用正在运行的这份（Linux 的 /proc/self/exe）
    let mut cands = vec![exe.to_path_buf()];
    if let Ok(r) = running_exe() {
        if r != exe {
            cands.push(r);
        }
    }
    let mut last = String::new();
    for cand in cands {
        let mut c = Command::new(&cand);
        c.arg("--data-dir").arg(dir_s).arg("__lg-update-guard").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        // 自己一个进程组 / 不挂控制台：被控服务退出时不会连带结束它（systemd 单元是 KillMode=process）
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            c.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000 | 0x0000_0200); // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP
        }
        let child = match c.spawn() {
            Ok(c) => c,
            Err(e) => {
                last = format!("{}：{e}", cand.display());
                tracing::warn!("启动更新守护进程失败：{last}");
                continue;
            }
        };
        #[cfg(unix)]
        {
            let mut child = child;
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        #[cfg(not(unix))]
        drop(child);
        if wait_guard(dir, 20) {
            tracing::info!("更新守护进程已启动（{}）", cand.display());
            return Ok(());
        }
        last = format!("{} {}", cand.display(), t!("没有心跳", "has no heartbeat"));
        tracing::warn!("更新守护进程 {last}");
    }
    Err(format!("{}：{last}", t!("更新守护进程没有起来", "The update guard did not start")))
}

/// 按备份换回；返回没换回来的文件。
fn restore(files: &[Replaced]) -> Vec<String> {
    let mut errs = vec![];
    for r in files {
        let res = match &r.backup {
            Some(b) => files::install_file(b, &r.target, 0o755),
            None => files::remove_file(&r.target),
        };
        if let Err(e) = res {
            errs.push(format!("{}：{e}", r.target.display()));
        }
    }
    errs
}

/// 备份、写进度、启动守护进程、换文件。在阻塞线程里跑。
fn apply(dir: &Path, found: &Found, staged: &Path, supervised: bool) -> Result<Applied, String> {
    let upd = update_dir(dir);
    let version = found.info.version.clone();
    // (新文件, 目标)：被控服务最后换
    let mut pairs: Vec<(PathBuf, PathBuf)> = vec![];
    let (kind, backup_targets, package) = match &found.plan {
        Plan::Core { target } => {
            pairs.push((staged.to_path_buf(), target.clone()));
            (Kind::Core, vec![target.clone()], None)
        }
        Plan::WindowsDesktop { dir: inst } => {
            let mut core = None;
            for rel in files::list_files(staged).map_err(|e| e.to_string())? {
                let p = (staged.join(&rel), inst.join(&rel));
                if rel == Path::new(DESKTOP_CORE_EXE) {
                    core = Some(p);
                } else {
                    pairs.push(p);
                }
            }
            pairs.extend(core);
            (Kind::WindowsDesktop, pairs.iter().map(|(_, t)| t.clone()).collect(), None)
        }
        Plan::LinuxPackage { format } => {
            if !supervised || !galaxy_host::sysuser::is_root() {
                return Err(t!("安装包装的被控端要由系统服务（root）更新。", "A packaged install must be updated by the system service (root).").into());
            }
            (Kind::LinuxPackage, pkg::BINARIES.iter().map(PathBuf::from).filter(|p| p.exists()).collect(), Some((*format, staged.to_path_buf())))
        }
    };
    // 备份（按序号放，避免同名）
    let backup_root = upd.join("backup");
    files::remove_dir_best_effort(&backup_root);
    let mut replaced = vec![];
    for (i, t) in backup_targets.iter().enumerate() {
        if t.exists() {
            let b = backup_root.join(i.to_string()).join(t.file_name().unwrap_or_default());
            std::fs::create_dir_all(b.parent().unwrap_or(&backup_root)).map_err(|e| e.to_string())?;
            std::fs::copy(t, &b).map_err(|e| format!("{} {}：{e}", t!("备份失败", "Backup failed"), t.display()))?;
            replaced.push(Replaced { target: t.clone(), backup: Some(b) });
        } else {
            replaced.push(Replaced { target: t.clone(), backup: None });
        }
    }

    if !supervised {
        // 前台运行：没有服务管理器拉起新版本，也没有人看着回滚。只换文件，重启由用户自己来
        for (src, t) in &pairs {
            if let Err(e) = files::install_file(src, t, 0o755) {
                restore(&replaced);
                return Err(format!("{} {}：{e}", t!("替换程序失败", "Cannot replace program"), t.display()));
            }
        }
        files::remove_dir_best_effort(&backup_root);
        return Ok(Applied { version, exit: false, restarting: false });
    }

    let guard = prepare_guard(dir)?;
    let trial = trial_secs();
    let now = now_ms();
    let wrote = State::update(dir, |s| {
        s.phase = if package.is_some() { Phase::Installing } else { Phase::Trial };
        s.kind = kind;
        s.from = VERSION.to_string();
        s.to = version.clone();
        s.applied_at = now;
        s.deadline = if package.is_some() { 0 } else { now + trial as i64 * 1000 };
        s.trial_secs = trial;
        s.starts = 0;
        s.files = replaced.clone();
        s.package = package.as_ref().map(|(_, p)| p.clone());
        s.package_format = package.as_ref().map(|(f, _)| f.name().to_string()).unwrap_or_default();
        s.guard = Some(guard.clone());
        s.reason.clear();
        s.rollback_bad = false;
        s.finished_at = 0;
        true
    });
    if !wrote {
        return Err(t!("写不了更新记录（数据目录）", "Cannot write the update record").into());
    }
    let fail = |reason: String| {
        State::update(dir, |s| {
            s.phase = Phase::Failed;
            s.reason = reason.clone();
            s.finished_at = now_ms();
            true
        });
        reason
    };
    // 先把看着的人安排好再动文件：守护进程起不来就不换
    if let Err(e) = spawn_guard(dir, &guard) {
        return Err(fail(e));
    }
    for (src, t) in &pairs {
        if let Err(e) = files::install_file(src, t, 0o755) {
            let errs = restore(&replaced);
            if !errs.is_empty() {
                tracing::error!("换回旧文件时出错：{}", errs.join("；"));
            }
            return Err(fail(format!("{} {}：{e}", t!("替换程序失败", "Cannot replace program"), t.display())));
        }
    }
    tracing::info!("新版本 {version} 已就位，{}", if package.is_some() { "由更新守护进程装包" } else { "重启服务" });
    Ok(Applied { version, exit: package.is_none(), restarting: true })
}

// ---------------- 新版本这一侧 ----------------

/// 被控服务启动最早的时候（`run` / `service`，还没联网、没开线程）：新版本试运行中就把启动次数加一；
/// 程序被手动换成了别的版本就作废这次更新；守护进程不在（比如中途重启了电脑）就再拉起来。
pub fn on_start(dir: &Path) {
    let st = State::load(dir);
    if !st.phase.pending() {
        return;
    }
    if st.to == VERSION {
        State::update(dir, |s| {
            if s.phase.pending() && s.to == VERSION {
                s.starts += 1;
                true
            } else {
                false
            }
        });
        tracing::info!("新版本 {VERSION} 试运行中（第 {} 次启动）", st.starts + 1);
    } else if st.from != VERSION {
        State::update(dir, |s| {
            if !s.phase.pending() || s.phase == Phase::RollingBack {
                return false;
            }
            s.phase = Phase::Failed;
            s.reason = format!("程序已被换成 {VERSION}，这次更新作废");
            s.finished_at = now_ms();
            true
        });
        tracing::info!("程序已被换成 {VERSION}（更新原本要从 {} 换到 {}），这次更新作废", st.from, st.to);
        return;
    }
    if state::guard_alive(dir) {
        return;
    }
    let exe = st.guard.clone().filter(|p| p.exists()).or_else(|| std::env::current_exe().ok());
    let dir = dir.to_path_buf();
    if let Some(exe) = exe {
        std::thread::spawn(move || {
            tracing::info!("更新守护进程不在，重新启动它");
            if let Err(e) = spawn_guard(&dir, &exe) {
                tracing::warn!("重新启动更新守护进程失败：{e}");
            }
        });
    }
}

/// 新版本试运行中：连上服务器并稳定 `HEALTHY_SECS` 秒后确认。`online` 返回现在是否连着服务器。
pub(crate) fn watch_health(os: Arc<dyn WorkerOs>, dir: &Path, online: impl Fn() -> bool + Send + 'static) {
    let st = State::load(dir);
    if !(st.phase.pending() && st.to == VERSION) {
        return;
    }
    let need = Duration::from_secs(healthy_secs());
    let dir = dir.to_path_buf();
    tokio::spawn(async move {
        let mut since: Option<Instant> = None;
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let st = State::load(&dir);
            if !(matches!(st.phase, Phase::Installing | Phase::Trial) && st.to == VERSION) {
                return;
            }
            // 包脚本先重启了服务，包管理器还在收尾：等它装完
            if st.phase == Phase::Installing {
                continue;
            }
            since = if online() { since.or(Some(Instant::now())) } else { None };
            if since.is_some_and(|t| t.elapsed() >= need) {
                let (os, dir) = (os.clone(), dir.clone());
                let _ = tokio::task::spawn_blocking(move || commit(os.as_ref(), &dir)).await;
                return;
            }
        }
    });
}

/// 确认新版本：删备份、删挪开的旧文件。离截止时间太近就不确认（交给守护进程，免得两边同时动手）。
fn commit(os: &dyn WorkerOs, dir: &Path) {
    let mut done = State::default();
    let ok = State::update(dir, |s| {
        if s.phase != Phase::Trial || s.to != VERSION || (s.deadline > 0 && now_ms() > s.deadline - 10_000) {
            return false;
        }
        s.phase = Phase::Committed;
        s.finished_at = now_ms();
        s.reason.clear();
        if s.kind != Kind::Core {
            s.desktop_version = VERSION.to_string();
        }
        done = s.clone();
        true
    });
    if !ok {
        return;
    }
    tracing::info!("更新到 {VERSION} 已确认：连上服务器并稳定运行（从 {} 换来，启动 {} 次）", done.from, done.starts);
    let upd = update_dir(dir);
    files::remove_dir_best_effort(&upd.join("backup"));
    files::remove_dir_best_effort(&upd.join("staging"));
    for r in &done.files {
        files::clean_old_of(&r.target);
    }
    if done.kind == Kind::WindowsDesktop {
        os.desktop_updated(VERSION);
    }
}

// ---------------- 更新守护进程 ----------------

/// `__lg-update-guard`：由旧版本启动、用旧版本程序的副本运行，看着新版本；必要时装包、换回。
pub fn guard_main(os: Arc<dyn WorkerOs>, dir: &Path) -> i32 {
    if state::guard_alive(dir) {
        tracing::info!("已有更新守护进程在运行");
        return 0;
    }
    state::heartbeat(dir);
    let st0 = State::load(dir);
    tracing::info!("更新守护进程（{VERSION}）启动：{} → {}，{:?}", st0.from, st0.to, st0.phase);
    let t0 = Instant::now();
    let max_life = Duration::from_secs(st0.trial_secs.max(TRIAL_SECS) + 3 * 3600);
    let mut installed_at: Option<Instant> = None;
    let mut restarted = false;
    loop {
        state::heartbeat(dir);
        let st = State::load(dir);
        match st.phase {
            Phase::Installing => match install_package(dir, &st) {
                Ok(()) => {
                    State::update(dir, |s| {
                        if s.phase != Phase::Installing {
                            return false;
                        }
                        s.phase = Phase::Trial;
                        s.deadline = now_ms() + s.trial_secs.max(30) as i64 * 1000;
                        true
                    });
                    installed_at = Some(Instant::now());
                }
                Err(e) => {
                    rollback(os.as_ref(), dir, &format!("安装包没装上：{e}"), false);
                }
            },
            Phase::Trial => {
                if st.starts > MAX_STARTS {
                    rollback(os.as_ref(), dir, &format!("新版本 {} 启动了 {} 次都没能稳定运行", st.to, st.starts), true);
                } else if st.deadline > 0 && now_ms() > st.deadline {
                    let mins = st.trial_secs.div_ceil(60).max(1);
                    rollback(os.as_ref(), dir, &format!("新版本 {} 在 {mins} 分钟内没有连上服务器并稳定运行（启动 {} 次）", st.to, st.starts), true);
                } else if st.kind == Kind::LinuxPackage && st.starts == 0 && !restarted && installed_at.is_some_and(|t| t.elapsed() > Duration::from_secs(45)) {
                    // 包脚本没有把服务重启成新版本（比如服务当时没在运行）：重启一次
                    restarted = true;
                    tracing::info!("包已装好但新版本还没启动，重启服务");
                    os.restart_service();
                }
            }
            // 上一个守护进程换回到一半就没了：接着做完
            Phase::RollingBack => {
                let (reason, bad) = (st.reason.clone(), st.rollback_bad);
                rollback(os.as_ref(), dir, &reason, bad);
            }
            p => {
                tracing::info!("更新已有结论（{p:?}），守护进程退出");
                state::clear_heartbeat(dir);
                return 0;
            }
        }
        if t0.elapsed() > max_life {
            tracing::warn!("更新守护进程运行太久，退出");
            state::clear_heartbeat(dir);
            return 1;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// 守护进程装包：dpkg / rpm 在子进程里跑（一边跑一边心跳），输出写进 update/install.log；dpkg 锁被占时等一会再试。
fn install_package(dir: &Path, st: &State) -> Result<(), String> {
    let fmt = pkg::Format::parse(&st.package_format).ok_or("不认识的安装包格式")?;
    let file = st.package.clone().ok_or("没有安装包")?;
    let log_path = update_dir(dir).join("install.log");
    let mut last = String::new();
    for attempt in 1..=6 {
        tracing::info!("装 {}（第 {attempt} 次）", file.display());
        let log = std::fs::File::create(&log_path).map_err(|e| e.to_string())?;
        let mut c = pkg::install_command(fmt, &file);
        c.stdin(Stdio::null()).stdout(log.try_clone().map_err(|e| e.to_string())?).stderr(log);
        let mut child = c.spawn().map_err(|e| e.to_string())?;
        let t0 = Instant::now();
        let status = loop {
            state::heartbeat(dir);
            match child.try_wait() {
                Ok(Some(s)) => break Some(s),
                Ok(None) if t0.elapsed() > Duration::from_secs(1200) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                Ok(None) => std::thread::sleep(Duration::from_secs(1)),
                Err(e) => return Err(e.to_string()),
            }
        };
        let out = std::fs::read_to_string(&log_path).unwrap_or_default();
        if status.is_some_and(|s| s.success()) {
            tracing::info!("安装包已装好：{}", out.lines().last().unwrap_or("").trim());
            return Ok(());
        }
        last = out.lines().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" / ");
        tracing::warn!("装包失败（{status:?}）：{last}");
        for _ in 0..10 {
            state::heartbeat(dir);
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    Err(last)
}

/// 换回旧版本：停服务、按备份换回、记下原因（`bad` 时记进坏版本）、再启动服务。
fn rollback(os: &dyn WorkerOs, dir: &Path, reason: &str, bad: bool) -> bool {
    let mut st = State::default();
    let claimed = State::update(dir, |s| {
        if !matches!(s.phase, Phase::Trial | Phase::Installing | Phase::RollingBack) {
            return false;
        }
        s.phase = Phase::RollingBack;
        s.reason = reason.to_string();
        s.rollback_bad = bad;
        st = s.clone();
        true
    });
    if !claimed {
        return false;
    }
    tracing::warn!("{reason}：换回 {}", st.from);
    if !os.stop_service() {
        tracing::warn!("停止服务失败，照样换回文件");
    }
    let errs = restore(&st.files);
    if !errs.is_empty() {
        tracing::error!("有文件没换回来：{}", errs.join("；"));
    }
    State::update(dir, |s| {
        s.phase = if bad { Phase::RolledBack } else { Phase::Failed };
        s.finished_at = now_ms();
        if bad {
            s.mark_bad(&st.to);
        }
        true
    });
    if errs.is_empty() {
        let upd = update_dir(dir);
        files::remove_dir_best_effort(&upd.join("backup"));
        files::remove_dir_best_effort(&upd.join("staging"));
    }
    if os.start_service() {
        tracing::warn!("已换回 {} 并重新启动服务{}", st.from, if bad { format!("；{} 不再自动安装", st.to) } else { String::new() });
    } else {
        tracing::error!("已换回 {}，但启动服务失败", st.from);
    }
    true
}

// ---------------- 定时自动更新 ----------------

/// 定时自动更新要不要再等：有人正在远程、且从发现新版本起还没满 24 小时
fn should_defer(busy: bool, waited: Duration) -> bool {
    busy && waited < MAX_DEFER
}

/// 服务模式下的定时自动更新：开着「自动更新」时每 6 小时检查一次；有人正在远程就先不重启，过一会再看，
/// 最多推迟 24 小时，之后照样更新（被控端自动回连，主控端自动重连）。`busy` 返回现在有没有活动的远程会话。
pub(crate) async fn auto_loop(os: Arc<dyn WorkerOs>, dir: PathBuf, busy: Arc<dyn Fn() -> bool + Send + Sync>) {
    tokio::time::sleep(FIRST_CHECK).await;
    let mut pending_since: Option<Instant> = None;
    loop {
        let host = galaxy_host::accounts::HostState::load(&dir.join("host.json"));
        let mut wait = CHECK_EVERY;
        if host.auto_update && !host.accounts.is_empty() {
            match check(os.as_ref(), &dir).await {
                Ok(Some(found)) => {
                    let first = pending_since.is_none();
                    let since = *pending_since.get_or_insert_with(Instant::now);
                    if should_defer(busy(), since.elapsed()) {
                        if first {
                            tracing::info!("有新版本 {}，有人正在远程，等远程结束再更新（最多推迟 24 小时）", found.info.version);
                        }
                        wait = BUSY_RETRY;
                    } else {
                        if busy() {
                            tracing::warn!("有人正在远程，但更新已推迟超过 24 小时，照常更新（被控端会自动回连）");
                        }
                        pending_since = None;
                        match update_now(os.clone(), &dir, true).await {
                            Ok(Some(a)) if a.exit => {
                                tracing::info!("已自动更新到 {}，重启服务", a.version);
                                crate::exit_for_update(&dir);
                            }
                            Ok(Some(a)) => tracing::info!("开始自动安装 {}，装好后服务自动重启", a.version),
                            Ok(None) => {}
                            Err(e) => tracing::warn!("自动更新失败：{e}"),
                        }
                    }
                }
                Ok(None) => pending_since = None,
                Err(e) => tracing::warn!("检查更新失败：{e}"),
            }
        } else {
            pending_since = None;
        }
        tokio::time::sleep(wait).await;
    }
}

// ---------------- 测试 ----------------

/// 端到端测试的故障注入（scripts/e2e-update.sh）：测试脚本把这串字节原样改成「…crash」或「…offline」，做出
/// 「起来就崩」「一直连不上服务器」的坏版本。只在只认测试密钥的测试构建里生效，正式构建不看它。
static TEST_FAULT: [u8; 32] = *b"LG-E2E-FAULT:none_______________";

pub(crate) fn test_fault() -> &'static str {
    if !galaxy_proto::update::TEST_RELEASE_KEY {
        return "";
    }
    // 按运行时的内容读：测试脚本改的是程序文件里的字节
    let b: [u8; 32] = unsafe { std::ptr::read_volatile(std::ptr::addr_of!(TEST_FAULT)) };
    if b.starts_with(b"LG-E2E-FAULT:crash") {
        "crash"
    } else if b.starts_with(b"LG-E2E-FAULT:offline") {
        "offline"
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_version_to_ask() {
        // 界面与被控服务同版本
        assert_eq!(desktop_current("0.1.0.75", Some("0.1.0.75"), ""), "0.1.0.75");
        // 界面落后（以前只换了核心）：按界面的版本问，同版本的桌面端包也装
        assert_eq!(desktop_current("0.1.0.75", Some("0.1.0.70"), ""), "0.1.0.70");
        // 读不出界面版本：按被控服务
        assert_eq!(desktop_current("0.1.0.75", None, ""), "0.1.0.75");
        // 界面版本读成了别的（本机构建等），但上次已确认装过 0.1.0.75：不再反复装
        assert_eq!(desktop_current("0.1.0.75", Some("0.1.0"), "0.1.0.75"), "0.1.0.75");
        assert_eq!(desktop_current("0.1.0.75", None, "0.1.0.75"), "0.1.0.75");
        // 回滚后包管理器记的是坏版本（比程序新）：按程序的版本
        assert_eq!(desktop_current("0.1.0.75", Some("0.1.0.76"), "0.1.0.74"), "0.1.0.75");
    }

    #[test]
    fn defer_only_while_busy_and_at_most_a_day() {
        assert!(!should_defer(false, Duration::ZERO));
        assert!(should_defer(true, Duration::from_secs(3600)));
        assert!(should_defer(true, MAX_DEFER - Duration::from_secs(1)));
        // 推迟满 24 小时：有人在远程也照样更新
        assert!(!should_defer(true, MAX_DEFER));
    }

    #[test]
    fn env_knobs_are_clamped() {
        assert_eq!(env_secs("LG_TEST_UNSET_KNOB", 600, 30, 86400), 600);
        std::env::set_var("LG_TEST_KNOB_LOW", "1");
        assert_eq!(env_secs("LG_TEST_KNOB_LOW", 600, 30, 86400), 30);
        std::env::set_var("LG_TEST_KNOB_BAD", "abc");
        assert_eq!(env_secs("LG_TEST_KNOB_BAD", 600, 30, 86400), 600);
    }

    #[test]
    fn fault_injection_only_in_test_builds() {
        assert_eq!(test_fault(), "");
    }

    /// 记下服务被停、被启动了几次的假系统
    #[derive(Default)]
    struct FakeOs {
        stops: std::sync::atomic::AtomicU32,
        starts: std::sync::atomic::AtomicU32,
        desktop: parking_lot::Mutex<Option<String>>,
    }

    impl WorkerOs for FakeOs {
        fn platform(&self) -> galaxy_proto::Platform {
            galaxy_proto::Platform::Linux
        }
        fn os_version(&self) -> String {
            "test".into()
        }
        fn default_data_dir(&self, _admin: bool) -> PathBuf {
            std::env::temp_dir()
        }
        fn default_system_user(&self) -> String {
            "test".into()
        }
        fn superuser_name(&self) -> &'static str {
            "root"
        }
        fn install_service(&self, _dir: &Path) -> i32 {
            0
        }
        fn uninstall_service(&self) -> i32 {
            0
        }
        fn stop_service(&self) -> bool {
            self.stops.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            true
        }
        fn start_service(&self) -> bool {
            self.starts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            true
        }
        fn desktop_updated(&self, version: &str) {
            *self.desktop.lock() = Some(version.into());
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lg-upd-flow-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// 装好的旧版本：inst/ 下两个文件，备份在 update/backup，新版本已换上（文件内容是 new），新加了一个文件
    fn trial_setup(d: &Path, phase: Phase, deadline: i64, starts: u32) -> (PathBuf, PathBuf, PathBuf) {
        let inst = d.join("inst");
        std::fs::create_dir_all(&inst).unwrap();
        let (a, b, added) = (inst.join(DESKTOP_CORE_EXE), inst.join(DESKTOP_GUI_EXE), inst.join("added.json"));
        let bk = update_dir(d).join("backup");
        std::fs::create_dir_all(bk.join("0")).unwrap();
        std::fs::create_dir_all(bk.join("1")).unwrap();
        std::fs::write(bk.join("0").join("a"), b"old-core").unwrap();
        std::fs::write(bk.join("1").join("b"), b"old-gui").unwrap();
        std::fs::write(&a, b"new-core").unwrap();
        std::fs::write(&b, b"new-gui").unwrap();
        std::fs::write(&added, b"new").unwrap();
        assert!(State::update(d, |s| {
            s.phase = phase;
            s.kind = Kind::WindowsDesktop;
            s.from = "0.0.0.1".into();
            s.to = VERSION.into();
            s.deadline = deadline;
            s.trial_secs = 60;
            s.starts = starts;
            s.files = vec![
                Replaced { target: a.clone(), backup: Some(bk.join("0").join("a")) },
                Replaced { target: b.clone(), backup: Some(bk.join("1").join("b")) },
                Replaced { target: added.clone(), backup: None },
            ];
            true
        }));
        (a, b, added)
    }

    #[test]
    fn rollback_restores_and_marks_bad() {
        let d = tmp("rollback");
        let (a, b, added) = trial_setup(&d, Phase::Trial, now_ms() + 60_000, 1);
        let os = FakeOs::default();
        assert!(rollback(&os, &d, "测试", true));
        assert_eq!(std::fs::read(&a).unwrap(), b"old-core");
        assert_eq!(std::fs::read(&b).unwrap(), b"old-gui");
        assert!(!added.exists());
        let st = State::load(&d);
        assert_eq!(st.phase, Phase::RolledBack);
        assert!(st.is_bad(VERSION));
        assert_eq!(st.reason, "测试");
        assert_eq!((os.stops.load(std::sync::atomic::Ordering::SeqCst), os.starts.load(std::sync::atomic::Ordering::SeqCst)), (1, 1));
        assert!(!update_dir(&d).join("backup").exists());
        // 已有结论：不会再换一次
        assert!(!rollback(&os, &d, "again", true));
        assert_eq!(os.stops.load(std::sync::atomic::Ordering::SeqCst), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn install_failure_is_not_a_bad_version() {
        let d = tmp("failed");
        trial_setup(&d, Phase::Installing, 0, 0);
        assert!(rollback(&FakeOs::default(), &d, "dpkg 失败", false));
        let st = State::load(&d);
        assert_eq!(st.phase, Phase::Failed);
        assert!(!st.is_bad(VERSION));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn commit_only_in_time() {
        let d = tmp("commit");
        let os = FakeOs::default();
        // 离截止时间不到 10 秒：不确认，交给守护进程
        trial_setup(&d, Phase::Trial, now_ms() + 5_000, 1);
        commit(&os, &d);
        assert_eq!(State::load(&d).phase, Phase::Trial);
        // 时间够：确认，删备份，记下桌面端版本
        let (a, _, _) = trial_setup(&d, Phase::Trial, now_ms() + 120_000, 1);
        let old = a.with_file_name("lemon-galaxy-worker.old.exe");
        std::fs::write(&old, b"x").unwrap();
        commit(&os, &d);
        let st = State::load(&d);
        assert_eq!(st.phase, Phase::Committed);
        assert_eq!(st.desktop_version, VERSION);
        assert!(!update_dir(&d).join("backup").exists());
        assert!(!old.exists());
        assert_eq!(os.desktop.lock().as_deref(), Some(VERSION));
        assert_eq!(std::fs::read(&a).unwrap(), b"new-core");
        // 已确认后守护进程不会再换回
        assert!(!rollback(&os, &d, "late", true));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn start_bookkeeping() {
        let d = tmp("start");
        trial_setup(&d, Phase::Trial, now_ms() + 120_000, 0);
        // 装作守护进程在：不再拉起
        std::fs::write(update_dir(&d).join("guard.alive"), format!("{} {}", std::process::id() + 1, now_ms())).unwrap();
        on_start(&d);
        on_start(&d);
        assert_eq!(State::load(&d).starts, 2);
        // 程序被换成了既不是旧版本也不是新版本：作废，守护进程不再换回
        State::update(&d, |s| {
            s.to = "9.9.9.9".into();
            true
        });
        on_start(&d);
        let st = State::load(&d);
        assert_eq!(st.phase, Phase::Failed);
        assert!(!rollback(&FakeOs::default(), &d, "x", true));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn guard_rolls_back_crash_loop_and_deadline() {
        for (deadline, starts) in [(now_ms() + 600_000, MAX_STARTS + 1), (now_ms() - 1, 1)] {
            let d = tmp("guard");
            let (a, _, _) = trial_setup(&d, Phase::Trial, deadline, starts);
            let os = Arc::new(FakeOs::default());
            assert_eq!(guard_main(os.clone(), &d), 0);
            assert_eq!(std::fs::read(&a).unwrap(), b"old-core");
            let st = State::load(&d);
            assert_eq!(st.phase, Phase::RolledBack, "{}", st.reason);
            assert!(st.is_bad(VERSION));
            assert_eq!(os.starts.load(std::sync::atomic::Ordering::SeqCst), 1);
            // 守护进程退出时删掉自己的心跳
            assert!(!state::guard_alive(&d) && !update_dir(&d).join("guard.alive").exists());
            let _ = std::fs::remove_dir_all(&d);
        }
    }

    #[test]
    fn guard_leaves_when_committed() {
        let d = tmp("guard-ok");
        trial_setup(&d, Phase::Committed, now_ms() - 1, 9);
        let os = Arc::new(FakeOs::default());
        assert_eq!(guard_main(os.clone(), &d), 0);
        assert_eq!(os.stops.load(std::sync::atomic::Ordering::SeqCst), 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    fn found(plan: Plan, version: &str) -> Found {
        let info = UpdateInfo { version: version.into(), file_name: "x".into(), size: 0, sha256: String::new(), url: String::new(), notes: String::new() };
        Found { info, plan, query: ("worker", "linux", VERSION.into()) }
    }

    #[test]
    fn apply_without_service_only_replaces_files() {
        let d = tmp("apply");
        // 只换核心程序
        let target = d.join("bin").join("lemon-galaxy-worker");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"old").unwrap();
        let staged = d.join("staged");
        std::fs::write(&staged, b"new").unwrap();
        let a = apply(&d, &found(Plan::Core { target: target.clone() }, "9.9.9.9"), &staged, false).unwrap();
        assert!(!a.exit && !a.restarting);
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(State::load(&d).phase, Phase::Idle);
        // Windows 桌面端：整个目录换，新文件也放进去，旧目录里多的文件不动
        let inst = d.join("inst");
        std::fs::create_dir_all(inst.join("Resources")).unwrap();
        std::fs::write(inst.join(DESKTOP_GUI_EXE), b"old-gui").unwrap();
        std::fs::write(inst.join("keep.txt"), b"keep").unwrap();
        let root = d.join("desk");
        std::fs::create_dir_all(root.join("Resources").join("Locales")).unwrap();
        std::fs::write(root.join(DESKTOP_GUI_EXE), b"new-gui").unwrap();
        std::fs::write(root.join(DESKTOP_CORE_EXE), b"new-core").unwrap();
        std::fs::write(root.join("Resources").join("Locales").join("en.json"), b"{}").unwrap();
        apply(&d, &found(Plan::WindowsDesktop { dir: inst.clone() }, "9.9.9.9"), &root, false).unwrap();
        assert_eq!(std::fs::read(inst.join(DESKTOP_GUI_EXE)).unwrap(), b"new-gui");
        assert_eq!(std::fs::read(inst.join(DESKTOP_CORE_EXE)).unwrap(), b"new-core");
        assert_eq!(std::fs::read(inst.join("Resources").join("Locales").join("en.json")).unwrap(), b"{}");
        assert_eq!(std::fs::read(inst.join("keep.txt")).unwrap(), b"keep");
        // 安装包只能由系统服务装
        assert!(apply(&d, &found(Plan::LinuxPackage { format: pkg::Format::Deb }, "9.9.9.9"), &staged, false).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
