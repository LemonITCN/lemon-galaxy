//! 显卡上的画面准备（硬件编码用）：截屏纹理 → 画光标 → D3D11 视频处理器转 NV12。
//! 颜色按 BT.601 有限范围转换、码流里不写色彩描述，与 openh264 软件编码的输出一致（各主控端按 BT.601 显示）。

use super::super::capture::{CursorPatch, Frame, Monitor, CURSOR_PATCH};
use std::mem::ManuallyDrop;
use windows::core::Interface;
use windows::Win32::Foundation::{HMODULE, LUID, RECT};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, ID3D11Texture2D, ID3D11VideoContext, ID3D11VideoContext1, ID3D11VideoDevice, ID3D11VideoProcessor, ID3D11VideoProcessorInputView,
    ID3D11VideoProcessorOutputView, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_BIND_VIDEO_ENCODER, D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
    D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, D3D11_VIDEO_PROCESSOR_COLOR_SPACE,
    D3D11_VIDEO_PROCESSOR_CONTENT_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_STREAM, D3D11_VIDEO_USAGE_OPTIMAL_SPEED, D3D11_VPIV_DIMENSION_TEXTURE2D,
    D3D11_VPOV_DIMENSION_TEXTURE2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709, DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P601, DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter1, IDXGIDevice, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};

pub const VENDOR_NVIDIA: u32 = 0x10DE;
pub const VENDOR_INTEL: u32 = 0x8086;
pub const VENDOR_AMD: u32 = 0x1002;

/// 一块显卡。
#[derive(Clone)]
pub struct Adapter {
    pub adapter: IDXGIAdapter1,
    pub name: String,
    pub vendor: u32,
    pub luid: LUID,
}

impl Adapter {
    pub fn same(&self, other: &LUID) -> bool {
        self.luid.LowPart == other.LowPart && self.luid.HighPart == other.HighPart
    }
}

fn describe(a: IDXGIAdapter1) -> Option<Adapter> {
    let d = unsafe { a.GetDesc1() }.ok()?;
    if d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
        return None;
    }
    let n = d.Description.iter().position(|c| *c == 0).unwrap_or(d.Description.len());
    Some(Adapter { adapter: a, name: String::from_utf16_lossy(&d.Description[..n]).trim().to_string(), vendor: d.VendorId, luid: d.AdapterLuid })
}

/// 本机的硬件显卡（不含微软基本渲染驱动这类软件适配器）。
pub fn adapters() -> Vec<Adapter> {
    let mut v = Vec::new();
    let mut ids = Vec::new();
    unsafe {
        let Ok(f) = CreateDXGIFactory1::<IDXGIFactory1>() else { return v };
        let mut i = 0;
        while let Ok(a) = f.EnumAdapters1(i) {
            i += 1;
            let Ok(d) = a.GetDesc1() else { continue };
            // 间接显示驱动（虚拟显示器）的适配器报的是渲染它的那块显卡的型号：同一型号只留第一个（真显卡排在前面）
            let id = (d.VendorId, d.DeviceId, d.SubSysId, d.Revision);
            if ids.contains(&id) {
                continue;
            }
            if let Some(a) = describe(a) {
                ids.push(id);
                v.push(a);
            }
        }
    }
    v
}

/// 设备所在的显卡。
pub fn device_adapter(device: &ID3D11Device) -> Option<Adapter> {
    let dxgi: IDXGIDevice = device.cast().ok()?;
    let a = unsafe { dxgi.GetAdapter() }.ok()?;
    describe(a.cast().ok()?)
}

pub fn device_luid(device: &ID3D11Device) -> Option<LUID> {
    let dxgi: IDXGIDevice = device.cast().ok()?;
    let a = unsafe { dxgi.GetAdapter() }.ok()?;
    unsafe { a.GetDesc() }.ok().map(|d| d.AdapterLuid)
}

/// 在指定显卡上建 D3D11 设备：带视频支持（视频处理器、Media Foundation 编码器要用），驱动不支持时退回普通设备。
/// 设备开多线程保护（Media Foundation 编码器会在自己的线程里用它）。
pub fn create_device(adapter: &IDXGIAdapter1) -> anyhow::Result<(ID3D11Device, ID3D11DeviceContext)> {
    let mut last = None;
    // 优先 11.1（有的硬件编码器要），老系统不认 11.1 时用默认级别
    let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_10_0];
    let video = D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT;
    for (flags, lv) in [(video, Some(&levels[..])), (video, None), (D3D11_CREATE_DEVICE_BGRA_SUPPORT, None)] {
        let mut device: Option<ID3D11Device> = None;
        let mut ctx: Option<ID3D11DeviceContext> = None;
        match unsafe { D3D11CreateDevice(adapter, D3D_DRIVER_TYPE_UNKNOWN, HMODULE::default(), flags, lv, D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut ctx)) } {
            Ok(()) => {
                if let (Some(d), Some(c)) = (device, ctx) {
                    if let Ok(mt) = d.cast::<ID3D11Multithread>() {
                        unsafe {
                            let _ = mt.SetMultithreadProtected(true);
                        }
                    }
                    return Ok((d, c));
                }
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.map(anyhow::Error::from).unwrap_or_else(|| anyhow::anyhow!("D3D11 设备创建失败")))
}

