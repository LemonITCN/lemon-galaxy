//! 画面编码：优先硬件 H.264，没有或出错时用 openh264 软件编码。输出 Annex-B，关键帧带 SPS/PPS，不带 AUD。
//!
//! 硬件编码器按顺序试（运行时检测，任何一步失败就试下一个）：
//! 1. NVIDIA NVENC 直连（nvenc.rs）；
//! 2. Media Foundation 硬件编码器（Intel Quick Sync、AMD 等，mft.rs）；
//! 3. openh264（soft.rs）。
//!
//! 硬件编码时画面在显卡上转 NV12（gpu.rs）。编码器与桌面复制在同一块显卡上时零拷贝；
//! 不在同一块（双显卡笔记本核显出图、独显编码，或 GDI 截屏）时从内存传上去。
//! 环境变量 LEMON_GALAXY_VIDEO_ENCODER 可指定：auto（默认）、nvenc、mf、mf:intel / mf:amd / mf:nvidia、software。

pub mod gpu;
mod mft;
// NVENC 的结构体按 64 位布局；NVIDIA 2018 年起不再出 32 位系统驱动，32 位版不带
#[cfg(target_pointer_width = "64")]
mod nvenc;
mod soft;

use super::capture::{draw_cursor, Frame, Monitor};
pub use galaxy_host::pacing::EncoderRate;
use gpu::{Adapter, Gpu, VENDOR_AMD, VENDOR_INTEL, VENDOR_NVIDIA};
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D};

/// 编好的一帧（一个访问单元）。
pub struct EncodedFrame {
    pub data: Vec<u8>,
    pub keyframe: bool,
}

/// 硬件编码器：输入显卡上的 NV12 纹理。
trait HwEncoder: Send {
    fn name(&self) -> String;
    /// 编一帧。异步编码器可能一次出 0 帧或多帧。
    fn encode(&mut self, nv12: &ID3D11Texture2D, keyframe: bool, pts_us: u64) -> anyhow::Result<Vec<EncodedFrame>>;
    /// 运行时改码率与 VBV（不出关键帧）；不支持返回 false（调用方重建编码器）。
    fn set_bitrate(&mut self, rate: EncoderRate) -> bool;
    /// 建好后试编一帧，确认真能出帧（输出丢掉）。
    fn probe(&mut self, _nv12: &ID3D11Texture2D) -> anyhow::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Nvenc,
    Mft,
    Software,
}

/// 用哪种编码器。
#[derive(Clone, Debug, Default)]
pub struct Request {
    /// None 为自动
    pub backend: Option<Backend>,
    /// 只用这家显卡（PCI 厂商号）
    pub vendor: Option<u32>,
}

impl Request {
    pub fn software() -> Self {
        Self { backend: Some(Backend::Software), vendor: None }
    }

    /// 从环境变量 LEMON_GALAXY_VIDEO_ENCODER 读（排障、测试用）。
    pub fn from_env() -> Self {
        std::env::var("LEMON_GALAXY_VIDEO_ENCODER").map(|s| Self::parse(&s)).unwrap_or_default()
    }

    pub fn parse(s: &str) -> Self {
        let s = s.trim().to_ascii_lowercase();
        let (kind, vendor) = s.split_once(':').unwrap_or((s.as_str(), ""));
        let vendor = match vendor {
            "intel" | "qsv" => Some(VENDOR_INTEL),
            "amd" => Some(VENDOR_AMD),
            "nvidia" => Some(VENDOR_NVIDIA),
            _ => u32::from_str_radix(vendor.trim_start_matches("0x"), 16).ok(),
        };
        let backend = match kind {
            "nvenc" | "nvidia" => Some(Backend::Nvenc),
            "mf" | "mft" | "qsv" => Some(Backend::Mft),
            "software" | "soft" | "openh264" | "sw" => Some(Backend::Software),
            _ => None,
        };
        Self { backend, vendor }
    }

    fn allows(&self, b: Backend) -> bool {
        self.backend.map(|x| x == b).unwrap_or(true)
    }
}

/// 建一个 COM 多线程套间（Media Foundation 需要）。已经初始化过也没关系。
pub fn com_init() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    }
}

enum Inner {
    Soft { enc: soft::H264, work: Frame },
    Hw { gpu: Gpu, enc: Box<dyn HwEncoder> },
}

/// 一路画面的编码器。
pub struct VideoEncoder {
    /// 编码尺寸（偶数）
    pub width: u32,
    pub height: u32,
    src: (u32, u32),
    inner: Inner,
    name: String,
    /// 与桌面复制共用设备（零拷贝）
    zero_copy: bool,
}

fn same_device(a: &ID3D11Device, b: Option<&ID3D11Device>) -> bool {
    b.map(|b| a.as_raw() == b.as_raw()).unwrap_or(false)
}

