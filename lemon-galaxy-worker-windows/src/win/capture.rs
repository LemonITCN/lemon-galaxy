//! 截屏：优先 DXGI 桌面复制（Windows 8 起，GPU 拷贝，只在画面变化时出帧），失败时用 GDI。
//! 光标不在桌面复制的画面里，统一用 GDI 画进帧（硬件编码时画在显卡纹理上，见 encode/gpu.rs）。
//! 硬件编码器与桌面复制在同一块显卡上时，画面直接在显卡上拷给编码器，不读回内存（`next_gpu`）。
//! 锁屏、UAC 时输入桌面会切到 Winlogon，采集线程跟着切过去（需要以 SYSTEM 身份运行）。

use galaxy_proto::peer::DisplayInfo;
use windows::core::{Interface, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, EnumDisplayMonitors, GetDC, GetMonitorInfoW, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HBITMAP, HDC,
    HGDIOBJ, HMONITOR, MONITORINFO, MONITORINFOEXW, SRCCOPY,
};
use windows::Win32::System::StationsAndDesktops::{CloseDesktop, GetThreadDesktop, GetUserObjectInformationW, OpenInputDesktop, SetThreadDesktop, DESKTOP_ACCESS_FLAGS, DESKTOP_CONTROL_FLAGS, HDESK, UOI_NAME};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{DrawIconEx, GetCursorInfo, GetIconInfo, CURSORINFO, CURSOR_SHOWING, DI_NORMAL, HCURSOR, ICONINFO};

/// 一块屏幕。坐标是虚拟桌面上的物理像素。
#[derive(Clone, Debug)]
pub struct Monitor {
    pub id: u32,
    pub device: String,
    pub rect: RECT,
    pub primary: bool,
    pub scale: f32,
}

impl Monitor {
    pub fn width(&self) -> u32 {
        (self.rect.right - self.rect.left).max(0) as u32
    }
    pub fn height(&self) -> u32 {
        (self.rect.bottom - self.rect.top).max(0) as u32
    }
    pub fn info(&self) -> DisplayInfo {
        DisplayInfo { id: self.id, name: self.device.trim_start_matches("\\\\.\\").to_string(), width: self.width(), height: self.height(), scale: self.scale, primary: self.primary, is_virtual: false }
    }
}

unsafe extern "system" fn enum_proc(hm: HMONITOR, _hdc: HDC, _rc: *mut RECT, data: LPARAM) -> BOOL {
    let v = &mut *(data.0 as *mut Vec<Monitor>);
    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(hm, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool() {
        let n = mi.szDevice.iter().position(|c| *c == 0).unwrap_or(mi.szDevice.len());
        let device = String::from_utf16_lossy(&mi.szDevice[..n]);
        let id = device.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<String>().chars().rev().collect::<String>().parse::<u32>().unwrap_or(v.len() as u32 + 1);
        let scale = super::compat::monitor_scale(hm);
        v.push(Monitor { id, device, rect: mi.monitorInfo.rcMonitor, primary: mi.monitorInfo.dwFlags & 1 != 0, scale });
    }
    BOOL(1)
}

pub fn monitors() -> Vec<Monitor> {
    let mut v: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(enum_proc), LPARAM(&mut v as *mut Vec<Monitor> as isize));
    }
    v.sort_by_key(|m| (!m.primary, m.id));
    v
}

/// 按 id 找屏幕；0 或找不到时用主屏。
pub fn find_monitor(id: u32) -> Option<Monitor> {
    let all = monitors();
    all.iter().find(|m| m.id == id).cloned().or_else(|| all.iter().find(|m| m.primary).cloned()).or_else(|| all.first().cloned())
}

// ---------------- 输入桌面 ----------------

