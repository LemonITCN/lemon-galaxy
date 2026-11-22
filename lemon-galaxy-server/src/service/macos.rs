//! macOS：launchd 系统守护进程（LaunchDaemon，root 运行，开机自启、退出自动拉起）。
//!
//! 程序在 /usr/local/lib/lemon-galaxy-server/（/usr/local/bin 下放一个同名链接），
//! 数据在 /Library/Application Support/LemonGalaxy/server（0700），日志写数据目录的 logs/server.log。
//! 系统「应用程序防火墙」打开时把程序加进允许列表（按程序放行，不按端口）。

use super::{SvcConfig, SvcState};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

pub const SERVICE_NAME: &str = "cn.lemonit.galaxy.server";
pub const FAILED_HINT: &str = "启动失败（看数据目录下的 logs/server.log）";
const PLIST: &str = "/Library/LaunchDaemons/cn.lemonit.galaxy.server.plist";
const INSTALL_DIR: &str = "/usr/local/lib/lemon-galaxy-server";
const BIN: &str = "/usr/local/lib/lemon-galaxy-server/lemon-galaxy-server";
const LINK: &str = "/usr/local/bin/lemon-galaxy-server";
const DATA_DIR: &str = "/Library/Application Support/LemonGalaxy/server";
const SOCKETFILTERFW: &str = "/usr/libexec/ApplicationFirewall/socketfilterfw";

extern "C" {
    fn geteuid() -> u32;
}

pub fn is_admin() -> bool {
    unsafe { geteuid() == 0 }
}

pub fn require_admin() -> anyhow::Result<()> {
    anyhow::ensure!(is_admin(), "需要 root 权限，请用 sudo 运行");
    Ok(())
}

pub fn default_data_dir() -> Option<PathBuf> {
    Some(PathBuf::from(DATA_DIR))
}

pub fn config_path() -> Option<PathBuf> {
    Some(Path::new(INSTALL_DIR).join("service.json"))
}

pub fn installed_exe() -> PathBuf {
    PathBuf::from(BIN)
}

pub fn secure_dir(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}

fn target() -> String {
    format!("system/{SERVICE_NAME}")
}

fn launchctl(args: &[&str]) -> anyhow::Result<()> {
    let o = Command::new("launchctl").args(args).stdin(Stdio::null()).output().map_err(|e| anyhow::anyhow!("无法运行 launchctl：{e}"))?;
    if !o.status.success() {
        let err = String::from_utf8_lossy(&o.stderr);
        anyhow::bail!("launchctl {} 失败：{}", args.join(" "), err.trim());
    }
    Ok(())
}