impl VideoEncoder {
    /// 按截屏尺寸建编码器。`capture`：桌面复制用的设备（有的话硬件编码器尽量与它同设备，零拷贝）。
    /// 返回编码器和沿途失败的原因（日志用）。
    pub fn new(src_w: u32, src_h: u32, fps: u32, kbps: u32, capture: Option<&ID3D11Device>, req: &Request) -> anyhow::Result<(Self, Vec<String>)> {
        let fps = fps.clamp(1, 60);
        let mut notes = Vec::new();
        if req.backend != Some(Backend::Software) {
            match Self::hardware(src_w, src_h, fps, kbps, capture, req, &mut notes) {
                Some(e) => return Ok((e, notes)),
                None if req.backend.is_some() => anyhow::bail!("指定的硬件编码器不可用：{}", notes.join("；")),
                None => {}
            }
        }
        let enc = soft::H264::new(src_w, src_h, fps, kbps)?;
        let e = Self {
            width: enc.width,
            height: enc.height,
            src: (src_w, src_h),
            name: "openh264（软件编码）".into(),
            inner: Inner::Soft { enc, work: Frame { width: 0, height: 0, data: vec![] } },
            zero_copy: false,
        };
        Ok((e, notes))
    }

    fn hardware(src_w: u32, src_h: u32, fps: u32, kbps: u32, capture: Option<&ID3D11Device>, req: &Request, notes: &mut Vec<String>) -> Option<Self> {
        let cap_adapter = capture.and_then(gpu::device_adapter);
        let all = gpu::adapters();
        // 候选：(编码器种类, 显卡, 与截屏同设备)。截屏所在显卡排前面（零拷贝），其他显卡从内存传画面
        let mut list: Vec<(Backend, Adapter, bool)> = Vec::new();
        let others = |v: u32| all.iter().filter(|a| a.vendor == v && cap_adapter.as_ref().map(|c| !c.same(&a.luid)).unwrap_or(true)).cloned().collect::<Vec<_>>();
        let vendor_ok = |v: u32| req.vendor.map(|x| x == v).unwrap_or(true);
        if req.allows(Backend::Nvenc) {
            if let Some(c) = cap_adapter.as_ref().filter(|c| c.vendor == VENDOR_NVIDIA) {
                list.push((Backend::Nvenc, c.clone(), true));
            }
            list.extend(others(VENDOR_NVIDIA).into_iter().map(|a| (Backend::Nvenc, a, false)));
        }
        if req.allows(Backend::Mft) && mft::available() {
            // NVIDIA 自己的 MFT 慢（每帧约 29 ms、一次只收一帧），只有明确指定时才用
            let nvidia_ok = req.backend == Some(Backend::Mft) && req.vendor.map(|v| v == VENDOR_NVIDIA).unwrap_or(true);
            let ok = |a: &Adapter| vendor_ok(a.vendor) && (a.vendor != VENDOR_NVIDIA || nvidia_ok);
            if let Some(c) = cap_adapter.as_ref().filter(|c| ok(c)) {
                list.push((Backend::Mft, c.clone(), true));
            }
            for a in &all {
                if ok(a) && cap_adapter.as_ref().map(|c| !c.same(&a.luid)).unwrap_or(true) {
                    list.push((Backend::Mft, a.clone(), false));
                }
            }
        } else if req.allows(Backend::Mft) {
            notes.push("Media Foundation 不可用".into());
        }
        for (kind, adapter, shared) in list {
            let device = if shared {
                capture.cloned()
            } else {
                match gpu::create_device(&adapter.adapter) {
                    Ok((d, _)) => Some(d),
                    Err(e) => {
                        notes.push(format!("{}：建设备失败 {e}", adapter.name));
                        None
                    }
                }
            };
            let Some(device) = device else { continue };
            let label = match kind {
                Backend::Nvenc => "NVENC",
                _ => "Media Foundation",
            };
            // 异步编码器可能还在读上一张，轮流用三张 NV12
            let g = match Gpu::new(&device, src_w, src_h, if kind == Backend::Mft { 3 } else { 1 }) {
                Ok(g) => g,
                Err(e) => {
                    notes.push(format!("{label}（{}）：{e}", adapter.name));
                    continue;
                }
            };
            let enc: anyhow::Result<Box<dyn HwEncoder>> = match kind {
                #[cfg(target_pointer_width = "64")]
                Backend::Nvenc => nvenc::Nvenc::new(&device, g.width, g.height, fps, kbps, &adapter.name).map(|e| Box::new(e) as _),
                #[cfg(not(target_pointer_width = "64"))]
                Backend::Nvenc => Err(anyhow::anyhow!("32 位版不带 NVENC")),
                _ => mft::Mft::new(&device, &adapter, g.width, g.height, fps, kbps, req.backend == Some(Backend::Mft)).map(|e| Box::new(e) as _),
            };
            let enc = enc.and_then(|mut e| e.probe(&g.any_output()).map(|_| e));
            match enc {
                Ok(enc) => {
                    let name = enc.name();
                    return Some(Self { width: g.width, height: g.height, src: (src_w, src_h), inner: Inner::Hw { gpu: g, enc }, name, zero_copy: shared });
                }
                Err(e) => notes.push(format!("{label}（{}）：{e}", adapter.name)),
            }
        }
        None
    }