fn desktop_name(d: HDESK) -> String {
    let mut buf = [0u16; 128];
    let mut need = 0u32;
    unsafe {
        if GetUserObjectInformationW(windows::Win32::Foundation::HANDLE(d.0), UOI_NAME, Some(buf.as_mut_ptr() as _), (buf.len() * 2) as u32, Some(&mut need)).is_err() {
            return String::new();
        }
    }
    let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

/// 当前线程切到输入桌面（锁屏 / UAC 时是 Winlogon）。返回切换后桌面名，失败返回 None。
pub fn attach_input_desktop() -> Option<String> {
    unsafe {
        let d = OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_ACCESS_FLAGS(0x10000000)).ok()?; // GENERIC_ALL
        let name = desktop_name(d);
        let cur = GetThreadDesktop(GetCurrentThreadId()).ok();
        if cur.map(|c| desktop_name(c) == name).unwrap_or(false) {
            let _ = CloseDesktop(d);
            return Some(name);
        }
        if SetThreadDesktop(d).is_err() {
            let _ = CloseDesktop(d);
            return None;
        }
        // 句柄留给线程用，不关闭
        Some(name)
    }
}

/// 输入桌面是否已经变了（与线程当前桌面不同）。
pub fn input_desktop_changed() -> bool {
    unsafe {
        let Ok(d) = OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_ACCESS_FLAGS(0x0001)) else { return false };
        let name = desktop_name(d);
        let _ = CloseDesktop(d);
        let cur = GetThreadDesktop(GetCurrentThreadId()).ok().map(desktop_name).unwrap_or_default();
        !name.is_empty() && name != cur
    }
}

// ---------------- 帧 ----------------

/// 一帧 BGRA，行距 = width * 4。
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

pub trait Capturer {
    /// 等下一帧，最多 `timeout_ms`。Ok(true) 表示 `frame` 已更新，Ok(false) 表示画面没变。
    fn next(&mut self, frame: &mut Frame, timeout_ms: u32) -> anyhow::Result<bool>;
    /// 显卡上截屏：新画面直接拷进 `dst`（`device()` 上、`size()` 大小的 BGRA 纹理），不读回内存。
    fn next_gpu(&mut self, _dst: &ID3D11Texture2D, _timeout_ms: u32) -> anyhow::Result<bool> {
        anyhow::bail!("这种截屏方式不能直接输出到显卡")
    }
    /// 截屏用的 D3D11 设备（桌面复制才有）
    fn device(&self) -> Option<&ID3D11Device> {
        None
    }
    /// 画面尺寸
    fn size(&self) -> (u32, u32);
    fn kind(&self) -> &'static str;
}

// ---------------- DXGI ----------------

pub struct DxgiCapturer {
    output: IDXGIOutput1,
    /// 桌面复制；失效（ACCESS_LOST：切分辨率、增删显示器、全屏独占、锁屏等）后为 None，下次取帧时重建
    dup: Option<IDXGIOutputDuplication>,
    ctx: ID3D11DeviceContext,
    device: ID3D11Device,
    staging: Option<ID3D11Texture2D>,
    width: u32,
    height: u32,
    holding: bool,
    /// 复制失效的时刻；一直重建不成（输出没了）超过几秒才报错，让调用方重新找屏幕
    lost_since: Option<std::time::Instant>,
    /// 复制建好后还没拿到真画面的起始时刻：画面一直不动时超过 1 秒就用 GDI 补一帧
    fresh_since: Option<std::time::Instant>,
    mon: Monitor,
}

