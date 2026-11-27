//! 自动更新：发布文件分类、版本比较、SHA256SUMS 解析与发布签名。
//!
//! 每次发布附 `SHA256SUMS.txt` 与它的签名 `SHA256SUMS.txt.sig`（Ed25519，域 `RELEASE_SIGN_DOMAIN`，base64）。
//! 客户端用内置的 `RELEASE_SIGN_PUBKEY` 验签名，再核对下载文件的 SHA-256 出现在清单里，才会安装。

/// LemonGalaxy 发布签名公钥（私钥只在 GitHub Actions 的密钥里）。
#[cfg(not(feature = "test-release-key"))]
pub const RELEASE_SIGN_PUBKEY: [u8; 32] = OFFICIAL_RELEASE_SIGN_PUBKEY;

/// 测试构建（打开 `test-release-key` 特性）只认测试密钥，不认正式密钥：端到端测试要发「可信的新版本」，
/// 正式私钥又只在发布任务里。特性默认关、发布构建从不打开，所以正式包只认正式密钥，不会被测试密钥绕过；
/// 测试构建反过来也装不了正式发布（两边互不相通）。发布任务会核对发布文件里没有测试公钥（.github/workflows/build.yml）。
#[cfg(feature = "test-release-key")]
pub const RELEASE_SIGN_PUBKEY: [u8; 32] = TEST_RELEASE_SIGN_PUBKEY;

#[cfg_attr(feature = "test-release-key", allow(dead_code))]
const OFFICIAL_RELEASE_SIGN_PUBKEY: [u8; 32] = [
    0x40, 0xd8, 0xac, 0x3e, 0x53, 0x0b, 0xac, 0x93, 0x13, 0x2b, 0x42, 0x16, 0xcf, 0x77, 0x3d, 0xff, 0x72, 0x56, 0x66, 0x1a, 0x0b, 0x82, 0xec, 0xc2, 0xc8, 0xfd, 0xcc, 0x4f, 0x60, 0xc4, 0x2f, 0x64,
];

/// 测试密钥的公钥。私钥种子公开写在仓库里（scripts/e2e-update.sh），任何人都能用它签名，所以只有测试构建认它。
/// 种子 = SHA-256("lemon-galaxy/test-release-key/v1: only trusted by builds with feature test-release-key")。
pub const TEST_RELEASE_SIGN_PUBKEY: [u8; 32] = [
    0xf8, 0xe1, 0x36, 0x53, 0x24, 0x73, 0x98, 0x0d, 0xad, 0xbe, 0x4f, 0x7f, 0x47, 0xbf, 0x93, 0xa9, 0xc7, 0x0b, 0xa6, 0x5f, 0xb6, 0x48, 0x05, 0x0b, 0x1e, 0x11, 0xa7, 0x92, 0x76, 0x87, 0xdd, 0x01,
];

/// 这是不是只认测试密钥的测试构建（启动时打警告，`version` 也会注明）。
pub const TEST_RELEASE_KEY: bool = cfg!(feature = "test-release-key");

pub const RELEASE_SIGN_DOMAIN: &[u8] = b"lemon-galaxy/release/v1";

pub const SUMS_FILE: &str = "SHA256SUMS.txt";
pub const SUMS_SIG_FILE: &str = "SHA256SUMS.txt.sig";

/// 被控端桌面端（界面 + 被控服务一起的安装包）的组件名。平台：windows / windows7 / windowsxp（压缩包），
/// linux-deb / linux-rpm（安装包）。被控服务自动更新时，装了桌面端的机器换整套，只装了命令行的照旧只换核心程序（`worker`）。
pub const WORKER_DESKTOP: &str = "worker-desktop";