fn texture(device: &ID3D11Device, w: u32, h: u32, format: DXGI_FORMAT, usage: windows::Win32::Graphics::Direct3D11::D3D11_USAGE, bind: u32, cpu: u32) -> anyhow::Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: usage,
        BindFlags: bind,
        CPUAccessFlags: cpu,
        MiscFlags: 0,
    };
    let mut t: Option<ID3D11Texture2D> = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut t))? };
    t.ok_or_else(|| anyhow::anyhow!("纹理创建失败"))
}

/// 显卡上的画面：截屏纹理（不含光标）、画上光标的副本、NV12 输出（几张轮流用，编码器异步读时不被下一帧覆盖）。
pub struct Gpu {
    pub device: ID3D11Device,
    ctx: ID3D11DeviceContext,
    vctx: ID3D11VideoContext,
    vp: ID3D11VideoProcessor,
    /// 截屏画面（截屏尺寸，BGRA）。零拷贝时桌面复制直接拷进来，否则从内存传上来
    pub desktop: ID3D11Texture2D,
    composed: ID3D11Texture2D,
    input: ID3D11VideoProcessorInputView,
    outputs: Vec<(ID3D11Texture2D, ID3D11VideoProcessorOutputView)>,
    next: usize,
    patch: ID3D11Texture2D,
    pub src_w: u32,
    pub src_h: u32,
    /// 编码尺寸（偶数，多出的一行 / 一列裁掉）
    pub width: u32,
    pub height: u32,
}

