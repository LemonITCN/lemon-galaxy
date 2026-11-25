//! 复制粘贴文件（docs/protocol.md 8.4.1）：把剪贴板文件清单写成资源管理器认的 FILEGROUPDESCRIPTORW。
//! 纯数据，不依赖 Windows 接口（本机也能跑单元测试）。

use galaxy_proto::peer::ClipOfferEntry;

/// 一个 FILEDESCRIPTORW 的字节数（shlobj_core.h，紧凑排列，各字段本来就 4 字节对齐）
pub const DESCRIPTOR_SIZE: usize = 592;
/// cFileName 能放的字符数（MAX_PATH，含结尾的 0）
const NAME_CHARS: usize = 260;

const FD_ATTRIBUTES: u32 = 0x4;
const FD_WRITESTIME: u32 = 0x20;
const FD_FILESIZE: u32 = 0x40;
/// 资源管理器显示它自己的「正在复制」进度框（Vista 起）
const FD_PROGRESSUI: u32 = 0x4000;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;

// 各字段在 FILEDESCRIPTORW 里的位置
const OFF_FLAGS: usize = 0;
const OFF_ATTRIBUTES: usize = 36;
const OFF_WRITE_TIME: usize = 56;
const OFF_SIZE_HIGH: usize = 64;
const OFF_SIZE_LOW: usize = 68;
const OFF_NAME: usize = 72;

/// Unix 秒 → FILETIME（1601 年起的 100 纳秒数）；不知道（0 或更早）时为 None
pub fn filetime(unix_secs: i64) -> Option<u64> {
    (unix_secs > 0).then(|| (unix_secs as u64 + 11_644_473_600) * 10_000_000)
}

/// 一段文件名改成 Windows 能建的：非法字符换成「_」，去掉结尾的点和空格，设备名（CON、NUL…）后面加「_」。
/// 同一段名字总是换成同一个结果，所以文件夹与它里面的项对得上。
pub fn windows_name(name: &str) -> String {
    let mut s: String = name.chars().map(|c| if c < ' ' || "<>:\"|?*\\/".contains(c) { '_' } else { c }).collect();
    while s.ends_with(['.', ' ']) {
        s.pop();
    }
    if s.is_empty() {
        return "_".into();
    }
    let stem = s.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        s.insert(stem.len(), '_');
    }
    s
}

/// 清单里的相对路径（'/' 分隔）→ 描述符里的相对路径（'\' 分隔，每段都是 Windows 能建的名字）
pub fn windows_path(path: &str) -> String {
    path.split('/').map(windows_name).collect::<Vec<_>>().join("\\")
}

