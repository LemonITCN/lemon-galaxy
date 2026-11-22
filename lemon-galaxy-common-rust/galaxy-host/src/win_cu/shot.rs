//! 只截目标窗口：PrintWindow(PW_RENDERFULLCONTENT)，被遮挡也能截；全黑时退回从屏幕上拷那块区域。
//! 裁掉看不见的拖动边，缩到长边不超过上限，编成 JPEG。

use super::{frame_rect, Geo};
use galaxy_proto::peer::ImageData;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

/// PW_RENDERFULLCONTENT（Win8.1 起；老系统不认时退回 0）
const PW_RENDERFULLCONTENT: u32 = 2;

/// 截窗口，返回图片与截图坐标系
pub(super) fn capture(hwnd: HWND, max_side: u32) -> Option<(ImageData, Geo)> {
    let mut wr = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut wr) }.ok()?;
    let (w, h) = (wr.right - wr.left, wr.bottom - wr.top);
    if w <= 0 || h <= 0 || w > 16384 || h > 16384 {
        return None;
    }
    let mut px = grab(hwnd, &wr, w, h)?;
    // 裁到看得见的边框
    let fr = frame_rect(hwnd);
    let cl = (fr.left - wr.left).clamp(0, w);
    let ct = (fr.top - wr.top).clamp(0, h);
    let cr = (fr.right - wr.left).clamp(cl, w);
    let cb = (fr.bottom - wr.top).clamp(ct, h);
    let (cw, ch) = if cr - cl >= 2 && cb - ct >= 2 { (cr - cl, cb - ct) } else { (w, h) };
    let (cl, ct) = if cw == w && ch == h { (0, 0) } else { (cl, ct) };
    if cw != w || ch != h {
        let mut out = Vec::with_capacity((cw * ch * 4) as usize);
        for y in ct..ct + ch {
            let s = ((y * w + cl) * 4) as usize;
            out.extend_from_slice(&px[s..s + (cw * 4) as usize]);
        }
        px = out;
    }
    let (cw, ch) = (cw as u32, ch as u32);
    let scale = (max_side as f64 / cw.max(ch) as f64).min(1.0);
    let (ow, oh) = (((cw as f64 * scale).round() as u32).max(1), ((ch as f64 * scale).round() as u32).max(1));
    let rgb = resize_bgra_to_rgb(&px, cw, ch, ow, oh);
    let mut data = Vec::new();
    jpeg_encoder::Encoder::new(&mut data, 82).encode(&rgb, ow as u16, oh as u16, jpeg_encoder::ColorType::Rgb).ok()?;
    let geo = Geo { left: wr.left + cl, top: wr.top + ct, scale: ow as f64 / cw as f64 };
    Some((ImageData { mime: "image/jpeg".into(), data, width: ow, height: oh }, geo))
}

/// 把窗口画进 32 位 DIB，返回自上而下的 BGRA
fn grab(hwnd: HWND, wr: &RECT, w: i32, h: i32) -> Option<Vec<u8>> {
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bmp = match CreateDIBSection(Some(mem), &bi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(b) if !bits.is_null() => b,
            _ => {
                let _ = DeleteDC(mem);
                ReleaseDC(None, screen);
                return None;
            }
        };
        let old = SelectObject(mem, bmp.into());
        let n = (w * h * 4) as usize;
        let mut ok = PrintWindow(hwnd, mem, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool();
        if !ok {
            ok = PrintWindow(hwnd, mem, PRINT_WINDOW_FLAGS(0)).as_bool();
        }
        let _ = GdiFlush();
        let blank = |b: &[u8]| b.chunks_exact(4).step_by(7).all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0);
        if !ok || blank(std::slice::from_raw_parts(bits as *const u8, n)) {
            // 有些硬件加速的窗口 PrintWindow 出来是黑的：从屏幕上拷（这时只有没被挡住的部分是对的）
            let _ = BitBlt(mem, 0, 0, w, h, Some(screen), wr.left, wr.top, SRCCOPY | CAPTUREBLT);
            let _ = GdiFlush();
        }
        let px = std::slice::from_raw_parts(bits as *const u8, n).to_vec();
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        Some(px)
    }
}

/// 缩小用区域平均（放大或等大时取最近点），输出 RGB
fn resize_bgra_to_rgb(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let mut out = vec![0u8; (dw * dh * 3) as usize];
    let fx = sw as f64 / dw as f64;
    let fy = sh as f64 / dh as f64;
    for y in 0..dh {
        let y0 = (y as f64 * fy) as u32;
        let y1 = (((y + 1) as f64 * fy).ceil() as u32).clamp(y0 + 1, sh);
        for x in 0..dw {
            let x0 = (x as f64 * fx) as u32;
            let x1 = (((x + 1) as f64 * fx).ceil() as u32).clamp(x0 + 1, sw);
            let (mut b, mut g, mut r, mut n) = (0u32, 0u32, 0u32, 0u32);
            for yy in y0..y1 {
                let row = (yy * sw) as usize;
                for xx in x0..x1 {
                    let i = (row + xx as usize) * 4;
                    b += src[i] as u32;
                    g += src[i + 1] as u32;
                    r += src[i + 2] as u32;
                    n += 1;
                }
            }
            let o = ((y * dw + x) * 3) as usize;
            out[o] = (r / n) as u8;
            out[o + 1] = (g / n) as u8;
            out[o + 2] = (b / n) as u8;
        }
    }
    out
}
