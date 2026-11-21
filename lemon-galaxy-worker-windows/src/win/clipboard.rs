//! 剪贴板：文字读写；文件列表只读（复制粘贴文件走文件传输）。

use galaxy_proto::peer::ClipboardData;
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
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
    let ClipboardData::Text(text) = d else { return };
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
