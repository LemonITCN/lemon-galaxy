//! 剪贴板：文字读写；文件列表读写（老主控端复制粘贴文件时先把文件传到暂存区，再把这里设成那些文件；
//! 新主控端只发清单，见 clipole.rs）。

use galaxy_proto::peer::ClipboardData;
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::core::w;
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;

struct Opened;
impl Opened {
    fn open() -> Option<Self> {
        // 别的程序正占着剪贴板时稍等重试
        for _ in 0..10 {
            if unsafe { OpenClipboard(None) }.is_ok() {
                return Some(Opened);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        None
    }
}
impl Drop for Opened {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

pub fn get() -> Option<ClipboardData> {
    let _g = Opened::open()?;
    unsafe {
        // 剪贴板里是我们放的主控端文件清单（clipole.rs）：不当成本机复制了东西，免得来回传
        if IsClipboardFormatAvailable(super::clipole::private_format()).is_ok() {
            return None;
        }
        if IsClipboardFormatAvailable(CF_HDROP).is_ok() {
            if let Ok(h) = GetClipboardData(CF_HDROP) {
                let drop = HDROP(h.0);
                let n = DragQueryFileW(drop, 0xFFFF_FFFF, None);
                let mut files = vec![];
                for i in 0..n {
                    let len = DragQueryFileW(drop, i, None) as usize;
                    let mut buf = vec![0u16; len + 1];
                    DragQueryFileW(drop, i, Some(&mut buf));
                    files.push(galaxy_host::files::native_to_wire(std::path::Path::new(&String::from_utf16_lossy(&buf[..len]))));
                }
                if !files.is_empty() {
                    return Some(ClipboardData::Files(files));
                }
            }
        }
        if IsClipboardFormatAvailable(CF_UNICODETEXT).is_ok() {
            let h = GetClipboardData(CF_UNICODETEXT).ok()?;
            let g = HGLOBAL(h.0);
            let p = GlobalLock(g) as *const u16;
            if p.is_null() {
                return None;
            }
            let max = GlobalSize(g) / 2;
            let mut n = 0;
            while n < max && *p.add(n) != 0 {
                n += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
            let _ = GlobalUnlock(g);
            return Some(ClipboardData::Text(s));
        }
    }
    None
}

pub fn set(d: ClipboardData) {
    match d {
        ClipboardData::Text(t) => set_text(t),
        ClipboardData::Files(v) => set_files(v),
        ClipboardData::Image(_) => {}
    }
}

/// 写入文件列表（CF_HDROP + 「复制」效果），资源管理器里直接粘贴。
fn set_files(paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    let mut list: Vec<u16> = vec![];
    for p in &paths {
        list.extend(p.encode_utf16());
        list.push(0);
    }
    list.push(0);
    // DROPFILES：pFiles(4) + pt(8) + fNC(4) + fWide(4) = 20 字节，后面紧跟宽字符路径表
    const HEADER: usize = 20;
    let Some(_g) = Opened::open() else { return };
    unsafe {
        let _ = EmptyClipboard();
        let Ok(g) = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, HEADER + list.len() * 2) else { return };
        let p = GlobalLock(g) as *mut u8;
        if p.is_null() {
            return;
        }
        std::ptr::write_unaligned(p as *mut u32, HEADER as u32);
        std::ptr::write_unaligned(p.add(16) as *mut u32, 1);
        std::ptr::copy_nonoverlapping(list.as_ptr() as *const u8, p.add(HEADER), list.len() * 2);
        let _ = GlobalUnlock(g);
        if SetClipboardData(CF_HDROP, Some(HANDLE(g.0))).is_err() {
            return;
        }
        // 告诉资源管理器这是「复制」不是「剪切」
        let fmt = RegisterClipboardFormatW(w!("Preferred DropEffect"));
        if fmt != 0 {
            if let Ok(e) = GlobalAlloc(GMEM_MOVEABLE, 4) {
                let q = GlobalLock(e) as *mut u32;
                if !q.is_null() {
                    *q = 1; // DROPEFFECT_COPY
                    let _ = GlobalUnlock(e);
                    let _ = SetClipboardData(fmt, Some(HANDLE(e.0)));
                }
            }
        }
    }
}

fn set_text(text: String) {
    let w: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let Some(_g) = Opened::open() else { return };
    unsafe {
        let _ = EmptyClipboard();
        let Ok(g) = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2) else { return };
        let p = GlobalLock(g) as *mut u16;
        if p.is_null() {
            return;
        }
        std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
        let _ = GlobalUnlock(g);
        // 成功后内存归系统所有
        let _ = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(g.0)));
    }
}
