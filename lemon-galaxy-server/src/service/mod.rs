//! 系统服务：install-service / uninstall-service / start-service / stop-service / restart-service / status，
//! Windows（服务管理器）、Linux（systemd）、macOS（launchd 守护进程）同一套用法。
//!
//! - 程序复制到固定位置（Windows `%ProgramFiles%\LemonGalaxy\Server`，Linux `/usr/local/bin`，
//!   macOS `/usr/local/lib/lemon-galaxy-server`），服务不依赖下载目录；
//! - 数据目录默认 `%ProgramData%\LemonGalaxy\server` / `/var/lib/lemon-galaxy-server` /
//!   `/Library/Application Support/LemonGalaxy/server`，只有系统账号与管理员能访问；
//! - 安装时给的端口、对外地址、监听地址、证书写进服务的启动参数，同时记在 service.json，
//!   再次安装时没写的参数沿用上次（写空值表示清除）；卸载时删掉服务、防火墙规则、程序与 service.json，数据目录保留。
//! - 服务启动后把客户端地址与初始化网址写进数据目录的 run-info.json，安装命令读它打印出来。

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(windows)]
pub mod windows;
#[cfg(windows)]
use self::windows as platform;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod other;
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
use other as platform;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 命令行上明确给出的参数（没给的是 None）。
#[derive(Default, Clone, Debug)]
pub struct Overrides {
    pub data_dir: Option<PathBuf>,
    pub port: Option<u16>,
    pub bind: Option<String>,
    pub public_host: Option<String>,
    pub tls_cert: Option<PathBuf>,
    pub tls_key: Option<PathBuf>,
}

/// 服务的安装参数（service.json）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SvcConfig {
    pub data_dir: PathBuf,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls_cert: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls_key: Option<PathBuf>,
}

fn non_empty_str(v: &Option<String>) -> Option<Option<String>> {
    v.as_ref().map(|s| if s.trim().is_empty() { None } else { Some(s.trim().to_string()) })
}

fn non_empty_path(v: &Option<PathBuf>) -> Option<Option<PathBuf>> {
    v.as_ref().map(|p| if p.as_os_str().is_empty() { None } else { Some(p.clone()) })
}

/// 相对路径按当前目录补全（服务的工作目录不是这里）。
fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

impl SvcConfig {
    /// 这次给的参数覆盖上次的，没给的沿用上次，空值表示清除。
    pub fn merge(saved: Option<SvcConfig>, o: &Overrides, default_dir: PathBuf) -> anyhow::Result<SvcConfig> {
        let saved = saved.unwrap_or(SvcConfig { data_dir: default_dir, port: galaxy_proto::DEFAULT_PORT, public_host: None, bind: None, tls_cert: None, tls_key: None });
        let mut c = SvcConfig {
            data_dir: o.data_dir.as_deref().map(absolute).unwrap_or(saved.data_dir),
            port: o.port.unwrap_or(saved.port),
            public_host: non_empty_str(&o.public_host).unwrap_or(saved.public_host),
            bind: non_empty_str(&o.bind).unwrap_or(saved.bind),
            tls_cert: non_empty_path(&o.tls_cert).unwrap_or(saved.tls_cert),
            tls_key: non_empty_path(&o.tls_key).unwrap_or(saved.tls_key),
        };
        c.tls_cert = c.tls_cert.as_deref().map(absolute);
        c.tls_key = c.tls_key.as_deref().map(absolute);
        anyhow::ensure!(c.port != 0, "端口不能是 0");
        anyhow::ensure!(c.tls_cert.is_some() == c.tls_key.is_some(), "--tls-cert 与 --tls-key 要一起给");
        for p in [&c.tls_cert, &c.tls_key].into_iter().flatten() {
            anyhow::ensure!(p.is_file(), "找不到证书文件 {}", p.display());
        }
        if let Some(b) = &c.bind {
            anyhow::ensure!(b.parse::<std::net::IpAddr>().is_ok(), "监听地址不对：{b}");
        }
        Ok(c)
    }

    /// 服务的启动参数（不含子命令）。
    #[cfg_attr(not(any(windows, target_os = "linux", target_os = "macos")), allow(dead_code))]
    pub fn args(&self) -> Vec<String> {
        let mut a = vec!["--data-dir".to_string(), self.data_dir.to_string_lossy().into_owned(), "--port".to_string(), self.port.to_string()];
        if let Some(h) = &self.public_host {
            a.extend(["--public-host".to_string(), h.clone()]);
        }
        if let Some(b) = &self.bind {
            a.extend(["--bind".to_string(), b.clone()]);
        }
        if let (Some(c), Some(k)) = (&self.tls_cert, &self.tls_key) {
            a.extend(["--tls-cert".to_string(), c.to_string_lossy().into_owned(), "--tls-key".to_string(), k.to_string_lossy().into_owned()]);
        }
        a
    }

