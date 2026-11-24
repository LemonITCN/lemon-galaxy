//! 剪贴板内容与各种 MIME 类型之间的转换（X11 选择区、Mutter、门户共用）。

use galaxy_proto::peer::{ClipboardData, ImageData};

pub const TEXT_TYPES: &[&str] = &["text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "STRING", "TEXT"];
pub const URI_LIST: &str = "text/uri-list";
pub const GNOME_FILES: &str = "x-special/gnome-copied-files";
pub const PNG: &str = "image/png";

/// 我们放进剪贴板时提供哪些类型（按优先顺序）与各自的数据。
pub fn offers(d: &ClipboardData) -> Vec<(String, Vec<u8>)> {
    match d {
        ClipboardData::Text(t) => TEXT_TYPES.iter().map(|m| (m.to_string(), if *m == "STRING" { latin1(t) } else { t.as_bytes().to_vec() })).collect(),
        ClipboardData::Image(img) => {
            let mime = if img.mime.is_empty() { PNG.to_string() } else { img.mime.clone() };
            vec![(mime, img.data.clone())]
        }
        ClipboardData::Files(paths) => {
            let uris: Vec<String> = paths.iter().map(|p| file_uri(p)).collect();
            let mut text = String::new();
            for u in &uris {
                text.push_str(u);
                text.push_str("\r\n");
            }
            let gnome = format!("copy\n{}", uris.join("\n"));
            vec![(URI_LIST.into(), text.into_bytes()), (GNOME_FILES.into(), gnome.into_bytes()), ("UTF8_STRING".into(), paths.join("\n").into_bytes())]
        }
    }
}

/// 对方提供了这些类型，选一个最好的：文件 > 图片 > 文字。
pub fn pick(available: &[String]) -> Option<String> {
    let has = |m: &str| available.iter().any(|a| a.eq_ignore_ascii_case(m));
    if has(URI_LIST) {
        return Some(URI_LIST.into());
    }
    if has(GNOME_FILES) {
        return Some(GNOME_FILES.into());
    }
    if has(PNG) {
        return Some(PNG.into());
    }
    for t in TEXT_TYPES {
        if has(t) {
            return Some(t.to_string());
        }
    }
    available.iter().find(|a| a.starts_with("image/")).cloned()
}

/// 把取到的数据按类型解析。文件列表里只认本机文件。
pub fn parse(mime: &str, data: Vec<u8>) -> Option<ClipboardData> {
    match mime {
        URI_LIST | GNOME_FILES => {
            let s = String::from_utf8_lossy(&data);
            let paths: Vec<String> = s.lines().map(|l| l.trim()).filter(|l| !l.is_empty() && !l.starts_with('#') && *l != "copy" && *l != "cut").filter_map(uri_path).collect();
            if paths.is_empty() {
                None
            } else {
                Some(ClipboardData::Files(paths))
            }
        }
        m if m.starts_with("image/") => {
            let (width, height) = if m == PNG { png_size(&data).unwrap_or((0, 0)) } else { (0, 0) };
            Some(ClipboardData::Image(ImageData { mime: m.to_string(), data, width, height }))
        }
        "STRING" => Some(ClipboardData::Text(data.iter().map(|b| *b as char).collect())),
        _ => {
            let t = String::from_utf8_lossy(&data).to_string();
            if t.is_empty() {
                None
            } else {
                Some(ClipboardData::Text(t))
            }
        }
    }
}

fn latin1(t: &str) -> Vec<u8> {
    t.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect()
}

pub fn png_size(d: &[u8]) -> Option<(u32, u32)> {
    if d.len() < 24 || &d[1..4] != b"PNG" {
        return None;
    }
    Some((u32::from_be_bytes(d[16..20].try_into().ok()?), u32::from_be_bytes(d[20..24].try_into().ok()?)))
}

pub fn file_uri(path: &str) -> String {
    let mut s = String::from("file://");
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

pub fn uri_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    // file://host/path：只认本机（空主机或 localhost）
    let path = if rest.starts_with('/') {
        rest
    } else {
        let (host, p) = rest.split_once('/')?;
        if !host.is_empty() && host != "localhost" {
            return None;
        }
        return Some(decode(&format!("/{p}")));
    };
    Some(decode(path))
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_roundtrip() {
        let p = "/home/alice/我的 文件#1.txt";
        let u = file_uri(p);
        assert!(u.starts_with("file:///home/alice/%E6"));
        assert_eq!(uri_path(&u).as_deref(), Some(p));
        assert_eq!(uri_path("file://localhost/tmp/a").as_deref(), Some("/tmp/a"));
        assert_eq!(uri_path("file://other/tmp/a"), None);
    }

    #[test]
    fn parse_files_and_pick() {
        let d = parse(GNOME_FILES, b"copy\nfile:///tmp/a\nfile:///tmp/b%20c".to_vec());
        assert!(matches!(d, Some(ClipboardData::Files(v)) if v == vec!["/tmp/a".to_string(), "/tmp/b c".to_string()]));
        let avail = vec!["TARGETS".to_string(), "UTF8_STRING".to_string(), "image/png".to_string()];
        assert_eq!(pick(&avail).as_deref(), Some(PNG));
        assert_eq!(pick(&["STRING".to_string()]).as_deref(), Some("STRING"));
    }
}