    pub fn name(&self) -> String {
        if self.zero_copy {
            format!("{}，显卡直通", self.name)
        } else if self.is_hardware() {
            format!("{}，经内存传画面", self.name)
        } else {
            self.name.clone()
        }
    }

    pub fn is_hardware(&self) -> bool {
        matches!(self.inner, Inner::Hw { .. })
    }

    /// 截屏尺寸（建编码器时的）。
    pub fn source_size(&self) -> (u32, u32) {
        self.src
    }

    /// 硬件编码器用的设备（重开截屏时传给它，在同一块显卡上就共用）。
    pub fn device(&self) -> Option<&ID3D11Device> {
        match &self.inner {
            Inner::Hw { gpu, .. } => Some(&gpu.device),
            Inner::Soft { .. } => None,
        }
    }

    /// 截屏设备与编码器同设备时，截屏直接拷进这张纹理（不经内存）。
    pub fn gpu_target(&self, capture: Option<&ID3D11Device>) -> Option<&ID3D11Texture2D> {
        match &self.inner {
            Inner::Hw { gpu, .. } if same_device(&gpu.device, capture) => Some(&gpu.desktop),
            _ => None,
        }
    }

    /// 内存里的新画面（截屏不在编码器的显卡上时）。软件编码直接用 `encode` 传入的帧，这里什么都不做。
    pub fn upload(&mut self, frame: &Frame) {
        if let Inner::Hw { gpu, .. } = &self.inner {
            gpu.upload(frame);
        }
    }

    /// 画上光标（`cursor` 为 false 时不画）并编码当前画面。`frame` 只有软件编码用（硬件编码用 `upload` / 零拷贝进来的画面）。
    pub fn encode(&mut self, frame: &Frame, mon: &Monitor, cursor: bool, keyframe: bool, pts_us: u64) -> anyhow::Result<Vec<EncodedFrame>> {
        match &mut self.inner {
            Inner::Soft { enc, work } => {
                work.width = frame.width;
                work.height = frame.height;
                work.data.clear();
                work.data.extend_from_slice(&frame.data);
                if cursor {
                    draw_cursor(work, mon);
                }
                if keyframe {
                    enc.force_keyframe();
                }
                Ok(match enc.encode(&work.data, work.width, work.height, pts_us)? {
                    Some((mut data, keyframe)) => {
                        strip_aud(&mut data);
                        vec![EncodedFrame { data, keyframe }]
                    }
                    None => vec![],
                })
            }
            Inner::Hw { gpu, enc } => {
                let nv12 = gpu.convert(mon, cursor)?;
                enc.encode(&nv12, keyframe, pts_us)
            }
        }
    }

    /// 编码器按配置帧率分码率、能运行时改码率（NVENC、Media Foundation 的 CBR）：画面帧率低时由 galaxy_host::pacing 放大码率。
    /// openh264 按时间戳分码率，不用放大。
    pub fn scales_bitrate(&self) -> bool {
        matches!(self.inner, Inner::Hw { .. })
    }

    /// 改码率。硬件编码器运行时改，不出关键帧；返回 false 时调用方重建编码器。
    pub fn set_bitrate(&mut self, rate: EncoderRate) -> bool {
        match &mut self.inner {
            Inner::Hw { enc, .. } => enc.set_bitrate(rate),
            Inner::Soft { .. } => false,
        }
    }
}

/// 列出本机显卡与可能的硬件编码器种类（自检用）。
pub fn hardware_requests() -> Vec<(String, Request)> {
    let mut v = Vec::new();
    let all = gpu::adapters();
    for a in &all {
        if a.vendor == VENDOR_NVIDIA {
            v.push((format!("NVENC（{}）", a.name), Request { backend: Some(Backend::Nvenc), vendor: Some(a.vendor) }));
        }
        v.push((format!("Media Foundation（{}）", a.name), Request { backend: Some(Backend::Mft), vendor: Some(a.vendor) }));
    }
    v
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

/// NAL 类型列表（自检用）。
pub fn nal_types(data: &[u8]) -> Vec<u8> {
    nal_units(data).iter().map(|&(_, h, _)| data[h] & 0x1f).collect()
}

/// 去掉 AUD（类型 9）：安卓有的解码器要求参数集打头，前面有 AUD 会解不出来。
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

/// 有没有 SPS / PPS。
pub fn has_parameter_sets(data: &[u8]) -> bool {
    let t = nal_types(data);
    t.contains(&7) && t.contains(&8)
}

/// 缩放 BGRA（双线性的简化：按面积取最近点，AI 截图够用）并编成 JPEG。
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
        let sy = ((y as f64 + 0.5) / k) as u32;
        let sy = sy.min(height - 1);
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