impl DxgiCapturer {
    /// `prefer`：已有的设备（硬件编码器的），在同一块显卡上就直接用它，画面可以零拷贝交给编码器。
    pub fn new(mon: &Monitor, prefer: Option<&ID3D11Device>) -> anyhow::Result<Self> {
        unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
            let mut i = 0;
            loop {
                let adapter: IDXGIAdapter1 = factory.EnumAdapters1(i).map_err(|_| anyhow::anyhow!("没有找到这块屏幕对应的显卡输出"))?;
                i += 1;
                let mut j = 0;
                while let Ok(output) = adapter.EnumOutputs(j) {
                    j += 1;
                    let desc = output.GetDesc()?;
                    let n = desc.DeviceName.iter().position(|c| *c == 0).unwrap_or(desc.DeviceName.len());
                    if String::from_utf16_lossy(&desc.DeviceName[..n]) != mon.device {
                        continue;
                    }
                    let luid = adapter.GetDesc1()?.AdapterLuid;
                    let reuse = prefer.filter(|d| super::encode::gpu::device_luid(d).map(|l| l.LowPart == luid.LowPart && l.HighPart == luid.HighPart).unwrap_or(false));
                    let (device, ctx) = match reuse {
                        Some(d) => (d.clone(), d.GetImmediateContext()?),
                        None => super::encode::gpu::create_device(&adapter)?,
                    };
                    let output: IDXGIOutput1 = output.cast()?;
                    // 刚切过模式时第一次复制常常马上失效，这里多试几次
                    let mut last = None;
                    for k in 0..4 {
                        match output.DuplicateOutput(&device) {
                            Ok(dup) => {
                                let dd = dup.GetDesc();
                                return Ok(Self {
                                    output,
                                    dup: Some(dup),
                                    ctx,
                                    device,
                                    staging: None,
                                    width: dd.ModeDesc.Width,
                                    height: dd.ModeDesc.Height,
                                    holding: false,
                                    lost_since: None,
                                    fresh_since: Some(std::time::Instant::now()),
                                    mon: mon.clone(),
                                });
                            }
                            Err(e) => {
                                last = Some(e);
                                std::thread::sleep(std::time::Duration::from_millis(30 * (k + 1)));
                            }
                        }
                    }
                    return Err(last.map(anyhow::Error::from).unwrap_or_else(|| anyhow::anyhow!("桌面复制失败")));
                }
            }
        }
    }

    /// 重建桌面复制（尺寸可能变了，暂存纹理跟着重建）。
    fn reduplicate(&mut self) -> bool {
        unsafe {
            match self.output.DuplicateOutput(&self.device) {
                Ok(dup) => {
                    let dd = dup.GetDesc();
                    if (dd.ModeDesc.Width, dd.ModeDesc.Height) != (self.width, self.height) {
                        self.staging = None;
                    }
                    self.width = dd.ModeDesc.Width;
                    self.height = dd.ModeDesc.Height;
                    self.dup = Some(dup);
                    self.lost_since = None;
                    self.fresh_since = Some(std::time::Instant::now());
                    true
                }
                Err(_) => false,
            }
        }
    }

    /// 取下一帧。Ok(None)：没有新帧（超时、复制刚失效正在重建）。
    fn acquire(&mut self, timeout_ms: u32) -> anyhow::Result<Option<(DXGI_OUTDUPL_FRAME_INFO, Option<IDXGIResource>)>> {
        unsafe {
            if self.holding {
                if let Some(d) = &self.dup {
                    let _ = d.ReleaseFrame();
                }
                self.holding = false;
            }
            if self.dup.is_none() && !self.reduplicate() {
                if self.lost_since.map(|t| t.elapsed() > std::time::Duration::from_secs(3)).unwrap_or(false) {
                    anyhow::bail!("access_lost");
                }
                std::thread::sleep(std::time::Duration::from_millis(timeout_ms.min(50) as u64));
                return Ok(None);
            }
            let dup = self.dup.as_ref().unwrap();
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut res: Option<IDXGIResource> = None;
            match dup.AcquireNextFrame(timeout_ms, &mut info, &mut res) {
                Ok(()) => {
                    self.holding = true;
                    Ok(Some((info, res)))
                }
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => Ok(None),
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                    // 失效了：放掉旧的，立刻试着重建一次（切模式过程中可能还不行，下次再试）
                    self.dup = None;
                    self.lost_since.get_or_insert_with(std::time::Instant::now);
                    self.reduplicate();
                    Ok(None)
                }
                Err(e) => Err(e.into()),
            }
        }
    }

    /// 刚建复制还没出真画面、已经等了 1 秒：用 GDI 截一帧顶上（尺寸与复制一致时）。
    fn gdi_fill(&mut self) -> Option<Frame> {
        if !self.fresh_since.map(|t| t.elapsed() > std::time::Duration::from_secs(1)).unwrap_or(false) {
            return None;
        }
        self.fresh_since = None;
        let mut g = GdiCapturer::new(&self.mon, 60);
        g.interval_ms = 0;
        let mut f = Frame { width: 0, height: 0, data: vec![] };
        g.next(&mut f, 0).ok()?;
        ((f.width, f.height) == (self.width, self.height)).then_some(f)
    }

    fn ensure_staging(&mut self) -> anyhow::Result<ID3D11Texture2D> {
        if let Some(s) = &self.staging {
            return Ok(s.clone());
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: self.width,
            Height: self.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut t: Option<ID3D11Texture2D> = None;
        unsafe {
            self.device.CreateTexture2D(&desc, None, Some(&mut t))?;
        }
        let t = t.ok_or_else(|| anyhow::anyhow!("无法创建暂存纹理"))?;
        self.staging = Some(t.clone());
        Ok(t)
    }
}

