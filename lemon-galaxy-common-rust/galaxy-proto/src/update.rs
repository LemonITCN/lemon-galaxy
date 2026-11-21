//! 自动更新：发布文件分类、版本比较、SHA256SUMS 解析与发布签名。
//!
//! 每次发布附 `SHA256SUMS.txt` 与它的签名 `SHA256SUMS.txt.sig`（Ed25519，域 `RELEASE_SIGN_DOMAIN`，base64）。
//! 客户端用内置的 `RELEASE_SIGN_PUBKEY` 验签名，再核对下载文件的 SHA-256 出现在清单里，才会安装。

/// LemonGalaxy 发布签名公钥（私钥只在 GitHub Actions 的密钥里）。
pub const RELEASE_SIGN_PUBKEY: [u8; 32] = [
    0x40, 0xd8, 0xac, 0x3e, 0x53, 0x0b, 0xac, 0x93, 0x13, 0x2b, 0x42, 0x16, 0xcf, 0x77, 0x3d, 0xff, 0x72, 0x56, 0x66, 0x1a, 0x0b, 0x82, 0xec, 0xc2, 0xc8, 0xfd, 0xcc, 0x4f, 0x60, 0xc4, 0x2f, 0x64,
];

pub const RELEASE_SIGN_DOMAIN: &[u8] = b"lemon-galaxy/release/v1";

pub const SUMS_FILE: &str = "SHA256SUMS.txt";
pub const SUMS_SIG_FILE: &str = "SHA256SUMS.txt.sig";

/// 发布文件对应的 (组件, 平台, 架构)。组件：control / worker / mobile / server。
pub fn classify_asset(name: &str) -> Option<(&'static str, &'static str, String)> {
    let lower = name.to_ascii_lowercase();
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
    if lower.starts_with("lemon-galaxy-worker-") && lower.contains("-linux-") {
        return Some(("worker", "linux", arch_of(&lower)));
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
    None
}

/// 本机架构在发布文件里的写法。
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
        assert_eq!(classify_asset("lemon-galaxy-worker-0.1.0.5-windows-x86_64.exe"), Some(("worker", "windows", "x86_64".into())));
        assert_eq!(classify_asset("LemonGalaxy-0.1.0.5-windows-x64.zip"), Some(("control", "windows", "x86_64".into())));
        assert_eq!(classify_asset("lemon-galaxy-server-0.1.0.5-linux-x86_64.tar.gz"), Some(("server", "linux", "x86_64".into())));
        assert_eq!(classify_asset("lemon-galaxy-worker-0.1.0.5-windows7-x86_64.exe"), Some(("worker", "windows7", "x86_64".into())));
        assert_eq!(classify_asset("LemonGalaxy-0.1.0.12-windows7-x64.zip"), Some(("control", "windows7", "x86_64".into())));
        assert_eq!(classify_asset("LemonGalaxy-0.1.0.12-windows-x64.zip"), Some(("control", "windows", "x86_64".into())));
        assert_eq!(classify_asset("lemon-galaxy-server-0.1.0.5-linux-aarch64.tar.gz"), Some(("server", "linux", "aarch64".into())));
        assert_eq!(classify_asset("lemon-galaxy-server-0.1.0.5-windows-x86_64.zip"), Some(("server", "windows", "x86_64".into())));
        assert_eq!(classify_asset("lemon-galaxy-server-0.1.0.5-windows-aarch64.zip"), Some(("server", "windows", "aarch64".into())));
        // 服务器的 Windows 包只认 zip
        assert_eq!(classify_asset("lemon-galaxy-server-0.1.0.5-windows-x86_64.exe"), None);
        assert_eq!(classify_asset("SHA256SUMS.txt"), None);
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
