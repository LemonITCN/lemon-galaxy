//! 画面编码：H.264 Annex-B，关键帧带 SPS/PPS，不带 AUD（与其他被控端相同）。
//!
//! 按顺序试（运行时检测，任何一步失败就试下一个），见 docs/linux-desktop.md 第 6 节：
//! 1. NVIDIA NVENC（nvenc.rs）：libnvidia-encode + libcuda，BGRA 直接上传到显存，显卡自己转色彩空间；
//! 2. VA-API（vaapi.rs）：Intel、AMD 等，CPU 转 NV12 后上传；
//! 3. openh264 软件编码（soft.rs）。
//!
//! 环境变量 LEMON_GALAXY_VIDEO_ENCODER=auto|nvenc|vaapi|software 可指定；
//! LEMON_GALAXY_VAAPI_DEVICE=/dev/dri/renderD129 指定 VA 设备，LEMON_GALAXY_CUDA_DEVICE=1 指定第几块 NVIDIA 卡。

pub mod convert;
pub mod dl;
mod nvenc;
mod soft;
mod vaapi;

/// 编好的一帧（一个访问单元）。
pub struct EncodedFrame {
    pub data: Vec<u8>,
    pub keyframe: bool,
}

/// 硬件编码器：输入内存里的 BGRA。
trait HwEncoder: Send {
    fn name(&self) -> String;
    fn encode(&mut self, bgra: &[u8], src_w: u32, keyframe: bool, pts_us: u64) -> anyhow::Result<Vec<EncodedFrame>>;
    /// 运行时改码率（不出关键帧）；不支持返回 false（调用方重建编码器）
    fn set_bitrate(&mut self, kbps: u32) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Nvenc,
    Vaapi,
    Software,
}

#[derive(Clone, Debug, Default)]
pub struct Request {
    /// None 为自动
    pub backend: Option<Backend>,
}

impl Request {
    pub fn software() -> Self {
        Self { backend: Some(Backend::Software) }
    }
    pub fn from_env() -> Self {
        std::env::var("LEMON_GALAXY_VIDEO_ENCODER").map(|s| Self::parse(&s)).unwrap_or_default()
    }
    pub fn parse(s: &str) -> Self {
        let backend = match s.trim().to_ascii_lowercase().as_str() {
            "nvenc" | "nvidia" | "cuda" => Some(Backend::Nvenc),
            "vaapi" | "va" | "intel" | "amd" | "qsv" => Some(Backend::Vaapi),
            "software" | "soft" | "openh264" | "sw" => Some(Backend::Software),
            _ => None,
        };
        Self { backend }
    }
    fn allows(&self, b: Backend) -> bool {
        self.backend.map(|x| x == b).unwrap_or(true)
    }
}

enum Inner {
    Soft(soft::H264),
    Hw(Box<dyn HwEncoder>),
}

/// 一路画面的编码器。
pub struct VideoEncoder {
    /// 编码尺寸（偶数）
    pub width: u32,
    pub height: u32,
    src: (u32, u32),
    inner: Inner,
    name: String,
}