impl Capturer for DxgiCapturer {
    fn next(&mut self, frame: &mut Frame, timeout_ms: u32) -> anyhow::Result<bool> {
        let got = self.acquire(timeout_ms)?;
        // LastPresentTime 为 0：画面没变（只有光标动）；刚建复制时的第一帧也是这样，那时表面还是全黑的，不能用
        let Some((_, res)) = got.filter(|(info, _)| info.LastPresentTime != 0) else {
            if let Some(f) = self.gdi_fill() {
                *frame = f;
                return Ok(true);
            }
            return Ok(false);
        };
        self.fresh_since = None;
        unsafe {
            let Some(res) = res else { return Ok(false) };
            let tex: ID3D11Texture2D = res.cast()?;
            let staging = self.ensure_staging()?;
            self.ctx.CopyResource(&staging, &tex);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let (w, h) = (self.width as usize, self.height as usize);
            frame.width = self.width;
            frame.height = self.height;
            frame.data.resize(w * h * 4, 0);
            let src = mapped.pData as *const u8;
            let pitch = mapped.RowPitch as usize;
            for y in 0..h {
                std::ptr::copy_nonoverlapping(src.add(y * pitch), frame.data.as_mut_ptr().add(y * w * 4), w * 4);
            }
            self.ctx.Unmap(&staging, 0);
            Ok(true)
        }
    }

    fn next_gpu(&mut self, dst: &ID3D11Texture2D, timeout_ms: u32) -> anyhow::Result<bool> {
        let got = self.acquire(timeout_ms)?;
        unsafe {
            // 复制重建后尺寸变了：调用方会按新尺寸重建目标纹理
            let mut d = D3D11_TEXTURE2D_DESC::default();
            dst.GetDesc(&mut d);
            if (d.Width, d.Height) != (self.width, self.height) {
                return Ok(false);
            }
            let Some((_, res)) = got.filter(|(info, _)| info.LastPresentTime != 0) else {
                if let Some(f) = self.gdi_fill() {
                    self.ctx.UpdateSubresource(dst, 0, None, f.data.as_ptr() as _, f.width * 4, 0);
                    return Ok(true);
                }
                return Ok(false);
            };
            self.fresh_since = None;
            let Some(res) = res else { return Ok(false) };
            let tex: ID3D11Texture2D = res.cast()?;
            self.ctx.CopyResource(dst, &tex);
            Ok(true)
        }
    }

    fn device(&self) -> Option<&ID3D11Device> {
        Some(&self.device)
    }

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn kind(&self) -> &'static str {
        "dxgi"
    }
}

impl Drop for DxgiCapturer {
    fn drop(&mut self) {
        if self.holding {
            if let Some(d) = &self.dup {
                unsafe {
                    let _ = d.ReleaseFrame();
                }
            }
        }
    }
}

// ---------------- GDI ----------------

struct Dib {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    width: u32,
    height: u32,
}

