// 资源：程序图标、清单（Common Controls 6、asInvoker、DPI 感知）、版本信息。
//
// 不用资源编译器（rc.exe / windres）：这里直接写出二进制的 .res 文件交给链接器。MSVC 的 link.exe 与 llvm-mingw 的
// lld 都把 .res 当普通输入（按文件头认出来转成 .rsrc 节），两套工具链同一份做法，也不要多装工具。
//
// 图标用 WPF 版的 gui/Resources/LemonGalaxyWorker.ico。里面的条目都是 PNG 压缩的，XP 不认（Vista 起才认）：
// 256 以下的解成 32 位带 Alpha 的位图条目，256 的保留 PNG（XP 会跳过它，Vista 起大图标用它）。
// 另外把 16–48 的像素导出给托盘（运行时在标志上画状态点，见 src/tray.rs）。
use std::fmt::Write as _;
use std::path::PathBuf;

const ICON: &str = "../gui/Resources/LemonGalaxyWorker.ico";
const MANIFEST: &str = "app.manifest";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={ICON}");
    println!("cargo:rerun-if-changed={MANIFEST}");
    println!("cargo:rerun-if-env-changed=LEMONGALAXY_BUILD_VERSION");
    println!("cargo:rerun-if-env-changed=LEMONGALAXY_BUILD_NUMBER");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let ico = std::fs::read(ICON).unwrap_or_else(|e| panic!("读不到图标 {ICON}：{e}"));
    let images = parse_ico(&ico);

    // 托盘用的小图（RGBA，按边长）
    let mut src = String::from("// build.rs 生成：托盘用的标志像素（RGBA，自上而下）\npub const PIXELS: &[(u32, &[u8])] = &[\n");
    for im in images.iter().filter(|i| i.size <= 48) {
        let p = out.join(format!("icon-{}.rgba", im.size));
        std::fs::write(&p, &im.rgba).expect("写 OUT_DIR");
        let _ = writeln!(src, "    ({}, include_bytes!({:?})),", im.size, p.display().to_string());
    }
    src.push_str("];\n");
    std::fs::write(out.join("icon_pixels.rs"), src).expect("写 OUT_DIR");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest = std::fs::read(MANIFEST).expect("读清单");
    let res = build_res(&images, &manifest, &app_version());
    let path = out.join("LemonGalaxyWorker.res");
    std::fs::write(&path, res).expect("写 .res");
    println!("cargo:rustc-link-arg-bins={}", path.display());
    // MSVC 的 link.exe 别再自己生成清单（清单已在 .res 里，两份会冲突）
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/MANIFEST:NO");
    }
}

/// 与 galaxy-proto 的 build.rs 同一个算法（界面「关于」里显示的 APP_VERSION 与文件版本一致）
fn app_version() -> String {
    let base = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let tag = std::env::var("LEMONGALAXY_BUILD_VERSION").unwrap_or_default();
    let tag = tag.trim().trim_start_matches('v');
    let num = std::env::var("LEMONGALAXY_BUILD_NUMBER").unwrap_or_default();
    if !tag.is_empty() {
        tag.to_string()
    } else if !num.trim().is_empty() {
        format!("{base}.{}", num.trim())
    } else {
        base
    }
}

// ---------------- 图标 ----------------

