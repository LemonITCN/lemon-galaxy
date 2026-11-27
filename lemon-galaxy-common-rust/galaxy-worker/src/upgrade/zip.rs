//! 解开 Windows 桌面端的压缩包（PowerShell Compress-Archive 打的 zip：存储或 deflate、路径分隔符可能是反斜杠、没有 zip64）。
//! 只解到指定目录下：拒绝绝对路径、盘符、`..`，限制条目数与总大小。

use std::io::Read;
use std::path::{Component, Path, PathBuf};

const MAX_ENTRIES: usize = 10_000;
const MAX_TOTAL: u64 = 1 << 30;

fn le16(b: &[u8], at: usize) -> Option<usize> {
    b.get(at..at.checked_add(2)?).map(|s| u16::from_le_bytes([s[0], s[1]]) as usize)
}

fn le32(b: &[u8], at: usize) -> Option<usize> {
    b.get(at..at.checked_add(4)?).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize)
}

/// 压缩包里的路径 → 安全的相对路径（目录条目返回 None）
fn safe_rel(name: &str) -> Result<Option<PathBuf>, String> {
    let n = name.replace('\\', "/");
    if n.ends_with('/') {
        return Ok(None);
    }
    if n.starts_with('/') || n.contains(':') {
        return Err(format!("压缩包里有不安全的路径：{name}"));
    }
    let mut out = PathBuf::new();
    for c in Path::new(&n).components() {
        match c {
            Component::Normal(s) => out.push(s),
            Component::CurDir => {}
            _ => return Err(format!("压缩包里有不安全的路径：{name}")),
        }
    }
    if out.as_os_str().is_empty() {
        return Ok(None);
    }
    Ok(Some(out))
}