impl Dib {
    fn new(width: u32, height: u32) -> anyhow::Result<Self> {
        unsafe {
            let screen = GetDC(None);
            let dc = CreateCompatibleDC(Some(screen));
            ReleaseDC(None, screen);
            let mut bi = BITMAPINFO::default();
            bi.bmiHeader = BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: width as i32, biHeight: -(height as i32), biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() };
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(Some(dc), &bi, DIB_RGB_COLORS, &mut bits, None, 0)?;
            let old = SelectObject(dc, bmp.into());
            Ok(Self { dc, bmp, old, bits: bits as *mut u8, width, height })
        }
    }

    fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.bits, (self.width * self.height * 4) as usize) }
    }

    fn bytes_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.width * self.height * 4) as usize) }
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bmp.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

pub struct GdiCapturer {
    mon: Monitor,
    dib: Option<Dib>,
    interval_ms: u32,
}

impl GdiCapturer {
    pub fn new(mon: &Monitor, fps: u32) -> Self {
        Self { mon: mon.clone(), dib: None, interval_ms: 1000 / fps.clamp(1, 60) }
    }
}

impl Capturer for GdiCapturer {
    fn next(&mut self, frame: &mut Frame, timeout_ms: u32) -> anyhow::Result<bool> {
        // GDI 不知道画面有没有变：按帧率节奏抓
        std::thread::sleep(std::time::Duration::from_millis(self.interval_ms.min(timeout_ms) as u64));
        let (w, h) = (self.mon.width(), self.mon.height());
        if self.dib.as_ref().map(|d| d.width != w || d.height != h).unwrap_or(true) {
            self.dib = Some(Dib::new(w, h)?);
        }
        let dib = self.dib.as_ref().unwrap();
        unsafe {
            let screen = GetDC(None);
            // CAPTUREBLT 把分层窗口也截进来；老系统的隐私屏开着时不能带（黑窗就是分层窗口，见 privacy.rs）
            let rop = if super::privacy::legacy_active() { SRCCOPY } else { SRCCOPY | CAPTUREBLT };
            let ok = BitBlt(dib.dc, 0, 0, w as i32, h as i32, Some(screen), self.mon.rect.left, self.mon.rect.top, rop);
            ReleaseDC(None, screen);
            ok?;
        }
        frame.width = w;
        frame.height = h;
        frame.data.clear();
        frame.data.extend_from_slice(dib.bytes());
        // GDI 的 alpha 是 0；显卡视频处理器会按 alpha 混合（全变黑），统一填成不透明
        for px in frame.data.chunks_exact_mut(4) {
            px[3] = 255;
        }
        Ok(true)
    }

    fn size(&self) -> (u32, u32) {
        (self.mon.width(), self.mon.height())
    }

    fn kind(&self) -> &'static str {
        "gdi"
    }
}

/// 打开截屏。`prefer` 见 `DxgiCapturer::new`。
pub fn open_capturer(mon: &Monitor, fps: u32, prefer: Option<&ID3D11Device>) -> Box<dyn Capturer> {
    match DxgiCapturer::new(mon, prefer) {
        Ok(c) => Box::new(c),
        Err(e) => {
            tracing::info!("桌面复制不可用（{e}），改用 GDI 截屏");
            Box::new(GdiCapturer::new(mon, fps))
        }
    }
}

// ---------------- 光标 ----------------

/// 光标贴片边长（像素）：先把光标底下这一块画面拿出来，用 GDI 画上光标再放回去。
pub const CURSOR_PATCH: i32 = 64;

/// 当前光标在某块屏幕画面上的位置。
pub struct CursorPatch {
    /// 贴片左上角（画面内坐标，可能在画面外）
    pub x: i32,
    pub y: i32,
    cursor: HCURSOR,
}