struct Image {
    size: u32,
    /// RGBA，自上而下
    rgba: Vec<u8>,
    /// 原始 PNG
    png: Vec<u8>,
}

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn u32be(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn parse_ico(d: &[u8]) -> Vec<Image> {
    assert!(d.len() > 6 && u16le(d, 2) == 1, "不是 .ico 文件");
    let n = u16le(d, 4) as usize;
    let mut v = vec![];
    for i in 0..n {
        let e = 6 + 16 * i;
        let size = u32le(d, e + 8) as usize;
        let off = u32le(d, e + 12) as usize;
        let data = &d[off..off + size];
        assert!(data.starts_with(b"\x89PNG\r\n\x1a\n"), "图标第 {i} 项不是 PNG（这里只处理 PNG 条目）");
        let (w, h, rgba) = decode_png(data);
        assert_eq!(w, h, "图标要是正方形");
        v.push(Image { size: w, rgba, png: data.to_vec() });
    }
    v.sort_by_key(|i| i.size);
    v
}

/// 只解图标里用到的格式：8 位 RGBA、不隔行
fn decode_png(p: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut pos = 8;
    let (mut w, mut h) = (0u32, 0u32);
    let mut idat = vec![];
    while pos + 8 <= p.len() {
        let len = u32be(p, pos) as usize;
        let kind = &p[pos + 4..pos + 8];
        let body = &p[pos + 8..pos + 8 + len];
        match kind {
            b"IHDR" => {
                w = u32be(body, 0);
                h = u32be(body, 4);
                assert!(body[8] == 8 && body[9] == 6 && body[12] == 0, "只支持 8 位 RGBA、不隔行的 PNG");
            }
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len;
    }
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&idat).expect("PNG 解压失败");
    let stride = w as usize * 4;
    let mut out = vec![0u8; stride * h as usize];
    let mut prev = vec![0u8; stride];
    for y in 0..h as usize {
        let line = &raw[y * (stride + 1)..(y + 1) * (stride + 1)];
        let f = line[0];
        let src = &line[1..];
        let mut cur = vec![0u8; stride];
        for x in 0..stride {
            let a = if x >= 4 { cur[x - 4] as i16 } else { 0 };
            let b = prev[x] as i16;
            let c = if x >= 4 { prev[x - 4] as i16 } else { 0 };
            let pred = match f {
                0 => 0,
                1 => a,
                2 => b,
                3 => (a + b) / 2,
                4 => {
                    let pp = a + b - c;
                    let (pa, pb, pc) = ((pp - a).abs(), (pp - b).abs(), (pp - c).abs());
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                _ => panic!("PNG 过滤类型 {f} 不认识"),
            };
            cur[x] = src[x].wrapping_add(pred as u8);
        }
        out[y * stride..(y + 1) * stride].copy_from_slice(&cur);
        prev = cur;
    }
    (w, h, out)
}

/// 图标条目：32 位位图（BITMAPINFOHEADER + 颜色自下而上 BGRA + 1 位掩码），256 的用 PNG
fn icon_entry(im: &Image) -> Vec<u8> {
    if im.size >= 256 {
        return im.png.clone();
    }
    let s = im.size as usize;
    let mask_row = s.div_ceil(32) * 4;
    let mut b = vec![];
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&(s as i32).to_le_bytes());
    b.extend_from_slice(&(2 * s as i32).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&((s * s * 4 + mask_row * s) as u32).to_le_bytes());
    b.extend_from_slice(&[0u8; 16]);
    for y in (0..s).rev() {
        for x in 0..s {
            let p = &im.rgba[(y * s + x) * 4..];
            b.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    for y in (0..s).rev() {
        let mut row = vec![0u8; mask_row];
        for x in 0..s {
            if im.rgba[(y * s + x) * 4 + 3] == 0 {
                row[x / 8] |= 0x80 >> (x % 8);
            }
        }
        b.extend_from_slice(&row);
    }
    b
}

// ---------------- .res ----------------

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
const RT_MANIFEST: u16 = 24;
const LANG: u16 = 0x0409;

fn pad4(b: &mut Vec<u8>) {
    while b.len() % 4 != 0 {
        b.push(0);
    }
}

fn res_entry(out: &mut Vec<u8>, kind: u16, id: u16, flags: u16, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes());
    out.extend_from_slice(&[0xFF, 0xFF]);
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&[0xFF, 0xFF]);
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&LANG.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(data);
    pad4(out);
}

fn build_res(images: &[Image], manifest: &[u8], version: &str) -> Vec<u8> {
    let mut out = vec![];
    // 开头一项空资源（.res 文件的格式要求）
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes());
    out.extend_from_slice(&[0xFF, 0xFF, 0, 0, 0xFF, 0xFF, 0, 0]);
    out.extend_from_slice(&[0u8; 16]);

    let mut group = vec![];
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(images.len() as u16).to_le_bytes());
    for (i, im) in images.iter().enumerate() {
        let id = (i + 1) as u16;
        let data = icon_entry(im);
        res_entry(&mut out, RT_ICON, id, 0x1010, &data);
        let wh = if im.size >= 256 { 0 } else { im.size as u8 };
        group.extend_from_slice(&[wh, wh, 0, 0]);
        group.extend_from_slice(&1u16.to_le_bytes());
        group.extend_from_slice(&32u16.to_le_bytes());
        group.extend_from_slice(&(data.len() as u32).to_le_bytes());
        group.extend_from_slice(&id.to_le_bytes());
    }
    // 图标组 1：资源管理器、窗口、托盘都用它（src/win/mod.rs 的 APP_ICON）
    res_entry(&mut out, RT_GROUP_ICON, 1, 0x1030, &group);
    res_entry(&mut out, RT_MANIFEST, 1, 0x0030, manifest);
    res_entry(&mut out, RT_VERSION, 1, 0x0030, &version_info(version));
    out
}