/// 发布文件对应的 (组件, 平台, 架构)。组件：control / worker / worker-desktop / mobile / server。
pub fn classify_asset(name: &str) -> Option<(&'static str, &'static str, String)> {
    let lower = name.to_ascii_lowercase();
    if let Some(x) = classify_worker_desktop(&lower) {
        return Some(x);
    }
    if lower.ends_with("-macos.dmg") {
        return Some(if lower.starts_with("lemongalaxyworker-") { ("worker", "macos", "universal".into()) } else { ("control", "macos", "universal".into()) });
    }
    if lower.ends_with("-android.apk") {
        return Some(("mobile", "android", "universal".into()));
    }
    let arch_of = |s: &str| -> String {
        let a = s.rsplit('-').next().unwrap_or("");
        let a = a.split('.').next().unwrap_or("");
        match a {
            "x64" | "amd64" => "x86_64".into(),
            "arm64" => "aarch64".into(),
            other => other.into(),
        }
    };
    // musl 静态精简版（Alpine 等）单独算一个平台，不能和 glibc 版互相更新
    if lower.starts_with("lemon-galaxy-worker-") && lower.contains("-linux-musl-") {
        return Some(("worker", "linux-musl", arch_of(&lower)));
    }
    if lower.starts_with("lemon-galaxy-worker-") && lower.contains("-linux-") {
        return Some(("worker", "linux", arch_of(&lower)));
    }
    // Windows XP 兼容版（32 位）与 Windows 7 兼容版（Vista / 7 / 8，x64 与 32 位）各算一个平台，
    // 不能和 Windows 10 起的正式版互相更新
    if lower.starts_with("lemon-galaxy-worker-") && lower.contains("-windowsxp-") && lower.ends_with(".exe") {
        return Some(("worker", "windowsxp", arch_of(&lower)));
    }
    if lower.starts_with("lemon-galaxy-worker-") && lower.contains("-windows7-") && lower.ends_with(".exe") {
        return Some(("worker", "windows7", arch_of(&lower)));
    }
    if lower.starts_with("lemon-galaxy-worker-") && lower.contains("-windows-") && lower.ends_with(".exe") {
        return Some(("worker", "windows", arch_of(&lower)));
    }
    if lower.starts_with("lemongalaxy-") && lower.contains("-windows7-") && lower.ends_with(".zip") {
        return Some(("control", "windows7", arch_of(&lower)));
    }
    if lower.starts_with("lemongalaxy-") && lower.contains("-windows-") && lower.ends_with(".zip") {
        return Some(("control", "windows", arch_of(&lower)));
    }
    if lower.starts_with("lemon-galaxy-server-") && lower.contains("-linux-") && lower.ends_with(".tar.gz") {
        return Some(("server", "linux", arch_of(lower.trim_end_matches(".tar.gz"))));
    }
    if lower.starts_with("lemon-galaxy-server-") && lower.contains("-windows-") && lower.ends_with(".zip") {
        return Some(("server", "windows", arch_of(&lower)));
    }
    // macOS 服务器只有一个 Apple 芯片与 Intel 通用的包
    if lower.starts_with("lemon-galaxy-server-") && lower.contains("-macos-") && lower.ends_with(".tar.gz") {
        return Some(("server", "macos", "universal".into()));
    }
    None
}

/// 被控端桌面端的包：`LemonGalaxyWorker-<版本>-<windows-x64|windows-arm64|windows7-x64|windows7-x86|windowsxp-x86>.zip`、
/// `lemon-galaxy-worker_<版本>_<amd64|arm64>.deb`、`lemon-galaxy-worker-<版本>.<x86_64|aarch64>.rpm`（参数已转小写）。
fn classify_worker_desktop(lower: &str) -> Option<(&'static str, &'static str, String)> {
    let norm = |a: &str| -> Option<String> {
        Some(match a {
            "x64" | "amd64" | "x86_64" => "x86_64".into(),
            "arm64" | "aarch64" => "aarch64".into(),
            "x86" => "x86".into(),
            _ => return None,
        })
    };
    if let Some(rest) = lower.strip_prefix("lemongalaxyworker-").and_then(|r| r.strip_suffix(".zip")) {
        // <版本>-<平台>-<架构>
        let mut it = rest.rsplitn(3, '-');
        let (arch, plat, ver) = (it.next()?, it.next()?, it.next()?);
        if ver.is_empty() || !ver.starts_with(|c: char| c.is_ascii_digit()) {
            return None;
        }
        let plat = match plat {
            "windows" => "windows",
            "windows7" => "windows7",
            "windowsxp" => "windowsxp",
            _ => return None,
        };
        return Some((WORKER_DESKTOP, plat, norm(arch)?));
    }
    if let Some(rest) = lower.strip_prefix("lemon-galaxy-worker_").and_then(|r| r.strip_suffix(".deb")) {
        let (ver, arch) = rest.rsplit_once('_')?;
        if ver.is_empty() || !ver.starts_with(|c: char| c.is_ascii_digit()) {
            return None;
        }
        return Some((WORKER_DESKTOP, "linux-deb", norm(arch)?));
    }
    if let Some(rest) = lower.strip_prefix("lemon-galaxy-worker-").and_then(|r| r.strip_suffix(".rpm")) {
        let (ver, arch) = rest.rsplit_once('.')?;
        if ver.is_empty() || !ver.starts_with(|c: char| c.is_ascii_digit()) {
            return None;
        }
        return Some((WORKER_DESKTOP, "linux-rpm", norm(arch)?));
    }
    None
}