impl CursorPatch {
    /// 光标可见且落在这块屏幕（`fw`×`fh` 的画面）上时返回贴片位置。
    pub fn current(mon: &Monitor, fw: u32, fh: u32) -> Option<Self> {
        unsafe {
            let mut ci = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
            if GetCursorInfo(&mut ci).is_err() || ci.flags.0 & CURSOR_SHOWING.0 == 0 || ci.hCursor.is_invalid() {
                return None;
            }
            let mut ii = ICONINFO::default();
            if GetIconInfo(ci.hCursor.into(), &mut ii).is_err() {
                return None;
            }
            let (hx, hy) = (ii.xHotspot as i32, ii.yHotspot as i32);
            if !ii.hbmMask.is_invalid() {
                let _ = DeleteObject(ii.hbmMask.into());
            }
            if !ii.hbmColor.is_invalid() {
                let _ = DeleteObject(ii.hbmColor.into());
            }
            let p: POINT = ci.ptScreenPos;
            let (x, y) = (p.x - hx - mon.rect.left, p.y - hy - mon.rect.top);
            let (fw, fh) = (fw as i32, fh as i32);
            if x >= fw || y >= fh || x + CURSOR_PATCH <= 0 || y + CURSOR_PATCH <= 0 {
                return None;
            }
            Some(Self { x, y, cursor: ci.hCursor })
        }
    }

    /// 贴片与画面相交的部分：(画面内 x0, y0, x1, y1)。
    pub fn clip(&self, fw: u32, fh: u32) -> (i32, i32, i32, i32) {
        (self.x.max(0), self.y.max(0), (self.x + CURSOR_PATCH).min(fw as i32), (self.y + CURSOR_PATCH).min(fh as i32))
    }

    /// 在贴片（`CURSOR_PATCH`² 的 BGRA，事先填好光标底下的画面）上画光标。
    pub fn draw(&self, pixels: &mut [u8]) -> bool {
        let n = (CURSOR_PATCH * CURSOR_PATCH * 4) as usize;
        if pixels.len() < n {
            return false;
        }
        let Ok(mut dib) = Dib::new(CURSOR_PATCH as u32, CURSOR_PATCH as u32) else { return false };
        dib.bytes_mut().copy_from_slice(&pixels[..n]);
        let ok = unsafe { DrawIconEx(dib.dc, 0, 0, self.cursor.into(), 0, 0, 0, None, DI_NORMAL).is_ok() };
        if ok {
            pixels[..n].copy_from_slice(dib.bytes());
        }
        ok
    }
}

/// 把当前光标画进帧里。返回光标是否可见。
pub fn draw_cursor(frame: &mut Frame, mon: &Monitor) -> bool {
    let Some(p) = CursorPatch::current(mon, frame.width, frame.height) else { return false };
    let (x0, y0, x1, y1) = p.clip(frame.width, frame.height);
    let s = CURSOR_PATCH as usize;
    let fw = frame.width as usize;
    let mut patch = vec![0u8; s * s * 4];
    let rows = |patch: &mut [u8], frame: &mut Frame, back: bool| {
        for fy in y0..y1 {
            let py = (fy - p.y) as usize;
            let px = (x0 - p.x) as usize;
            let w = (x1 - x0) as usize * 4;
            let fi = (fy as usize * fw + x0 as usize) * 4;
            let pi = (py * s + px) * 4;
            if back {
                frame.data[fi..fi + w].copy_from_slice(&patch[pi..pi + w]);
            } else {
                patch[pi..pi + w].copy_from_slice(&frame.data[fi..fi + w]);
            }
        }
    };
    rows(&mut patch, frame, false);
    if !p.draw(&mut patch) {
        return false;
    }
    rows(&mut patch, frame, true);
    true
}

/// 截一张图（AI 截屏用）：GDI 抓整屏并画上光标。
pub fn grab(mon: &Monitor) -> anyhow::Result<Frame> {
    let mut c = GdiCapturer::new(mon, 60);
    c.interval_ms = 0;
    let mut f = Frame { width: 0, height: 0, data: vec![] };
    c.next(&mut f, 0)?;
    draw_cursor(&mut f, mon);
    Ok(f)
}

#[allow(dead_code)]
fn _unused(_: HWND, _: PCWSTR) {}
