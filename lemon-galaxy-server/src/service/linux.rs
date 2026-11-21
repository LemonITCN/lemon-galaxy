//! Linux：systemd 系统服务（root 运行，程序在 /usr/local/bin，数据在 /var/lib/lemon-galaxy-server）。

use super::SvcConfig;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const SERVICE_NAME: &str = "lemon-galaxy-server";
const UNIT_PATH: &str = "/etc/systemd/system/lemon-galaxy-server.service";
const BIN: &str = "/usr/local/bin/lemon-galaxy-server";
const DATA_DIR: &str = "/var/lib/lemon-galaxy-server";
const CONFIG: &str = "/etc/lemon-galaxy-server/service.json";

extern "C" {
    fn geteuid() -> u32;
}

fn is_root() -> bool {
    unsafe { geteuid() == 0 }
}

pub fn require_admin() -> anyhow::Result<()> {
    anyhow::ensure!(is_root(), "需要 root 权限，请用 sudo 运行");
    Ok(())
}

pub fn default_data_dir() -> Option<PathBuf> {
    Some(PathBuf::from(DATA_DIR))
}

pub fn config_path() -> Option<PathBuf> {
    Some(PathBuf::from(CONFIG))
}

pub fn installed_exe() -> PathBuf {
    PathBuf::from(BIN)
}

pub fn secure_dir(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}

fn systemctl(args: &[&str]) -> anyhow::Result<()> {
    let st = Command::new("systemctl").args(args).status().map_err(|e| anyhow::anyhow!("无法运行 systemctl：{e}"))?;
    anyhow::ensure!(st.success(), "systemctl {} 失败", args.join(" "));
    Ok(())
}

fn same_file(a: &Path, b: &Path) -> bool {
    matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

/// systemd 的 ExecStart 参数写法：普通字符原样，其余加双引号转义；% 与 $ 要写两遍。
fn quote(s: &str) -> String {
    let plain = !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "/._:@,+=-".contains(c));
    let s = if plain {
        s.to_string()
    } else {
        let mut q = String::from("\"");
        for c in s.chars() {
            match c {
                '"' | '\\' => {
                    q.push('\\');
                    q.push(c);
                }
                _ => q.push(c),
            }
        }
        q.push('"');
        q
    };
    s.replace('%', "%%").replace('$', "$$")
}

fn unit_text(cfg: &SvcConfig) -> String {
    let mut args = vec![BIN.to_string()];
    args.extend(cfg.args());
    args.extend(["--log-file".to_string(), cfg.log_file().to_string_lossy().into_owned(), "run".to_string()]);
    let exec: Vec<String> = args.iter().map(|a| quote(a)).collect();
    format!(
        "[Unit]\nDescription=LemonGalaxy Server\nAfter=network-online.target\nWants=network-online.target\n\n\
         [Service]\nType=simple\nExecStart={}\nRestart=always\nRestartSec=3\nTimeoutStopSec=20\nLimitNOFILE=65536\n\
         NoNewPrivileges=yes\nPrivateTmp=yes\nProtectSystem=full\n\n\
         [Install]\nWantedBy=multi-user.target\n",
        exec.join(" ")
    )
}

/// 复制程序、写 unit、启动。返回防火墙处理结果（Linux 不改防火墙）。
pub fn install(cfg: &SvcConfig) -> anyhow::Result<Result<bool, String>> {
    anyhow::ensure!(Path::new("/run/systemd/system").exists(), "没有检测到 systemd。请用你的系统的方式让这条命令开机自动运行：\n{} {} run", std::env::current_exe()?.display(), cfg.args().join(" "));
    let src = std::env::current_exe()?;
    if !same_file(&src, Path::new(BIN)) {
        // 先写临时文件再改名：正在运行的旧程序不受影响
        let tmp = format!("{BIN}.new");
        std::fs::copy(&src, &tmp).map_err(|e| anyhow::anyhow!("无法复制到 {tmp}：{e}"))?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        std::fs::rename(&tmp, BIN)?;
    }
    std::fs::write(UNIT_PATH, unit_text(cfg)).map_err(|e| anyhow::anyhow!("无法写入 {UNIT_PATH}：{e}"))?;
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "--quiet", SERVICE_NAME])?;
    // 已在运行时用新参数重启
    systemctl(&["restart", SERVICE_NAME])?;
    Ok(Ok(false))
}

/// 返回是否真的卸载了服务。
pub fn uninstall() -> anyhow::Result<bool> {
    let had = Path::new(UNIT_PATH).exists();
    if had {
        let _ = Command::new("systemctl").args(["disable", "--now", "--quiet", SERVICE_NAME]).status();
        let _ = std::fs::remove_file(UNIT_PATH);
        let _ = Command::new("systemctl").args(["daemon-reload"]).status();
        let _ = Command::new("systemctl").args(["reset-failed", SERVICE_NAME]).stderr(std::process::Stdio::null()).status();
        let _ = std::fs::remove_file(BIN);
    }
    Ok(had)
}

fn systemctl_out(args: &[&str]) -> String {
    Command::new("systemctl").args(args).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
}

pub fn state() -> Option<String> {
    if !Path::new(UNIT_PATH).exists() {
        return None;
    }
    let active = systemctl_out(&["is-active", SERVICE_NAME]);
    Some(match active.as_str() {
        "active" => {
            let pid = systemctl_out(&["show", "-p", "MainPID", "--value", SERVICE_NAME]);
            format!("运行中（进程 {pid}）")
        }
        "activating" | "reloading" => "正在启动".into(),
        "deactivating" => "正在停止".into(),
        "failed" => "启动失败（journalctl -u lemon-galaxy-server 查看原因）".into(),
        _ => "已停止".into(),
    })
}

/// 已停止，或启动失败正等着自动重启。
pub fn is_stopped() -> bool {
    let out = systemctl_out(&["show", "-p", "ActiveState", "-p", "SubState", SERVICE_NAME]);
    out.lines().any(|l| matches!(l.trim(), "ActiveState=failed" | "ActiveState=inactive" | "SubState=auto-restart"))
}

pub fn update_target() -> Option<PathBuf> {
    (Path::new(UNIT_PATH).exists() && Path::new(BIN).exists()).then(|| PathBuf::from(BIN))
}

pub fn restart_if_installed() -> anyhow::Result<bool> {
    if !Path::new(UNIT_PATH).exists() {
        return Ok(false);
    }
    require_admin()?;
    systemctl(&["restart", SERVICE_NAME])?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(quote("/var/lib/lemon-galaxy-server"), "/var/lib/lemon-galaxy-server");
        assert_eq!(quote("1.2.3.4"), "1.2.3.4");
        assert_eq!(quote("::"), "::");
        assert_eq!(quote("/etc/my certs/a.pem"), "\"/etc/my certs/a.pem\"");
        assert_eq!(quote("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(quote("50%$HOME"), "\"50%%$$HOME\"");
        assert_eq!(quote(""), "\"\"");
    }

    #[test]
    fn unit() {
        let cfg = SvcConfig { data_dir: DATA_DIR.into(), port: 7443, public_host: Some("example.com".into()), bind: None, tls_cert: None, tls_key: None };
        let u = unit_text(&cfg);
        assert!(u.contains("ExecStart=/usr/local/bin/lemon-galaxy-server --data-dir /var/lib/lemon-galaxy-server --port 7443 --public-host example.com --log-file /var/lib/lemon-galaxy-server/logs/server.log run\n"));
        assert!(u.contains("Restart=always"));
        assert!(u.contains("WantedBy=multi-user.target"));
    }
}