/// launchctl print 的输出（没有加载时为 None）。
fn print_job() -> Option<String> {
    let o = Command::new("launchctl").args(["print", &target()]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

fn loaded() -> bool {
    print_job().is_some()
}

/// 从 launchctl print 的输出里取「state = running」「pid = 123」。
fn parse_print(out: &str) -> (String, Option<u32>) {
    let mut state = String::new();
    let mut pid = None;
    for l in out.lines() {
        let l = l.trim();
        // 只看第一层的字段（子段落里也有同名项，缩进更深，但 trim 后无法区分，取第一次出现的）
        if let Some(v) = l.strip_prefix("state = ") {
            if state.is_empty() {
                state = v.trim().to_string();
            }
        } else if let Some(v) = l.strip_prefix("pid = ") {
            if pid.is_none() {
                pid = v.trim().parse().ok();
            }
        }
    }
    (state, pid)
}

fn same_file(a: &Path, b: &Path) -> bool {
    matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn plist_text(cfg: &SvcConfig) -> String {
    let mut args = vec![BIN.to_string()];
    args.extend(cfg.args());
    args.extend(["--log-file".to_string(), cfg.log_file().to_string_lossy().into_owned(), "run".to_string()]);
    let args: String = args.iter().map(|a| format!("\t\t<string>{}</string>\n", xml_escape(a))).collect();
    // 控制台日志与文件日志内容相同，标准输出丢掉；标准错误（启动失败的原因、崩溃信息）也写进日志文件
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\
         \t<key>Label</key>\n\t<string>{SERVICE_NAME}</string>\n\
         \t<key>ProgramArguments</key>\n\t<array>\n{args}\t</array>\n\
         \t<key>WorkingDirectory</key>\n\t<string>{}</string>\n\
         \t<key>RunAtLoad</key>\n\t<true/>\n\
         \t<key>KeepAlive</key>\n\t<true/>\n\
         \t<key>ThrottleInterval</key>\n\t<integer>5</integer>\n\
         \t<key>ExitTimeOut</key>\n\t<integer>20</integer>\n\
         \t<key>SoftResourceLimits</key>\n\t<dict>\n\t\t<key>NumberOfFiles</key>\n\t\t<integer>65536</integer>\n\t</dict>\n\
         \t<key>StandardOutPath</key>\n\t<string>/dev/null</string>\n\
         \t<key>StandardErrorPath</key>\n\t<string>{}</string>\n\
         </dict>\n</plist>\n",
        xml_escape(&cfg.data_dir.to_string_lossy()),
        xml_escape(&cfg.log_file().to_string_lossy()),
    )
}

/// 卸载（停止）任务，等它真的退出。
fn bootout() {
    if !loaded() {
        return;
    }
    let _ = Command::new("launchctl").args(["bootout", &target()]).stdin(Stdio::null()).stderr(Stdio::null()).status();
    for _ in 0..100 {
        if !loaded() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn bootstrap() -> anyhow::Result<()> {
    // 之前被 launchctl disable 过的话先解除，否则加载了也不会启动
    let _ = Command::new("launchctl").args(["enable", &target()]).stdin(Stdio::null()).stderr(Stdio::null()).status();
    launchctl(&["bootstrap", "system", PLIST])
}

fn firewall_on() -> bool {
    Command::new(SOCKETFILTERFW)
        .arg("--getglobalstate")
        .stdin(Stdio::null())
        .output()
        .map(|o| {
            let s = String::from_utf8_lossy(&o.stdout).to_ascii_lowercase();
            s.contains("enabled") || s.contains("state = 1") || s.contains("state = 2")
        })
        .unwrap_or(false)
}

/// 应用程序防火墙打开时把程序加进允许列表。返回 Ok(true) 表示加了，Ok(false) 表示防火墙没开。
fn firewall_add() -> Result<bool, String> {
    if !firewall_on() {
        return Ok(false);
    }
    for args in [["--add", BIN], ["--unblockapp", BIN]] {
        let o = Command::new(SOCKETFILTERFW).args(args).stdin(Stdio::null()).output().map_err(|e| format!("无法运行 socketfilterfw：{e}"))?;
        if !o.status.success() {
            return Err(format!("socketfilterfw {} 退出码 {}", args[0], o.status.code().unwrap_or(-1)));
        }
    }
    Ok(true)
}

fn firewall_remove() {
    let _ = Command::new(SOCKETFILTERFW).args(["--remove", BIN]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

pub fn firewall_added_msg(port: u16) -> String {
    format!("已把程序加进 macOS 应用程序防火墙的允许列表（TCP 与 UDP {port} 端口可以连入）。")
}

/// 复制程序、写 plist、加载并启动。返回防火墙处理结果。
pub fn install(cfg: &SvcConfig) -> anyhow::Result<Result<bool, String>> {
    let src = std::env::current_exe()?;
    std::fs::create_dir_all(INSTALL_DIR).map_err(|e| anyhow::anyhow!("无法创建 {INSTALL_DIR}：{e}"))?;
    if !same_file(&src, Path::new(BIN)) {
        // 先写临时文件再改名：正在运行的旧程序不受影响
        let tmp = format!("{BIN}.new");
        std::fs::copy(&src, &tmp).map_err(|e| anyhow::anyhow!("无法复制到 {tmp}：{e}"))?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        // 浏览器下载的程序带隔离标记，复制时会一起带过来；去掉它，免得 launchd 启动时被门禁拦下
        let _ = Command::new("xattr").args(["-d", "com.apple.quarantine", &tmp]).stderr(Stdio::null()).status();
        std::fs::rename(&tmp, BIN)?;
    }
    // /usr/local/bin 里放个链接，sudo lemon-galaxy-server status 之类的命令可以直接用
    let link = Path::new(LINK);
    let ours = std::fs::read_link(link).map(|t| t == Path::new(BIN)).unwrap_or(false);
    if !ours && std::fs::symlink_metadata(link).is_err() {
        let _ = std::fs::create_dir_all("/usr/local/bin");
        let _ = std::os::unix::fs::symlink(BIN, link);
    }
    // launchd 打开日志文件时不会建目录
    if let Some(d) = cfg.log_file().parent() {
        std::fs::create_dir_all(d)?;
    }
    bootout();
    std::fs::write(PLIST, plist_text(cfg)).map_err(|e| anyhow::anyhow!("无法写入 {PLIST}：{e}"))?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(PLIST, std::fs::Permissions::from_mode(0o644));
    let fw = firewall_add();
    bootstrap()?;
    Ok(fw)
}

/// 返回是否真的卸载了服务。
pub fn uninstall() -> anyhow::Result<bool> {
    let had = Path::new(PLIST).exists() || loaded();
    if had {
        bootout();
        let _ = std::fs::remove_file(PLIST);
        firewall_remove();
    }
    if std::fs::read_link(LINK).map(|t| t == Path::new(BIN)).unwrap_or(false) {
        let _ = std::fs::remove_file(LINK);
    }
    let _ = std::fs::remove_file(BIN);
    let _ = std::fs::remove_file(format!("{BIN}.new"));
    Ok(had)
}

pub fn state() -> Option<SvcState> {
    if !Path::new(PLIST).exists() {
        return None;
    }
    let Some(out) = print_job() else { return Some(SvcState::Stopped) };
    let (state, pid) = parse_print(&out);
    Some(match state.as_str() {
        "running" => SvcState::Running(pid),
        "spawn scheduled" | "spawning" | "xpcproxy" => SvcState::Starting,
        "exiting" => SvcState::Stopping,
        _ => {
            // 加载着但没在运行：KeepAlive 会再拉起，等着重启的通常是启动失败
            if out.contains("last exit code = 0") || !out.contains("last exit code") {
                SvcState::Stopped
            } else {
                SvcState::Failed
            }
        }
    })
}

pub fn is_stopped() -> bool {
    !matches!(state(), Some(SvcState::Running(_)) | Some(SvcState::Starting))
}

/// 装好的 LaunchDaemon 都是 RunAtLoad；被 launchctl disable 过的不会开机启动。
pub fn autostart() -> Option<bool> {
    if !Path::new(PLIST).exists() {
        return None;
    }
    let o = Command::new("launchctl").args(["print-disabled", "system"]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    let s = String::from_utf8_lossy(&o.stdout);
    let disabled = s.lines().any(|l| l.contains(&format!("\"{SERVICE_NAME}\"")) && (l.contains("=> disabled") || l.contains("=> true")));
    Some(!disabled)
}

pub fn start() -> anyhow::Result<()> {
    if loaded() {
        launchctl(&["kickstart", &target()])
    } else {
        bootstrap()
    }
}

/// 停止：卸载任务（KeepAlive 下直接结束进程会被马上拉起）。plist 还在，开机时照常启动。
pub fn stop() -> anyhow::Result<()> {
    bootout();
    anyhow::ensure!(!loaded(), "停止服务失败（launchctl bootout {}）", target());
    Ok(())
}

pub fn restart() -> anyhow::Result<()> {
    // 程序被自动升级替换过：签名变了，防火墙允许列表要重新加
    let _ = firewall_add();
    if loaded() {
        launchctl(&["kickstart", "-k", &target()])
    } else {
        bootstrap()
    }
}

pub fn update_target() -> Option<PathBuf> {
    (Path::new(PLIST).exists() && Path::new(BIN).exists()).then(|| PathBuf::from(BIN))
}

pub fn restart_if_installed() -> anyhow::Result<bool> {
    if !Path::new(PLIST).exists() {
        return Ok(false);
    }
    require_admin()?;
    restart()?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist() {
        let cfg = SvcConfig { data_dir: DATA_DIR.into(), port: 7443, public_host: Some("a&b.example.com".into()), bind: None, tls_cert: None, tls_key: None };
        let p = plist_text(&cfg);
        assert!(p.contains("<string>cn.lemonit.galaxy.server</string>"));
        assert!(p.contains("\t\t<string>/usr/local/lib/lemon-galaxy-server/lemon-galaxy-server</string>\n\t\t<string>--data-dir</string>\n\t\t<string>/Library/Application Support/LemonGalaxy/server</string>\n"));
        assert!(p.contains("<string>a&amp;b.example.com</string>"));
        assert!(p.contains("\t\t<string>--log-file</string>\n\t\t<string>/Library/Application Support/LemonGalaxy/server/logs/server.log</string>\n\t\t<string>run</string>\n\t</array>"));
        assert!(p.contains("<key>RunAtLoad</key>\n\t<true/>"));
        assert!(p.contains("<key>KeepAlive</key>\n\t<true/>"));
        // 系统自带的 plutil 检查格式
        let f = std::env::temp_dir().join(format!("lg-plist-{}.plist", std::process::id()));
        std::fs::write(&f, &p).unwrap();
        let ok = Command::new("plutil").arg("-lint").arg(&f).output().map(|o| o.status.success()).unwrap_or(true);
        let _ = std::fs::remove_file(&f);
        assert!(ok, "plutil -lint 不通过");
    }

    #[test]
    fn print_parsing() {
        let out = "system/cn.lemonit.galaxy.server = {\n\tactive count = 1\n\tpath = /Library/LaunchDaemons/cn.lemonit.galaxy.server.plist\n\tstate = running\n\n\tprogram = /usr/local/lib/x\n\tpid = 4242\n\tlast exit code = (never exited)\n}\n";
        assert_eq!(parse_print(out), ("running".to_string(), Some(4242)));
        assert_eq!(parse_print("state = not running\n"), ("not running".to_string(), None));
    }
}
