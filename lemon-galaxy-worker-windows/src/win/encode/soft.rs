//! H.264 软件编码（openh264，屏幕内容实时模式）。没有可用的硬件编码器时用。输出 Annex-B，关键帧带 SPS/PPS。
//! openh264 的封装没有运行时改码率的接口，改码率要重建（会出关键帧）。

use openh264::encoder::{BitRate, Encoder, EncoderConfig, FrameRate, FrameType, IntraFramePeriod, RateControlMode, UsageType};
use openh264::formats::{BgraSliceU8, YUVBuffer};
use openh264::OpenH264API;

pub struct H264 {
    enc: Encoder,
    yuv: YUVBuffer,
    /// 编码尺寸（偶数）
    pub width: u32,
    pub height: u32,
    crop: Vec<u8>,
}

impl H264 {
    pub fn new(width: u32, height: u32, fps: u32, bitrate_kbps: u32) -> anyhow::Result<Self> {
        let (w, h) = (width & !1, height & !1);
        anyhow::ensure!(w >= 16 && h >= 16, "画面太小 {width}x{height}");
        let cfg = EncoderConfig::new()
            .bitrate(BitRate::from_bps(bitrate_kbps.clamp(200, 200_000) * 1000))
            .max_frame_rate(FrameRate::from_hz(fps.clamp(1, 60) as f32))
            .rate_control_mode(RateControlMode::Bitrate)
            .usage_type(UsageType::ScreenContentRealTime)
            .skip_frames(true)
            // 每 10 秒左右一个关键帧，丢包后靠主控端请求关键帧恢复
            .intra_frame_period(IntraFramePeriod::from_num_frames(fps.clamp(1, 60) * 10))
            .num_threads(std::thread::available_parallelism().map(|n| n.get().min(4) as u16).unwrap_or(2));
        let enc = Encoder::with_api_config(OpenH264API::from_source(), cfg)?;
        Ok(Self { enc, yuv: YUVBuffer::new(w as usize, h as usize), width: w, height: h, crop: Vec::new() })
    }

    pub fn force_keyframe(&mut self) {
        self.enc.force_intra_frame();
    }

    /// 编码一帧 BGRA（宽高可以是奇数，会裁掉最后一行 / 列）。返回 (数据, 是否关键帧)；被跳过的帧返回 None。
    pub fn encode(&mut self, bgra: &[u8], width: u32, height: u32) -> anyhow::Result<Option<(Vec<u8>, bool)>> {
        let (w, h) = (self.width as usize, self.height as usize);
        let src: &[u8] = if width as usize == w && height as usize == h {
            bgra
        } else {
            self.crop.resize(w * h * 4, 0);
            let sw = width as usize * 4;
            for y in 0..h.min(height as usize) {
                self.crop[y * w * 4..(y + 1) * w * 4].copy_from_slice(&bgra[y * sw..y * sw + w * 4]);
            }
            &self.crop
        };
        self.yuv.read_bgra8(BgraSliceU8::new(src, (w, h)));
        let bs = self.enc.encode(&self.yuv)?;
        let kind = bs.frame_type();
        if matches!(kind, FrameType::Skip | FrameType::Invalid) {
            return Ok(None);
        }
        let data = bs.to_vec();
        if data.is_empty() {
            return Ok(None);
        }
        Ok(Some((data, matches!(kind, FrameType::IDR | FrameType::I))))
    }
}