/// 整份清单 → FILEGROUPDESCRIPTORW（cItems + 每项一个 FILEDESCRIPTORW）。第 i 项就是 FileContents 的 lindex i。
/// 路径超过 259 个字符（资源管理器的上限）时报错。
pub fn file_group_descriptor(entries: &[ClipOfferEntry]) -> Result<Vec<u8>, String> {
    let mut out = vec![0u8; 4 + entries.len() * DESCRIPTOR_SIZE];
    out[..4].copy_from_slice(&(entries.len() as u32).to_le_bytes());
    for (i, e) in entries.iter().enumerate() {
        let d = &mut out[4 + i * DESCRIPTOR_SIZE..4 + (i + 1) * DESCRIPTOR_SIZE];
        let name: Vec<u16> = windows_path(&e.path).encode_utf16().collect();
        if name.len() >= NAME_CHARS {
            return Err(format!("路径太长，Windows 资源管理器粘贴不了：{}", e.path));
        }
        let put = |d: &mut [u8], off: usize, v: u32| d[off..off + 4].copy_from_slice(&v.to_le_bytes());
        let mut flags = FD_ATTRIBUTES | FD_FILESIZE | FD_PROGRESSUI;
        if let Some(t) = filetime(e.mtime) {
            flags |= FD_WRITESTIME;
            put(d, OFF_WRITE_TIME, t as u32);
            put(d, OFF_WRITE_TIME + 4, (t >> 32) as u32);
        }
        put(d, OFF_FLAGS, flags);
        put(d, OFF_ATTRIBUTES, if e.is_dir { FILE_ATTRIBUTE_DIRECTORY } else { FILE_ATTRIBUTE_NORMAL });
        let size = if e.is_dir { 0 } else { e.size };
        put(d, OFF_SIZE_HIGH, (size >> 32) as u32);
        put(d, OFF_SIZE_LOW, size as u32);
        for (j, c) in name.iter().enumerate() {
            d[OFF_NAME + j * 2..OFF_NAME + j * 2 + 2].copy_from_slice(&c.to_le_bytes());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, is_dir: bool, size: u64, mtime: i64) -> ClipOfferEntry {
        ClipOfferEntry { path: path.into(), is_dir, size, mtime }
    }

    fn u32_at(b: &[u8], off: usize) -> u32 {
        u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
    }

    fn name_at(d: &[u8]) -> String {
        let w: Vec<u16> = d[OFF_NAME..DESCRIPTOR_SIZE].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|c| *c != 0).collect();
        String::from_utf16(&w).unwrap()
    }

    #[test]
    fn builds_descriptor_group() {
        let entries = vec![entry("资料", true, 0, 0), entry("资料/报告.docx", false, 5_000_000_000, 1_700_000_000), entry("a.txt", false, 3, 0)];
        let b = file_group_descriptor(&entries).unwrap();
        assert_eq!(b.len(), 4 + 3 * DESCRIPTOR_SIZE);
        assert_eq!(u32_at(&b, 0), 3);
        let d = |i: usize| &b[4 + i * DESCRIPTOR_SIZE..4 + (i + 1) * DESCRIPTOR_SIZE];
        // 文件夹
        assert_eq!(name_at(d(0)), "资料");
        assert_eq!(u32_at(d(0), OFF_ATTRIBUTES), FILE_ATTRIBUTE_DIRECTORY);
        assert_eq!(u32_at(d(0), OFF_FLAGS), FD_ATTRIBUTES | FD_FILESIZE | FD_PROGRESSUI);
        // 大文件：反斜杠、64 位大小、修改时间
        assert_eq!(name_at(d(1)), "资料\\报告.docx");
        assert_eq!(u32_at(d(1), OFF_ATTRIBUTES), FILE_ATTRIBUTE_NORMAL);
        assert_eq!(u32_at(d(1), OFF_FLAGS), FD_ATTRIBUTES | FD_FILESIZE | FD_PROGRESSUI | FD_WRITESTIME);
        assert_eq!(((u32_at(d(1), OFF_SIZE_HIGH) as u64) << 32) | u32_at(d(1), OFF_SIZE_LOW) as u64, 5_000_000_000);
        let ft = ((u32_at(d(1), OFF_WRITE_TIME + 4) as u64) << 32) | u32_at(d(1), OFF_WRITE_TIME) as u64;
        assert_eq!(ft, 133_444_736_000_000_000); // 2023-11-14 22:13:20 UTC
        assert_eq!(name_at(d(2)), "a.txt");
        assert_eq!(u32_at(d(2), OFF_SIZE_LOW), 3);
    }

    #[test]
    fn sanitizes_names_for_windows() {
        assert_eq!(windows_name("a:b?c"), "a_b_c");
        assert_eq!(windows_name("结尾. "), "结尾");
        assert_eq!(windows_name("..."), "_");
        assert_eq!(windows_name("con"), "con_");
        assert_eq!(windows_name("NUL.txt"), "NUL_.txt");
        assert_eq!(windows_name("com1.log"), "com1_.log");
        assert_eq!(windows_name("COMMENT.txt"), "COMMENT.txt");
        assert_eq!(windows_path("x:y/con/z.txt"), "x_y\\con_\\z.txt");
    }

    #[test]
    fn rejects_too_long_path() {
        let long = format!("{}/{}", "d".repeat(130), "f".repeat(129));
        assert!(file_group_descriptor(&[entry(&"d".repeat(130), true, 0, 0), entry(&long, false, 1, 0)]).is_err());
        let ok = format!("{}/{}", "d".repeat(130), "f".repeat(128));
        assert!(file_group_descriptor(&[entry(&"d".repeat(130), true, 0, 0), entry(&ok, false, 1, 0)]).is_ok());
    }
}