// ---------------- 版本信息 ----------------

fn utf16z(s: &str) -> Vec<u8> {
    s.encode_utf16().chain(std::iter::once(0)).flat_map(|c| c.to_le_bytes()).collect()
}

enum Val<'a> {
    None,
    Text(&'a str),
    Bin(&'a [u8]),
}

/// 版本信息里的一个块：wLength、wValueLength、wType、键、值、子块（各自 4 字节对齐）
fn block(key: &str, value: Val, children: &[Vec<u8>]) -> Vec<u8> {
    let mut b = vec![0u8; 6];
    b.extend_from_slice(&utf16z(key));
    pad4(&mut b);
    let (vlen, kind) = match value {
        Val::None => (0u16, 1u16),
        Val::Text(s) => {
            let w = utf16z(s);
            b.extend_from_slice(&w);
            ((w.len() / 2) as u16, 1)
        }
        Val::Bin(d) => {
            b.extend_from_slice(d);
            (d.len() as u16, 0)
        }
    };
    for c in children {
        pad4(&mut b);
        b.extend_from_slice(c);
    }
    let len = b.len() as u16;
    b[0..2].copy_from_slice(&len.to_le_bytes());
    b[2..4].copy_from_slice(&vlen.to_le_bytes());
    b[4..6].copy_from_slice(&kind.to_le_bytes());
    b
}

fn version_info(version: &str) -> Vec<u8> {
    let mut n = [0u16; 4];
    for (i, p) in version.split(['.', '-', '+']).take(4).enumerate() {
        n[i] = p.parse().unwrap_or(0);
    }
    let ms = (n[0] as u32) << 16 | n[1] as u32;
    let ls = (n[2] as u32) << 16 | n[3] as u32;
    let mut fixed = vec![];
    for v in [0xFEEF04BDu32, 0x0001_0000, ms, ls, ms, ls, 0x3F, 0, 0x0004_0004, 1, 0, 0, 0] {
        fixed.extend_from_slice(&v.to_le_bytes());
    }
    let strings: Vec<Vec<u8>> = [
        ("CompanyName", "LemonIT.CN"),
        ("FileDescription", "LemonGalaxy Worker"),
        ("FileVersion", version),
        ("InternalName", "LemonGalaxyWorker"),
        ("LegalCopyright", "Copyright © 2026 LemonIT.CN · MIT License"),
        ("OriginalFilename", "LemonGalaxyWorker.exe"),
        ("ProductName", "LemonGalaxy Worker"),
        ("ProductVersion", version),
    ]
    .iter()
    .map(|(k, v)| block(k, Val::Text(v), &[]))
    .collect();
    let table = block("040904b0", Val::None, &strings);
    let sfi = block("StringFileInfo", Val::None, &[table]);
    let translation = [0x09u8, 0x04, 0xB0, 0x04];
    let var = block("Translation", Val::Bin(&translation), &[]);
    let vfi = block("VarFileInfo", Val::None, &[var]);
    block("VS_VERSION_INFO", Val::Bin(&fixed), &[sfi, vfi])
}
