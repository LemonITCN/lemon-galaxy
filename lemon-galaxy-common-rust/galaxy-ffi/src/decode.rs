//! H.264 软件解码（openh264），给没有现成硬件解码管线的界面用（Windows 主控端）。输出 BGRA。

use crate::{GalaxyError, Result};
use openh264::decoder::Decoder;
use openh264::formats::YUVSource;
use parking_lot::Mutex;
use std::sync::Arc;

#[derive(uniffi::Record, Clone, Debug)]
pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,
    /// 每行 width * 4 字节，B G R A 顺序（A 恒为 255）
    pub bgra: Vec<u8>,
}

#[derive(uniffi::Object)]
pub struct H264Decoder {
    dec: Mutex<Decoder>,
}

#[uniffi::export]
impl H264Decoder {
    #[uniffi::constructor]
    pub fn new() -> Result<Arc<Self>> {
        let dec = Decoder::new().map_err(|e| GalaxyError::Other { detail: format!("解码器启动失败：{e}") })?;
        Ok(Arc::new(Self { dec: Mutex::new(dec) }))
    }

    /// 解码一帧（Annex-B，可含 SPS/PPS）。还没有可显示的画面时返回 None。
    pub fn decode(&self, data: Vec<u8>) -> Result<Option<DecodedFrame>> {
        let mut d = self.dec.lock();
        let yuv = match d.decode(&data) {
            Ok(Some(y)) => y,
            Ok(None) => return Ok(None),
            Err(e) => return Err(GalaxyError::Protocol { detail: format!("解码失败：{e}") }),
        };
        let (w, h) = yuv.dimensions();
        let (sy, su, sv) = yuv.strides();
        let mut out = vec![0u8; w * h * 4];
        yuv_to_bgra(yuv.y(), yuv.u(), yuv.v(), sy, su, sv, w, h, &mut out);
        Ok(Some(DecodedFrame { width: w as u32, height: h as u32, bgra: out }))
    }
}

/// BT.601 有限范围 YUV420 转 BGRA（整数运算）。
#[allow(clippy::too_many_arguments)]
fn yuv_to_bgra(y: &[u8], u: &[u8], v: &[u8], sy: usize, su: usize, sv: usize, w: usize, h: usize, out: &mut [u8]) {
    for row in 0..h {
        let yr = &y[row * sy..row * sy + w];
        let ur = &u[(row / 2) * su..];
        let vr = &v[(row / 2) * sv..];
        let o = &mut out[row * w * 4..(row + 1) * w * 4];
        for col in 0..w {
            let c = (yr[col] as i32 - 16) * 298;
            let d = ur[col / 2] as i32 - 128;
            let e = vr[col / 2] as i32 - 128;
            let r = (c + 409 * e + 128) >> 8;
            let g = (c - 100 * d - 208 * e + 128) >> 8;
            let b = (c + 516 * d + 128) >> 8;
            let px = &mut o[col * 4..col * 4 + 4];
            px[0] = b.clamp(0, 255) as u8;
            px[1] = g.clamp(0, 255) as u8;
            px[2] = r.clamp(0, 255) as u8;
            px[3] = 255;
        }
    }
}