/// 解开 `data` 到 `dest`，返回解出的文件（相对路径）。
pub fn extract_all(data: &[u8], dest: &Path) -> Result<Vec<PathBuf>, String> {
    let bad = || "压缩包格式不对".to_string();
    let last = data.len().checked_sub(22).ok_or_else(bad)?;
    let first = last.saturating_sub(65535);
    let eocd = (first..=last).rev().find(|&i| le32(data, i) == Some(0x0605_4b50)).ok_or_else(bad)?;
    let count = le16(data, eocd + 10).ok_or_else(bad)?;
    let mut p = le32(data, eocd + 16).ok_or_else(bad)?;
    if count == 0xffff || p == 0xffff_ffff {
        return Err("不支持 zip64 压缩包".into());
    }
    if count > MAX_ENTRIES {
        return Err("压缩包里的文件太多".into());
    }
    let mut out = vec![];
    let mut total = 0u64;
    for _ in 0..count {
        if le32(data, p) != Some(0x0201_4b50) {
            return Err(bad());
        }
        let flags = le16(data, p + 8).ok_or_else(bad)?;
        let method = le16(data, p + 10).ok_or_else(bad)?;
        let crc = le32(data, p + 16).ok_or_else(bad)? as u32;
        let packed = le32(data, p + 20).ok_or_else(bad)?;
        let size = le32(data, p + 24).ok_or_else(bad)?;
        let name_len = le16(data, p + 28).ok_or_else(bad)?;
        let extra_len = le16(data, p + 30).ok_or_else(bad)?;
        let comment_len = le16(data, p + 32).ok_or_else(bad)?;
        let local = le32(data, p + 42).ok_or_else(bad)?;
        let raw_name = data.get(p + 46..p + 46 + name_len).ok_or_else(bad)?;
        let name = String::from_utf8_lossy(raw_name).into_owned();
        p += 46 + name_len + extra_len + comment_len;
        if flags & 1 != 0 {
            return Err(format!("压缩包里的 {name} 加了密"));
        }
        let Some(rel) = safe_rel(&name)? else { continue };
        total += size as u64;
        if total > MAX_TOTAL {
            return Err("压缩包解开太大".into());
        }
        if le32(data, local) != Some(0x0403_4b50) {
            return Err(bad());
        }
        let start = local + 30 + le16(data, local + 26).ok_or_else(bad)? + le16(data, local + 28).ok_or_else(bad)?;
        let raw = data.get(start..start.checked_add(packed).ok_or_else(bad)?).ok_or_else(bad)?;
        let content = match method {
            0 => raw.to_vec(),
            8 => {
                let mut v = Vec::with_capacity(size);
                flate2::read::DeflateDecoder::new(raw).take(size as u64 + 1).read_to_end(&mut v).map_err(|e| format!("{name}：{e}"))?;
                v
            }
            m => return Err(format!("{name} 用了不支持的压缩方式 {m}")),
        };
        let mut c = flate2::Crc::new();
        c.update(&content);
        if content.len() != size || c.sum() != crc {
            return Err(format!("压缩包里的 {name} 校验不对"));
        }
        let target = dest.join(&rel);
        if let Some(d) = target.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}：{e}", d.display()))?;
        }
        std::fs::write(&target, &content).map_err(|e| format!("{}：{e}", target.display()))?;
        out.push(rel);
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    /// 写一个 zip（每项可选存储或 deflate），与 Compress-Archive / Info-ZIP 的布局一致。
    pub(crate) fn make_zip(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let (mut out, mut cd) = (Vec::new(), Vec::new());
        for (name, data, deflate) in entries {
            let packed = if *deflate {
                let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(data).unwrap();
                e.finish().unwrap()
            } else {
                data.to_vec()
            };
            let mut crc = flate2::Crc::new();
            crc.update(data);
            let method: u16 = if *deflate { 8 } else { 0 };
            let off = out.len() as u32;
            out.extend(0x0403_4b50u32.to_le_bytes());
            out.extend([20u16, 0, method, 0, 0].iter().flat_map(|v| v.to_le_bytes()));
            out.extend([crc.sum(), packed.len() as u32, data.len() as u32].iter().flat_map(|v| v.to_le_bytes()));
            out.extend([name.len() as u16, 0].iter().flat_map(|v| v.to_le_bytes()));
            out.extend(name.as_bytes());
            out.extend(&packed);
            cd.extend(0x0201_4b50u32.to_le_bytes());
            cd.extend([20u16, 20, 0, method, 0, 0].iter().flat_map(|v| v.to_le_bytes()));
            cd.extend([crc.sum(), packed.len() as u32, data.len() as u32].iter().flat_map(|v| v.to_le_bytes()));
            cd.extend([name.len() as u16, 0, 0, 0, 0].iter().flat_map(|v| v.to_le_bytes()));
            cd.extend([0u32, off].iter().flat_map(|v| v.to_le_bytes()));
            cd.extend(name.as_bytes());
        }
        let cd_off = out.len() as u32;
        out.extend(&cd);
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend([0u16, 0, entries.len() as u16, entries.len() as u16].iter().flat_map(|v| v.to_le_bytes()));
        out.extend([cd.len() as u32, cd_off].iter().flat_map(|v| v.to_le_bytes()));
        out.extend(0u16.to_le_bytes());
        out
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lg-upd-zip-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn extracts_compress_archive_layout() {
        let exe: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        // Compress-Archive：同名文件夹在最上层，反斜杠分隔，有目录条目
        let z = make_zip(&[
            ("LemonGalaxyWorker-1.2.3.4-windows-x64\\", b"", false),
            ("LemonGalaxyWorker-1.2.3.4-windows-x64\\LemonGalaxyWorker.exe", &exe, true),
            ("LemonGalaxyWorker-1.2.3.4-windows-x64\\Resources\\Locales\\en.json", b"{}", false),
            ("LemonGalaxyWorker-1.2.3.4-windows-x64/lemon-galaxy-worker.exe", b"MZ", true),
        ]);
        let d = tmp("ok");
        let files = extract_all(&z, &d).unwrap();
        assert_eq!(files.len(), 3);
        let top = d.join("LemonGalaxyWorker-1.2.3.4-windows-x64");
        assert_eq!(std::fs::read(top.join("LemonGalaxyWorker.exe")).unwrap(), exe);
        assert_eq!(std::fs::read(top.join("Resources").join("Locales").join("en.json")).unwrap(), b"{}");
        assert_eq!(std::fs::read(top.join("lemon-galaxy-worker.exe")).unwrap(), b"MZ");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rejects_unsafe_and_broken() {
        for evil in ["../evil.exe", "a/../../evil.exe", "/etc/passwd", "C:\\Windows\\evil.exe", "a\\..\\..\\evil"] {
            let d = tmp("evil");
            assert!(extract_all(&make_zip(&[(evil, b"x", false)]), &d).is_err(), "{evil}");
            assert!(!std::env::temp_dir().join("evil.exe").exists());
            let _ = std::fs::remove_dir_all(&d);
        }
        let z = make_zip(&[("a.bin", &[7u8; 5000], true)]);
        let mut broken = z.clone();
        broken[40] ^= 0xff;
        let d = tmp("broken");
        assert!(extract_all(&broken, &d).is_err());
        assert!(extract_all(&z[..z.len() - 30], &d).is_err());
        assert!(extract_all(b"not a zip", &d).is_err());
        assert!(extract_all(&[], &d).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