    pub fn log_file(&self) -> PathBuf {
        log_file_in(&self.data_dir)
    }
}

pub fn log_file_in(data_dir: &Path) -> PathBuf {
    data_dir.join("logs").join("server.log")
}

/// 上次安装服务时的参数。
pub fn load_config() -> Option<SvcConfig> {
    let p = platform::config_path()?;
    serde_json::from_slice(&std::fs::read(p).ok()?).ok()
}

fn save_config(c: &SvcConfig) -> anyhow::Result<()> {
    let Some(p) = platform::config_path() else { return Ok(()) };
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(&p, serde_json::to_vec_pretty(c)?)?;
    Ok(())
}

/// 系统服务默认的数据目录（服务入口用）。
#[cfg(windows)]
pub fn default_data_dir() -> Option<PathBuf> {
    platform::default_data_dir()
}

// ---------------- 运行信息 ----------------

/// 服务器启动后写的运行信息（安装命令、status 命令读它）。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RunInfo {
    pub pid: u32,
    pub version: String,
    pub started_at: i64,
    pub address: String,
    #[serde(default)]
    pub setup_url: Option<String>,
}

fn run_info_path(data_dir: &Path) -> PathBuf {
    data_dir.join("run-info.json")
}

pub fn write_run_info(data_dir: &Path, info: &RunInfo) -> anyhow::Result<()> {
    let p = run_info_path(data_dir);
    let tmp = p.with_extension("json.tmp");
    // 初始化网址带一次性口令，只给本人看
    {
        let mut o = std::fs::OpenOptions::new();
        o.create(true).write(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let mut f = o.open(&tmp)?;
        std::io::Write::write_all(&mut f, &serde_json::to_vec_pretty(info)?)?;
    }
    std::fs::rename(&tmp, &p)?;
    Ok(())
}

pub fn read_run_info(data_dir: &Path) -> Option<RunInfo> {
    serde_json::from_slice(&std::fs::read(run_info_path(data_dir)).ok()?).ok()
}

/// 等服务器写出这次启动的运行信息。服务已经停下（启动失败）时提前返回。
fn wait_started(data_dir: &Path, since_ms: i64, timeout: Duration) -> anyhow::Result<RunInfo> {
    let start = std::time::Instant::now();
    let mut stopped_checks = 0;
    loop {
        if let Some(i) = read_run_info(data_dir).filter(|i| i.started_at >= since_ms) {
            return Ok(i);
        }
        // 刚发出启动命令时服务可能还没进入运行状态，连续几次都是已停止才算失败
        if platform::is_stopped() {
            stopped_checks += 1;
        } else {
            stopped_checks = 0;
        }
        if stopped_checks >= 5 || start.elapsed() > timeout {
            let log = log_file_in(data_dir);
            let tail = crate::logfile::tail(&log, 20);
            anyhow::bail!(
                "服务没有正常启动{}。日志 {}：\n{}",
                if stopped_checks >= 5 { "（已停止）" } else { "（等待超时）" },
                log.display(),
                if tail.is_empty() { "（日志为空）".to_string() } else { tail }
            );
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

// ---------------- 命令 ----------------

pub fn install(o: &Overrides) -> anyhow::Result<()> {
    platform::require_admin()?;
    let default_dir = platform::default_data_dir().ok_or_else(|| anyhow::anyhow!("这个系统不支持安装为系统服务"))?;
    let saved = load_config();
    // 之前在这里前台运行过：提醒可以沿用那份数据
    let local = Path::new(crate::LOCAL_DATA_DIR);
    let local_hint = (saved.is_none() && o.data_dir.is_none() && local.join("server.db").exists()).then(|| absolute(local));
    let cfg = SvcConfig::merge(saved, o, default_dir.clone())?;
    let fresh = !cfg.data_dir.exists();
    std::fs::create_dir_all(&cfg.data_dir)?;
    // 只收紧默认目录或这次新建的目录，不动用户指定的已有目录
    if fresh || cfg.data_dir == default_dir {
        platform::secure_dir(&cfg.data_dir);
    }
    let since = galaxy_proto::now_ms();
    let fw = platform::install(&cfg)?;
    save_config(&cfg)?;
    let info = wait_started(&cfg.data_dir, since, Duration::from_secs(40)).map_err(|e| anyhow::anyhow!("{e}\n改好后重新运行 install-service，或用 uninstall-service 卸载。"))?;
    println!("已安装并启动系统服务 {}（开机自启，出错自动重启）。", platform::SERVICE_NAME);
    println!("程序：{}", platform::installed_exe().display());
    println!("数据目录：{}", cfg.data_dir.display());
    println!("日志：{}", cfg.log_file().display());
    match fw {
        Ok(true) => println!("{}", platform::firewall_added_msg(cfg.port)),
        Ok(false) => println!("如果有防火墙或云服务器安全组，请放行 TCP 与 UDP {} 端口。", cfg.port),
        Err(e) => println!("添加防火墙规则失败（{e}），请手动放行 TCP 与 UDP {} 端口。", cfg.port),
    }
    print_addresses(&info);
    if let Some(l) = local_hint {
        println!();
        println!("提示：当前目录下有之前前台运行留下的数据 {}，服务用的是新的数据目录。", l.display());
        println!("      要沿用之前的账号与设备，加 --data-dir \"{}\" 再运行一次 install-service。", l.display());
    }
    if cfg.public_host.is_none() && info.address.starts_with("127.0.0.1:") {
        println!();
        println!("提示：没有指定 --public-host，地址里是 127.0.0.1，其他电脑连不上。");
        println!("      用 install-service --public-host <域名或 IP> 再运行一次即可修改（其他参数沿用）。");
    }
    Ok(())
}

pub fn print_addresses(info: &RunInfo) {
    println!("客户端填写的服务器地址：{}", info.address);
    if let Some(u) = &info.setup_url {
        println!("尚未创建管理员。请打开 {u} 完成初始化（浏览器会提示自签证书，确认地址里的指纹后继续）");
    }
}

pub fn uninstall() -> anyhow::Result<()> {
    platform::require_admin()?;
    let cfg = load_config();
    let removed = platform::uninstall()?;
    if let Some(p) = platform::config_path() {
        let _ = std::fs::remove_file(&p);
        if let Some(d) = p.parent() {
            let _ = std::fs::remove_dir(d);
        }
    }
    if removed {
        println!("已卸载服务 {}。", platform::SERVICE_NAME);
    } else {
        println!("没有安装服务。");
    }
    let dir = cfg.map(|c| c.data_dir).or_else(platform::default_data_dir);
    if let Some(d) = dir.filter(|d| d.exists()) {
        println!("数据目录保留在 {}（数据库与密钥），不再需要可手动删除。", d.display());
    }
    Ok(())
}

/// 服务的运行状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SvcState {
    Running(Option<u32>),
    Starting,
    Stopping,
    Stopped,
    Failed,
    #[allow(dead_code)]
    Paused,
}

/// 服务名（Windows 服务名 / systemd 单元名 / launchd 标签）。
pub fn service_name() -> &'static str {
    platform::SERVICE_NAME
}

/// 服务状态（没安装为 None）。
pub fn query() -> Option<SvcState> {
    platform::state()
}

/// 服务状态的文字说明（没安装为 None）。
pub fn state() -> Option<String> {
    Some(match query()? {
        SvcState::Running(Some(pid)) => format!("运行中（进程 {pid}）"),
        SvcState::Running(None) => "运行中".into(),
        SvcState::Starting => "正在启动".into(),
        SvcState::Stopping => "正在停止".into(),
        SvcState::Stopped => "已停止".into(),
        SvcState::Failed => platform::FAILED_HINT.into(),
        SvcState::Paused => "已暂停".into(),
    })
}

/// 是否开机自启（没安装为 None）。
pub fn autostart() -> Option<bool> {
    platform::autostart()
}

/// 当前进程有没有管理员 / root 权限（装服务、启停服务要用）。
pub fn is_admin() -> bool {
    platform::is_admin()
}

/// 装了服务时，服务用的数据目录。
pub fn service_data_dir() -> Option<PathBuf> {
    load_config().map(|c| c.data_dir).or_else(platform::default_data_dir)
}

#[derive(Clone, Copy, Debug)]
pub enum Ctl {
    Start,
    Stop,
    Restart,
}

/// start-service / stop-service / restart-service。启动、重启后等服务写出运行信息再打印地址。
pub fn control(c: Ctl) -> anyhow::Result<()> {
    platform::require_admin()?;
    anyhow::ensure!(query().is_some(), "没有安装系统服务（先运行 install-service）");
    let name = platform::SERVICE_NAME;
    let dir = service_data_dir().ok_or_else(|| anyhow::anyhow!("这个系统不支持系统服务"))?;
    match c {
        Ctl::Stop => {
            platform::stop()?;
            for _ in 0..100 {
                if platform::is_stopped() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            println!("已停止服务 {name}。开机时仍会自动启动；不想让它启动请用 uninstall-service 卸载。");
        }
        Ctl::Start if !platform::is_stopped() => println!("服务 {name} 已在运行。"),
        Ctl::Start | Ctl::Restart => {
            let since = galaxy_proto::now_ms();
            if matches!(c, Ctl::Start) {
                platform::start()?;
            } else {
                platform::restart()?;
            }
            let info = wait_started(&dir, since, Duration::from_secs(40))?;
            println!("{}服务 {name}。", if matches!(c, Ctl::Start) { "已启动" } else { "已重启" });
            print_addresses(&info);
        }
    }
    Ok(())
}

/// status 命令：服务状态、地址、初始化网址。`admins` 查当前管理员数（数据库打不开时为 None）。
pub fn print_status(data_dir: &Path, fallback_address: &str, admins: Option<i64>) {
    match state() {
        Some(s) => {
            println!("服务：{} {}", platform::SERVICE_NAME, s);
            if let Some(a) = autostart() {
                println!("开机自启：{}", if a { "是" } else { "否" });
            }
            println!("程序：{}", platform::installed_exe().display());
        }
        None => println!("服务：没有安装（install-service 安装为系统服务）"),
    }
    println!("数据目录：{}", data_dir.display());
    let log = log_file_in(data_dir);
    if log.exists() {
        println!("日志：{}", log.display());
    }
    match read_run_info(data_dir) {
        Some(mut i) => {
            if admins.unwrap_or(0) > 0 {
                i.setup_url = None;
            }
            print_addresses(&i);
        }
        None => println!("客户端填写的服务器地址：{fallback_address}"),
    }
}

/// 自动升级要替换的程序：装了服务就换服务用的那一份。
pub fn update_target() -> Option<PathBuf> {
    platform::update_target()
}

/// 装了服务就重启它。返回是否重启了。
pub fn restart_if_installed() -> anyhow::Result<bool> {
    platform::restart_if_installed()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ov() -> Overrides {
        Overrides::default()
    }

    #[test]
    fn merge_keeps_previous_and_clears_empty() {
        let d = PathBuf::from("/var/lib/x");
        let c = SvcConfig::merge(None, &ov(), d.clone()).unwrap();
        assert_eq!(c, SvcConfig { data_dir: d.clone(), port: galaxy_proto::DEFAULT_PORT, public_host: None, bind: None, tls_cert: None, tls_key: None });

        let first = SvcConfig::merge(None, &Overrides { port: Some(9000), public_host: Some("1.2.3.4".into()), bind: Some("0.0.0.0".into()), ..ov() }, d.clone()).unwrap();
        assert_eq!(first.args(), vec!["--data-dir", "/var/lib/x", "--port", "9000", "--public-host", "1.2.3.4", "--bind", "0.0.0.0"]);

        // 只改端口，其他沿用
        let second = SvcConfig::merge(Some(first.clone()), &Overrides { port: Some(9001), ..ov() }, d.clone()).unwrap();
        assert_eq!(second.port, 9001);
        assert_eq!(second.public_host.as_deref(), Some("1.2.3.4"));
        assert_eq!(second.bind.as_deref(), Some("0.0.0.0"));

        // 空值清除
        let third = SvcConfig::merge(Some(second), &Overrides { public_host: Some(String::new()), bind: Some(" ".into()), ..ov() }, d.clone()).unwrap();
        assert_eq!(third.public_host, None);
        assert_eq!(third.bind, None);
        assert_eq!(third.port, 9001);

        // 上次的数据目录不会被默认值盖掉
        let custom = SvcConfig { data_dir: PathBuf::from("/srv/galaxy"), ..third };
        assert_eq!(SvcConfig::merge(Some(custom), &ov(), d).unwrap().data_dir, PathBuf::from("/srv/galaxy"));
    }

    #[test]
    fn merge_validates() {
        let d = PathBuf::from("/var/lib/x");
        assert!(SvcConfig::merge(None, &Overrides { bind: Some("not-an-ip".into()), ..ov() }, d.clone()).is_err());
        assert!(SvcConfig::merge(None, &Overrides { port: Some(0), ..ov() }, d.clone()).is_err());
        assert!(SvcConfig::merge(None, &Overrides { tls_cert: Some("/nonexistent/cert.pem".into()), ..ov() }, d.clone()).is_err());
        // 相对路径的数据目录补成绝对路径
        let c = SvcConfig::merge(None, &Overrides { data_dir: Some("rel-data".into()), ..ov() }, d).unwrap();
        assert!(c.data_dir.is_absolute());
    }

    #[test]
    fn run_info_roundtrip() {
        let dir = std::env::temp_dir().join(format!("lg-runinfo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let i = RunInfo { pid: 1, version: "0.1.0".into(), started_at: 5, address: "h:1#ab".into(), setup_url: Some("https://h:1/setup?token=x".into()) };
        write_run_info(&dir, &i).unwrap();
        let r = read_run_info(&dir).unwrap();
        assert_eq!(r.address, "h:1#ab");
        assert_eq!(r.setup_url.as_deref(), Some("https://h:1/setup?token=x"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir.join("run-info.json")).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
