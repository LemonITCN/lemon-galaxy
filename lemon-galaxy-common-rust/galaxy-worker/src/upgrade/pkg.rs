//! Linux 安装包（deb / rpm）：本机是不是包管理器装的、怎么装新包、怎么在装之前试一下包里的程序。
//! 装包由更新守护进程在独立的 systemd 单元里做（包的 postinst 会重启被控服务，不能在服务进程里装）。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// 包名
pub const PACKAGE: &str = "lemon-galaxy-worker";
/// 包里的程序（换包前备份，回滚时换回）
pub const BINARIES: [&str; 2] = ["/usr/bin/lemon-galaxy-worker", "/usr/bin/lemon-galaxy-worker-gui"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Deb,
    Rpm,
}

impl Format {
    /// 发布文件的平台名（galaxy_proto::update::classify_asset）
    pub fn platform(self) -> &'static str {
        match self {
            Format::Deb => "linux-deb",
            Format::Rpm => "linux-rpm",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Deb => "deb",
            Format::Rpm => "rpm",
        }
    }

    pub fn parse(s: &str) -> Option<Format> {
        match s {
            "deb" => Some(Format::Deb),
            "rpm" => Some(Format::Rpm),
            _ => None,
        }
    }
}

fn output(cmd: &str, args: &[&str]) -> Option<(bool, String)> {
    let o = Command::new(cmd).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    Some((o.status.success(), String::from_utf8_lossy(&o.stdout).trim().to_string()))
}

/// `dpkg-query -W -f '${Status}\t${Version}'` 的输出 → 已装的版本
pub fn parse_dpkg_status(s: &str) -> Option<String> {
    let (status, version) = s.trim().split_once('\t')?;
    (status.split_whitespace().last() == Some("installed") && !status.contains("not-installed")).then(|| version.trim().to_string())
}

/// 本机这个包是 deb 还是 rpm 装的、装的版本。只看包管理器的记录。
pub fn detect() -> Option<(Format, Option<String>)> {
    if let Some((true, s)) = output("dpkg-query", &["-W", "-f=${Status}\t${Version}", PACKAGE]) {
        if let Some(v) = parse_dpkg_status(&s) {
            return Some((Format::Deb, (!v.is_empty()).then_some(v)));
        }
    }
    if let Some((true, v)) = output("rpm", &["-q", "--qf", "%{VERSION}", PACKAGE]) {
        return Some((Format::Rpm, (!v.is_empty()).then_some(v)));
    }
    None
}

/// 装包的命令（守护进程执行；以 root 运行，不交互）
pub fn install_command(fmt: Format, pkg: &Path) -> Command {
    match fmt {
        Format::Deb => {
            let mut c = Command::new("dpkg");
            c.args(["--force-confdef", "--force-confold", "-i"]).arg(pkg).env("DEBIAN_FRONTEND", "noninteractive");
            c
        }
        Format::Rpm => {
            let mut c = Command::new("rpm");
            c.args(["-U", "--replacepkgs"]).arg(pkg);
            c
        }
    }
}

/// 把包解到 `dest`，返回里面的被控端程序（装之前先跑一下它的 `version`，起不来的包不装）。
/// rpm 要有 rpm2cpio 与 cpio；没有时返回 None（不试，照常装）。
pub fn unpack_core(fmt: Format, pkg: &Path, dest: &Path) -> Result<Option<PathBuf>, String> {
    let _ = std::fs::remove_dir_all(dest);
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let ok = match fmt {
        Format::Deb => Command::new("dpkg-deb").arg("-x").arg(pkg).arg(dest).stdin(Stdio::null()).output().map_err(|e| format!("dpkg-deb：{e}"))?.status.success(),
        Format::Rpm => {
            let has = |c: &str| Command::new(c).arg("--version").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok();
            if !has("rpm2cpio") || !has("cpio") {
                return Ok(None);
            }
            let mut r = Command::new("rpm2cpio").arg(pkg).stdin(Stdio::null()).stdout(Stdio::piped()).spawn().map_err(|e| format!("rpm2cpio：{e}"))?;
            let out = r.stdout.take().ok_or("rpm2cpio")?;
            let c = Command::new("cpio").args(["-idm", "--quiet"]).current_dir(dest).stdin(out).status().map_err(|e| format!("cpio：{e}"))?;
            r.wait().map(|s| s.success()).unwrap_or(false) && c.success()
        }
    };
    if !ok {
        return Err(format!("解不开 {}", pkg.display()));
    }
    let core = dest.join("usr/bin/lemon-galaxy-worker");
    if !core.exists() {
        return Err("安装包里没有 /usr/bin/lemon-galaxy-worker".into());
    }
    Ok(Some(core))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpkg_status() {
        assert_eq!(parse_dpkg_status("install ok installed\t0.1.0.75").as_deref(), Some("0.1.0.75"));
        assert_eq!(parse_dpkg_status("hold ok installed\t0.1.0.75\n").as_deref(), Some("0.1.0.75"));
        // 删了但留着配置、装了一半
        assert_eq!(parse_dpkg_status("deinstall ok config-files\t0.1.0.75"), None);
        assert_eq!(parse_dpkg_status("install ok half-configured\t0.1.0.75"), None);
        assert_eq!(parse_dpkg_status("unknown ok not-installed\t"), None);
        assert_eq!(parse_dpkg_status(""), None);
    }

    #[test]
    fn formats() {
        for f in [Format::Deb, Format::Rpm] {
            assert_eq!(Format::parse(f.name()), Some(f));
        }
        assert_eq!(Format::Deb.platform(), "linux-deb");
        assert_eq!(Format::Rpm.platform(), "linux-rpm");
        assert_eq!(Format::parse("apk"), None);
    }
}
