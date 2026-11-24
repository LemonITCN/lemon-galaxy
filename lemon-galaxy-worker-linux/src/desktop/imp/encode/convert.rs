//! BGRA → NV12（BT.601 有限范围，与 openh264、NVENC 默认一致，主控端按这个显示）。按行分段多线程转。

/// 把 `src`（宽 `src_w`，BGRA）左上角 `w`×`h`（偶数）转成 NV12 写进 `y`（行距 `y_pitch`）与 `uv`（行距 `uv_pitch`）。
#[allow(clippy::too_many_arguments)]
pub fn bgra_to_nv12(src: &[u8], src_w: usize, w: usize, h: usize, y: &mut [u8], y_pitch: usize, uv: &mut [u8], uv_pitch: usize) {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).clamp(1, 8);
    let rows_per = ((h / 2).div_ceil(threads)) * 2;
    if rows_per == 0 {
        return;
    }
    std::thread::scope(|s| {
        let mut y_rest = &mut y[..];
        let mut uv_rest = &mut uv[..];
        let mut row = 0;
        while row < h {
            let n = rows_per.min(h - row);
            let (yl, ul) = ((n * y_pitch).min(y_rest.len()), (n / 2 * uv_pitch).min(uv_rest.len()));
            let (yb, yr) = std::mem::take(&mut y_rest).split_at_mut(yl);
            let (ub, ur) = std::mem::take(&mut uv_rest).split_at_mut(ul);
            y_rest = yr;
            uv_rest = ur;
            let r0 = row;
            s.spawn(move || band(src, src_w, w, r0, n, yb, y_pitch, ub, uv_pitch));
            row += n;
        }
    });
}

#[inline(always)]
fn luma(b: i32, g: i32, r: i32) -> u8 {
    (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16) as u8
}

#[allow(clippy::too_many_arguments)]
fn band(src: &[u8], src_w: usize, w: usize, row0: usize, rows: usize, y: &mut [u8], y_pitch: usize, uv: &mut [u8], uv_pitch: usize) {
    let stride = src_w * 4;
    for pair in 0..rows / 2 {
        let r = row0 + pair * 2;
        let s0 = &src[r * stride..r * stride + w * 4];
        let s1 = &src[(r + 1) * stride..(r + 1) * stride + w * 4];
        let (ya, yb) = y[pair * 2 * y_pitch..].split_at_mut(y_pitch);
        let uvr = &mut uv[pair * uv_pitch..pair * uv_pitch + w];
        for x in (0..w).step_by(2) {
            let p = |s: &[u8], i: usize| (s[i * 4] as i32, s[i * 4 + 1] as i32, s[i * 4 + 2] as i32);
            let (b0, g0, r0) = p(s0, x);
            let (b1, g1, r1) = p(s0, x + 1);
            let (b2, g2, r2) = p(s1, x);
            let (b3, g3, r3) = p(s1, x + 1);
            ya[x] = luma(b0, g0, r0);
            ya[x + 1] = luma(b1, g1, r1);
            yb[x] = luma(b2, g2, r2);
            yb[x + 1] = luma(b3, g3, r3);
            let (b, g, r) = ((b0 + b1 + b2 + b3 + 2) >> 2, (g0 + g1 + g2 + g3 + 2) >> 2, (r0 + r1 + r2 + r3 + 2) >> 2);
            uvr[x] = (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
            uvr[x + 1] = (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_and_black() {
        let (w, h) = (6, 4);
        let mut src = vec![0u8; w * h * 4];
        for px in src.chunks_mut(4).take(w * 2) {
            px.copy_from_slice(&[255, 255, 255, 255]);
        }
        let mut y = vec![0u8; w * h];
        let mut uv = vec![0u8; w * h / 2];
        bgra_to_nv12(&src, w, w, h, &mut y, w, &mut uv, w);
        assert_eq!(y[0], 235);
        assert_eq!(y[w * 2], 16);
        assert_eq!(uv[0], 128);
        assert_eq!(uv[1], 128);
    }
}