/// 本机架构在发布文件里的写法（32 位 x86 写作 `x86`）。
pub fn this_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => other,
    }
}

/// 版本号按点分段的数字比较（`0.1.0.12` > `0.1.0.5`），非数字段按 0 算。
pub fn version_newer(candidate: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> { v.trim_start_matches('v').split(['.', '-', '+']).map(|p| p.parse().unwrap_or(0)).collect() };
    let (a, b) = (parse(candidate), parse(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// 解析 `sha256sum` 格式的清单：返回 (文件名, 小写十六进制 SHA-256)。
pub fn parse_sums(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let (h, n) = l.split_once(char::is_whitespace)?;
            let n = n.trim().trim_start_matches('*');
            (h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit())).then(|| (n.to_string(), h.to_ascii_lowercase()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify() {
        assert_eq!(classify_asset("LemonGalaxy-0.1.0.5-macOS.dmg"), Some(("control", "macos", "universal".into())));
        assert_eq!(classify_asset("LemonGalaxyWorker-0.1.0.5-macOS.dmg"), Some(("worker", "macos", "universal".into())));
        assert_eq!(classify_asset("LemonGalaxy-0.1.0.5-Android.apk"), Some(("mobile", "android", "universal".into())));
        assert_eq!(classify_asset("lemon-galaxy-worker-0.1.0.5-linux-aarch64"), Some(("worker", "linux", "aarch64".into())));
        assert_eq!(classify_asset("lemon-galaxy-worker-0.1.0.5-linux-musl-x86_64"), Some(("worker", "linux-musl", "x86_64".into())));
        // 被控端桌面端（界面 + 被控服务）：装了桌面端的机器整套更新
        let d = |p: &'static str, a: &str| Some(("worker-desktop", p, a.to_string()));
        assert_eq!(classify_asset("lemon-galaxy-worker_0.1.0.5_amd64.deb"), d("linux-deb", "x86_64"));
        assert_eq!(classify_asset("lemon-galaxy-worker_0.1.0.5_arm64.deb"), d("linux-deb", "aarch64"));
        assert_eq!(classify_asset("lemon-galaxy-worker-0.1.0.5.x86_64.rpm"), d("linux-rpm", "x86_64"));
        assert_eq!(classify_asset("lemon-galaxy-worker-0.1.0.5.aarch64.rpm"), d("linux-rpm", "aarch64"));
        assert_eq!(classify_asset("LemonGalaxyWorker-0.1.0.5-windows-x64.zip"), d("windows", "x86_64"));
        assert_eq!(classify_asset("LemonGalaxyWorker-0.1.0.5-windows-arm64.zip"), d("windows", "aarch64"));
        assert_eq!(classify_asset("LemonGalaxyWorker-0.1.0.62-windows7-x64.zip"), d("windows7", "x86_64"));
        assert_eq!(classify_asset("LemonGalaxyWorker-0.1.0.62-windows7-x86.zip"), d("windows7", "x86"));
        assert_eq!(classify_asset("LemonGalaxyWorker-0.1.0.62-windowsxp-x86.zip"), d("windowsxp", "x86"));
        // 认不出的平台、架构，以及 Mac 被控端（DMG，归 worker / macos）不算桌面端包
        assert_eq!(classify_asset("LemonGalaxyWorker-0.1.0.5-linux-x64.zip"), None);
        assert_eq!(classify_asset("LemonGalaxyWorker-0.1.0.5-windows-mips.zip"), None);
        assert_eq!(classify_asset("LemonGalaxyWorker-windows-x64.zip"), None);
        assert_eq!(classify_asset("lemon-galaxy-worker_0.1.0.5_i386.deb"), None);
        assert_eq!(classify_asset("lemon-galaxy-worker-debuginfo.x86_64.rpm"), None);
        assert_eq!(classify_asset("LemonGalaxyWorker-0.1.0.5-macOS.dmg"), Some(("worker", "macos", "universal".into())));
        // 主控端的 Windows 压缩包不受影响
        assert_eq!(classify_asset("LemonGalaxy-0.1.0.5-windows-arm64.zip"), Some(("control", "windows", "aarch64".into())));
        assert_eq!(classify_asset("lemon-galaxy-worker-0.1.0.5-windows-x86_64.exe"), Some(("worker", "windows", "x86_64".into())));
        assert_eq!(classify_asset("LemonGalaxy-0.1.0.5-windows-x64.zip"), Some(("control", "windows", "x86_64".into())));
        assert_eq!(classify_asset("lemon-galaxy-server-0.1.0.5-linux-x86_64.tar.gz"), Some(("server", "linux", "x86_64".into())));
        assert_eq!(classify_asset("lemon-galaxy-worker-0.1.0.5-windows7-x86_64.exe"), Some(("worker", "windows7", "x86_64".into())));
        assert_eq!(classify_asset("LemonGalaxy-0.1.0.12-windows7-x64.zip"), Some(("control", "windows7", "x86_64".into())));
        // 32 位：Vista / 7 / 8 与 XP 各一份，架构都是 x86，平台名不同
        assert_eq!(classify_asset("lemon-galaxy-worker-0.1.0.40-windows7-x86.exe"), Some(("worker", "windows7", "x86".into())));
        assert_eq!(classify_asset("lemon-galaxy-worker-0.1.0.40-windowsxp-x86.exe"), Some(("worker", "windowsxp", "x86".into())));
        assert_eq!(classify_asset("LemonGalaxy-0.1.0.12-windows-x64.zip"), Some(("control", "windows", "x86_64".into())));
        assert_eq!(classify_asset("lemon-galaxy-server-0.1.0.5-linux-aarch64.tar.gz"), Some(("server", "linux", "aarch64".into())));
        assert_eq!(classify_asset("lemon-galaxy-server-0.1.0.5-windows-x86_64.zip"), Some(("server", "windows", "x86_64".into())));
        assert_eq!(classify_asset("lemon-galaxy-server-0.1.0.5-windows-aarch64.zip"), Some(("server", "windows", "aarch64".into())));
        assert_eq!(classify_asset("lemon-galaxy-server-0.1.0.5-macos-universal.tar.gz"), Some(("server", "macos", "universal".into())));
        // 服务器的 Windows 包只认 zip
        assert_eq!(classify_asset("lemon-galaxy-server-0.1.0.5-windows-x86_64.exe"), None);
        assert_eq!(classify_asset("SHA256SUMS.txt"), None);
    }

    /// 正式构建只认正式密钥；测试密钥只在打开 test-release-key 特性的测试构建里生效。
    #[test]
    fn release_key() {
        assert_ne!(OFFICIAL_RELEASE_SIGN_PUBKEY, TEST_RELEASE_SIGN_PUBKEY);
        assert_eq!(TEST_RELEASE_KEY, cfg!(feature = "test-release-key"));
        let want = if TEST_RELEASE_KEY { TEST_RELEASE_SIGN_PUBKEY } else { OFFICIAL_RELEASE_SIGN_PUBKEY };
        assert_eq!(RELEASE_SIGN_PUBKEY, want);
    }

    #[test]
    fn versions() {
        assert!(version_newer("0.1.0.12", "0.1.0.5"));
        assert!(!version_newer("0.1.0.5", "0.1.0.5"));
        assert!(version_newer("v0.2.0", "0.1.9.99"));
        assert!(!version_newer("0.1.0", "0.1.0.1"));
    }

    #[test]
    fn sums() {
        let t = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  a.bin\nbad line\n";
        assert_eq!(parse_sums(t), vec![("a.bin".to_string(), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string())]);
    }
}
