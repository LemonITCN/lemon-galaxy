//! 截屏：优先 DXGI 桌面复制（Windows 8 起，GPU 拷贝，只在画面变化时出帧），失败时用 GDI。
//! 光标不在桌面复制的画面里，统一用 GDI 画进帧。
//! 锁屏、UAC 时输入桌面会切到 Winlogon，采集线程跟着切过去（需要以 SYSTEM 身份运行）。

use galaxy_proto::peer::DisplayInfo;
use windows::core::{Interface, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, EnumDisplayMonitors, GetDC, GetMonitorInfoW, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HBITMAP, HDC,
    HGDIOBJ, HMONITOR, MONITORINFO, MONITORINFOEXW, SRCCOPY,
};
use windows::Win32::System::StationsAndDesktops::{CloseDesktop, GetThreadDesktop, GetUserObjectInformationW, OpenInputDesktop, SetThreadDesktop, DESKTOP_ACCESS_FLAGS, DESKTOP_CONTROL_FLAGS, HDESK, UOI_NAME};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{DrawIconEx, GetCursorInfo, GetIconInfo, CURSORINFO, CURSOR_SHOWING, DI_NORMAL, ICONINFO};

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
        let (mut dx, mut dy) = (96u32, 96u32);
        let scale = if GetDpiForMonitor(hm, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).is_ok() { dx as f32 / 96.0 } else { 1.0 };
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
    fn kind(&self) -> &'static str;
}

// ---------------- DXGI ----------------

pub struct DxgiCapturer {
    dup: IDXGIOutputDuplication,
    ctx: ID3D11DeviceContext,
    device: ID3D11Device,
    staging: Option<ID3D11Texture2D>,
    width: u32,
    height: u32,
    holding: bool,
}

impl DxgiCapturer {
    pub fn new(mon: &Monitor) -> anyhow::Result<Self> {
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
                    let mut device: Option<ID3D11Device> = None;
                    let mut ctx: Option<ID3D11DeviceContext> = None;
                    D3D11CreateDevice(&adapter, D3D_DRIVER_TYPE_UNKNOWN, windows::Win32::Foundation::HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut ctx))?;
                    let device = device.ok_or_else(|| anyhow::anyhow!("D3D11 设备创建失败"))?;
                    let ctx = ctx.ok_or_else(|| anyhow::anyhow!("D3D11 设备创建失败"))?;
                    let o1: IDXGIOutput1 = output.cast()?;
                    let dup = o1.DuplicateOutput(&device)?;
                    let dd = dup.GetDesc();
                    return Ok(Self { dup, ctx, device, staging: None, width: dd.ModeDesc.Width, height: dd.ModeDesc.Height, holding: false });
                }
            }
        }
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
        unsafe {
            if self.holding {
                let _ = self.dup.ReleaseFrame();
                self.holding = false;
            }
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut res: Option<IDXGIResource> = None;
            match self.dup.AcquireNextFrame(timeout_ms, &mut info, &mut res) {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(false),
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => anyhow::bail!("access_lost"),
                Err(e) => return Err(e.into()),
            }
            self.holding = true;
            // 只有光标移动、画面没变时 LastPresentTime 为 0
            if info.LastPresentTime == 0 && frame.width == self.width && !frame.data.is_empty() {
                return Ok(false);
            }
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

    fn kind(&self) -> &'static str {
        "dxgi"
    }
}

impl Drop for DxgiCapturer {
    fn drop(&mut self) {
        if self.holding {
            unsafe {
                let _ = self.dup.ReleaseFrame();
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
            let ok = BitBlt(dib.dc, 0, 0, w as i32, h as i32, Some(screen), self.mon.rect.left, self.mon.rect.top, SRCCOPY | CAPTUREBLT);
            ReleaseDC(None, screen);
            ok?;
        }
        frame.width = w;
        frame.height = h;
        frame.data.clear();
        frame.data.extend_from_slice(dib.bytes());
        Ok(true)
    }

    fn kind(&self) -> &'static str {
        "gdi"
    }
}

pub fn open_capturer(mon: &Monitor, fps: u32) -> Box<dyn Capturer> {
    match DxgiCapturer::new(mon) {
        Ok(c) => Box::new(c),
        Err(e) => {
            tracing::info!("桌面复制不可用（{e}），改用 GDI 截屏");
            Box::new(GdiCapturer::new(mon, fps))
        }
    }
}

// ---------------- 光标 ----------------

/// 把当前光标画进帧里。返回光标是否可见。
pub fn draw_cursor(frame: &mut Frame, mon: &Monitor) -> bool {
    unsafe {
        let mut ci = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
        if GetCursorInfo(&mut ci).is_err() || ci.flags.0 & CURSOR_SHOWING.0 == 0 || ci.hCursor.is_invalid() {
            return false;
        }
        let mut ii = ICONINFO::default();
        if GetIconInfo(ci.hCursor.into(), &mut ii).is_err() {
            return false;
        }
        let (hx, hy) = (ii.xHotspot as i32, ii.yHotspot as i32);
        if !ii.hbmMask.is_invalid() {
            let _ = DeleteObject(ii.hbmMask.into());
        }
        if !ii.hbmColor.is_invalid() {
            let _ = DeleteObject(ii.hbmColor.into());
        }
        const S: i32 = 64;
        let p: POINT = ci.ptScreenPos;
        let (x0, y0) = (p.x - hx - mon.rect.left, p.y - hy - mon.rect.top);
        let (fw, fh) = (frame.width as i32, frame.height as i32);
        if x0 >= fw || y0 >= fh || x0 + S <= 0 || y0 + S <= 0 {
            return false;
        }
        let Ok(mut dib) = Dib::new(S as u32, S as u32) else { return false };
        // 把光标下面的画面拷进小位图，画上光标，再拷回去
        let copy = |dib: &mut Dib, back: bool, frame: &mut Frame| {
            let d = dib.bytes_mut().as_mut_ptr();
            for yy in 0..S {
                let fy = y0 + yy;
                if fy < 0 || fy >= fh {
                    continue;
                }
                for xx in 0..S {
                    let fx = x0 + xx;
                    if fx < 0 || fx >= fw {
                        continue;
                    }
                    let fi = ((fy * fw + fx) * 4) as usize;
                    let di = ((yy * S + xx) * 4) as usize;
                    if back {
                        frame.data[fi..fi + 4].copy_from_slice(std::slice::from_raw_parts(d.add(di), 4));
                    } else {
                        std::ptr::copy_nonoverlapping(frame.data.as_ptr().add(fi), d.add(di), 4);
                    }
                }
            }
        };
        copy(&mut dib, false, frame);
        let ok = DrawIconEx(dib.dc, 0, 0, ci.hCursor.into(), 0, 0, 0, None, DI_NORMAL).is_ok();
        if ok {
            copy(&mut dib, true, frame);
        }
        ok
    }
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