impl VideoEncoder {
    /// 按截屏尺寸建编码器。返回编码器和沿途没用上的原因（日志用）。
    pub fn new(src_w: u32, src_h: u32, fps: u32, kbps: u32, req: &Request) -> anyhow::Result<(Self, Vec<String>)> {
        let fps = fps.clamp(1, 60);
        let (w, h) = (src_w & !1, src_h & !1);
        anyhow::ensure!(w >= 16 && h >= 16, "画面太小 {src_w}x{src_h}");
        let mut notes = Vec::new();
        if req.backend != Some(Backend::Software) {
            let mut tries: Vec<(Backend, &str)> = vec![];
            if req.allows(Backend::Nvenc) {
                tries.push((Backend::Nvenc, "NVENC"));
            }
            if req.allows(Backend::Vaapi) {
                tries.push((Backend::Vaapi, "VA-API"));
            }
            for (b, label) in tries {
                let made: anyhow::Result<Box<dyn HwEncoder>> = match b {
                    Backend::Nvenc => nvenc::Nvenc::new(w, h, fps, kbps).map(|e| Box::new(e) as _),
                    _ => vaapi::Vaapi::new(w, h, fps, kbps).map(|e| Box::new(e) as _),
                };
                match made.and_then(|mut e| probe(&mut *e, w, h).map(|_| e)) {
                    Ok(e) => {
                        let name = e.name();
                        return Ok((Self { width: w, height: h, src: (src_w, src_h), inner: Inner::Hw(e), name }, notes));
                    }
                    Err(e) => notes.push(format!("{label}：{e}")),
                }
            }
            if req.backend.is_some() {
                anyhow::bail!("指定的硬件编码器不可用：{}", notes.join("；"));
            }
        }
        let enc = soft::H264::new(src_w, src_h, fps, kbps)?;
        Ok((Self { width: enc.width, height: enc.height, src: (src_w, src_h), inner: Inner::Soft(enc), name: "openh264（软件编码）".into() }, notes))
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_hardware(&self) -> bool {
        matches!(self.inner, Inner::Hw(_))
    }

    pub fn source_size(&self) -> (u32, u32) {
        self.src
    }

    /// 编一帧 BGRA（尺寸是建编码器时的截屏尺寸）。
    pub fn encode(&mut self, bgra: &[u8], keyframe: bool, pts_us: u64) -> anyhow::Result<Vec<EncodedFrame>> {
        let (sw, sh) = self.src;
        anyhow::ensure!(bgra.len() >= sw as usize * sh as usize * 4, "画面数据不完整");
        match &mut self.inner {
            Inner::Soft(e) => {
                if keyframe {
                    e.force_keyframe();
                }
                Ok(e.encode(bgra, sw, sh)?.map(|(data, keyframe)| vec![EncodedFrame { data, keyframe }]).unwrap_or_default())
            }
            Inner::Hw(e) => {
                let mut v = e.encode(bgra, sw, keyframe, pts_us)?;
                for f in &mut v {
                    strip_aud(&mut f.data);
                    f.keyframe = f.keyframe && has_parameter_sets(&f.data);
                }
                v.retain(|f| !f.data.is_empty());
                Ok(v)
            }
        }
    }

    /// 改码率；返回 false 时调用方重建编码器。
    pub fn set_bitrate(&mut self, kbps: u32) -> bool {
        match &mut self.inner {
            Inner::Soft(_) => false,
            Inner::Hw(e) => e.set_bitrate(kbps),
        }
    }
}

/// 建好后试编一帧灰色关键帧，确认真能出带 SPS/PPS 的码流。
fn probe(e: &mut dyn HwEncoder, w: u32, h: u32) -> anyhow::Result<()> {
    let buf = vec![0x80u8; w as usize * h as usize * 4];
    let mut got = Vec::new();
    for i in 0..3 {
        for f in e.encode(&buf, w, true, i * 16_666)? {
            got.extend_from_slice(&f.data);
        }
        if has_parameter_sets(&got) {
            return Ok(());
        }
    }
    anyhow::bail!("试编码没有输出参数集（NAL 类型 {:?}）", nal_types(&got))
}

/// 自检用：能用的硬件编码器
pub fn hardware_requests() -> Vec<(String, Request)> {
    vec![("NVENC".into(), Request { backend: Some(Backend::Nvenc) }), ("VA-API".into(), Request { backend: Some(Backend::Vaapi) })]
}

// ---------------- Annex B ----------------

/// 每个 NAL 单元：(起始码开始位置, 头字节位置, 结束位置)。
pub fn nal_units(data: &[u8]) -> Vec<(usize, usize, usize)> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            let begin = if i > 0 && data[i - 1] == 0 { i - 1 } else { i };
            starts.push((begin, i + 3));
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut v = Vec::with_capacity(starts.len());
    for (k, &(begin, head)) in starts.iter().enumerate() {
        let end = starts.get(k + 1).map(|s| s.0).unwrap_or(data.len());
        if head < end {
            v.push((begin, head, end));
        }
    }
    v
}

pub fn nal_types(data: &[u8]) -> Vec<u8> {
    nal_units(data).iter().map(|&(_, h, _)| data[h] & 0x1f).collect()
}

/// 去掉 AUD（类型 9）：安卓有的解码器要求参数集打头。
pub fn strip_aud(data: &mut Vec<u8>) {
    let units = nal_units(data);
    if !units.iter().any(|&(_, h, _)| data[h] & 0x1f == 9) {
        return;
    }
    let mut out = Vec::with_capacity(data.len());
    for (b, h, e) in units {
        if data[h] & 0x1f != 9 {
            out.extend_from_slice(&data[b..e]);
        }
    }
    *data = out;
}

pub fn has_parameter_sets(data: &[u8]) -> bool {
    let t = nal_types(data);
    t.contains(&7) && t.contains(&8)
}

/// 缩放 BGRA（取最近点，AI 截图够用）并编成 JPEG。
pub fn jpeg(bgra: &[u8], width: u32, height: u32, max_w: u32, max_h: u32) -> anyhow::Result<(Vec<u8>, u32, u32)> {
    let mut k = 1.0f64;
    if max_w > 0 && width > max_w {
        k = k.min(max_w as f64 / width as f64);
    }
    if max_h > 0 && height > max_h {
        k = k.min(max_h as f64 / height as f64);
    }
    let (ow, oh) = (((width as f64 * k).round() as u32).max(1), ((height as f64 * k).round() as u32).max(1));
    let mut rgb = vec![0u8; (ow * oh * 3) as usize];
    for y in 0..oh {
        let sy = (((y as f64 + 0.5) / k) as u32).min(height - 1);
        for x in 0..ow {
            let sx = (((x as f64 + 0.5) / k) as u32).min(width - 1);
            let si = ((sy * width + sx) * 4) as usize;
            let di = ((y * ow + x) * 3) as usize;
            rgb[di] = bgra[si + 2];
            rgb[di + 1] = bgra[si + 1];
            rgb[di + 2] = bgra[si];
        }
    }
    let mut out = Vec::new();
    let enc = jpeg_encoder::Encoder::new(&mut out, 80);
    enc.encode(&rgb, ow as u16, oh as u16, jpeg_encoder::ColorType::Rgb)?;
    Ok((out, ow, oh))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annexb() {
        let mut d = vec![0, 0, 0, 1, 0x09, 0x10, 0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 1, 0x65, 4];
        strip_aud(&mut d);
        assert_eq!(nal_types(&d), vec![7, 8, 5]);
        assert!(has_parameter_sets(&d));
    }

    #[test]
    fn software_encoder_outputs_keyframe() {
        let (mut e, _) = VideoEncoder::new(64, 48, 30, 1000, &Request::software()).unwrap();
        let f = vec![0x40u8; 64 * 48 * 4];
        let out = e.encode(&f, true, 0).unwrap();
        assert!(out.iter().any(|x| x.keyframe && has_parameter_sets(&x.data)));
    }
}