impl Gpu {
    pub fn new(device: &ID3D11Device, src_w: u32, src_h: u32, ring: usize) -> anyhow::Result<Self> {
        let (w, h) = (src_w & !1, src_h & !1);
        anyhow::ensure!(w >= 16 && h >= 16, "画面太小 {src_w}x{src_h}");
        unsafe {
            let ctx = device.GetImmediateContext()?;
            if let Ok(mt) = device.cast::<ID3D11Multithread>() {
                let _ = mt.SetMultithreadProtected(true);
            }
            let vdev: ID3D11VideoDevice = device.cast().map_err(|_| anyhow::anyhow!("显卡驱动不支持视频处理"))?;
            let vctx: ID3D11VideoContext = ctx.cast()?;
            let rate = DXGI_RATIONAL { Numerator: 60, Denominator: 1 };
            let content = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputFrameRate: rate,
                InputWidth: src_w,
                InputHeight: src_h,
                OutputFrameRate: rate,
                OutputWidth: w,
                OutputHeight: h,
                Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED,
            };
            let venum = vdev.CreateVideoProcessorEnumerator(&content)?;
            let vp = vdev.CreateVideoProcessor(&venum, 0)?;
            // 桌面是全范围 sRGB，输出 BT.601 有限范围
            if let Ok(v1) = vctx.cast::<ID3D11VideoContext1>() {
                v1.VideoProcessorSetStreamColorSpace1(&vp, 0, DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709);
                v1.VideoProcessorSetOutputColorSpace1(&vp, DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P601);
            } else {
                // 位域：Usage:1 RGB_Range:1 YCbCr_Matrix:1 YCbCr_xvYCC:1 Nominal_Range:2；RGB_Range=0 全范围，Matrix=0 BT.601，Nominal_Range=1 为 16–235
                vctx.VideoProcessorSetStreamColorSpace(&vp, 0, &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 0 });
                vctx.VideoProcessorSetOutputColorSpace(&vp, &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 1 << 4 });
            }
            vctx.VideoProcessorSetStreamFrameFormat(&vp, 0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
            vctx.VideoProcessorSetStreamAutoProcessingMode(&vp, 0, false);
            let rect = RECT { left: 0, top: 0, right: w as i32, bottom: h as i32 };
            vctx.VideoProcessorSetStreamSourceRect(&vp, 0, true, Some(&rect));
            vctx.VideoProcessorSetStreamDestRect(&vp, 0, true, Some(&rect));
            vctx.VideoProcessorSetOutputTargetRect(&vp, true, Some(&rect));

            let rgb_bind = (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32;
            let desktop = texture(device, src_w, src_h, DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_USAGE_DEFAULT, rgb_bind, 0)?;
            let composed = texture(device, src_w, src_h, DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_USAGE_DEFAULT, rgb_bind, 0)?;
            let mut idesc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC { FourCC: 0, ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D, ..Default::default() };
            idesc.Anonymous.Texture2D.MipSlice = 0;
            let mut input: Option<ID3D11VideoProcessorInputView> = None;
            vdev.CreateVideoProcessorInputView(&composed, &venum, &idesc, Some(&mut input)).map_err(|e| anyhow::anyhow!("视频处理器不收 BGRA 画面：{e}"))?;
            let input = input.ok_or_else(|| anyhow::anyhow!("视频处理器输入创建失败"))?;
            let mut outputs = Vec::new();
            let odesc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC { ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D, ..Default::default() };
            for _ in 0..ring.max(1) {
                // 编码器输入：有的驱动不认 VIDEO_ENCODER 绑定，退回只做渲染目标
                let t = texture(device, w, h, DXGI_FORMAT_NV12, D3D11_USAGE_DEFAULT, (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_VIDEO_ENCODER.0) as u32, 0)
                    .or_else(|_| texture(device, w, h, DXGI_FORMAT_NV12, D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET.0 as u32, 0))
                    .map_err(|e| anyhow::anyhow!("显卡不支持 NV12 纹理：{e}"))?;
                let mut view: Option<ID3D11VideoProcessorOutputView> = None;
                vdev.CreateVideoProcessorOutputView(&t, &venum, &odesc, Some(&mut view)).map_err(|e| anyhow::anyhow!("视频处理器不能输出 NV12：{e}"))?;
                outputs.push((t, view.ok_or_else(|| anyhow::anyhow!("视频处理器输出创建失败"))?));
            }
            let patch = texture(device, CURSOR_PATCH as u32, CURSOR_PATCH as u32, DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_USAGE_STAGING, 0, D3D11_CPU_ACCESS_READ.0 as u32)?;
            Ok(Self { device: device.clone(), ctx, vctx, vp, desktop, composed, input, outputs, next: 0, patch, src_w, src_h, width: w, height: h })
        }
    }

    /// 一张 NV12 纹理（试编码用）。
    pub fn any_output(&self) -> ID3D11Texture2D {
        self.outputs[0].0.clone()
    }

    /// 把内存里的画面传到截屏纹理（截屏不在这块显卡上时）。
    pub fn upload(&self, frame: &Frame) {
        if frame.width != self.src_w || frame.height != self.src_h || frame.data.len() < (self.src_w * self.src_h * 4) as usize {
            return;
        }
        unsafe {
            self.ctx.UpdateSubresource(&self.desktop, 0, None, frame.data.as_ptr() as _, self.src_w * 4, 0);
        }
    }

    /// 画上光标并转成 NV12，返回这次写入的 NV12 纹理。
    pub fn convert(&mut self, mon: &Monitor, cursor: bool) -> anyhow::Result<ID3D11Texture2D> {
        unsafe {
            self.ctx.CopyResource(&self.composed, &self.desktop);
            if cursor {
                self.draw_cursor(mon);
            }
            let (tex, view) = &self.outputs[self.next];
            self.next = (self.next + 1) % self.outputs.len();
            let stream = D3D11_VIDEO_PROCESSOR_STREAM { Enable: true.into(), pInputSurface: ManuallyDrop::new(Some(self.input.clone())), ..Default::default() };
            let streams = [stream];
            let r = self.vctx.VideoProcessorBlt(&self.vp, view, 0, &streams);
            let [mut stream] = streams;
            ManuallyDrop::drop(&mut stream.pInputSurface);
            r.map_err(|e| anyhow::anyhow!("NV12 转换失败：{e}"))?;
            Ok(tex.clone())
        }
    }

    /// 光标：把光标底下一小块读回内存，用 GDI 画上光标再写回（反色光标也能画对）。
    fn draw_cursor(&self, mon: &Monitor) {
        let Some(p) = CursorPatch::current(mon, self.src_w, self.src_h) else { return };
        let (x0, y0, x1, y1) = p.clip(self.src_w, self.src_h);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let s = CURSOR_PATCH as usize;
        let (dx, dy) = ((x0 - p.x) as u32, (y0 - p.y) as u32);
        let bx = D3D11_BOX { left: x0 as u32, top: y0 as u32, front: 0, right: x1 as u32, bottom: y1 as u32, back: 1 };
        let mut px = vec![0u8; s * s * 4];
        unsafe {
            self.ctx.CopySubresourceRegion(&self.patch, 0, dx, dy, 0, &self.composed, 0, Some(&bx));
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            if self.ctx.Map(&self.patch, 0, D3D11_MAP_READ, 0, Some(&mut m)).is_err() {
                return;
            }
            for y in 0..s {
                std::ptr::copy_nonoverlapping((m.pData as *const u8).add(y * m.RowPitch as usize), px.as_mut_ptr().add(y * s * 4), s * 4);
            }
            self.ctx.Unmap(&self.patch, 0);
            if !p.draw(&mut px) {
                return;
            }
            // GDI 画的光标像素 alpha 可能是 0，视频处理器会按 alpha 混合
            for c in px.chunks_exact_mut(4) {
                c[3] = 255;
            }
            let off = (dy as usize * s + dx as usize) * 4;
            self.ctx.UpdateSubresource(&self.composed, 0, Some(&bx), px.as_ptr().add(off) as _, (s * 4) as u32, 0);
        }
    }
}
