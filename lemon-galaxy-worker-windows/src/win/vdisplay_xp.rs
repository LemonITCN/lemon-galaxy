//! XP 虚拟屏（driver/xp 的 XPDM 驱动）：装好后系统里多出几块不接入桌面的显示设备「LemonGalaxy Virtual Display」，
//! 加屏 = 把要的分辨率写进它的显示设备键（小端口据此把这个分辨率排进模式表），再用 ChangeDisplaySettingsEx 接入，
//! 放在现有桌面右边；删屏 = 断开。
//!
//! XP 接入新屏只能写注册表（CDS_UPDATEREGISTRY），代理异常退出、重启后屏还接着：代理启动时先把我们的屏都断开。

use super::capture::monitors;
use galaxy_proto::peer::DisplayInfo;
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{
    ChangeDisplaySettingsExW, EnumDisplayDevicesW, EnumDisplaySettingsExW, CDS_NORESET, CDS_TYPE, CDS_UPDATEREGISTRY, DEVMODEW, DISPLAY_DEVICEW, DISP_CHANGE_SUCCESSFUL, DM_BITSPERPEL, DM_PELSHEIGHT, DM_PELSWIDTH, DM_POSITION,
    ENUM_CURRENT_SETTINGS, ENUM_DISPLAY_SETTINGS_FLAGS, ENUM_DISPLAY_SETTINGS_MODE,
};
use windows::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_SET_VALUE, REG_DWORD};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_XVIRTUALSCREEN};

/// 驱动 INF 里的设备描述，也是 EnumDisplayDevices 的 DeviceString
pub const DESC: &str = "LemonGalaxy Virtual Display";
pub const HARDWARE_ID: &str = "Root\\LemonGalaxyDisplayXP";
const DISPLAY_DEVICE_ATTACHED_TO_DESKTOP: u32 = 0x1;
/// 小端口模式表里的常见分辨率（要的分辨率接不上时退到其中不超过它的最大一个）
const STD_MODES: [(u32, u32); 15] = [(2560, 1600), (2560, 1440), (1920, 1200), (1920, 1080), (1680, 1050), (1600, 1200), (1600, 900), (1440, 900), (1366, 768), (1280, 1024), (1280, 800), (1280, 720), (1152, 864), (1024, 768), (800, 600)];

#[derive(Clone, Debug)]
pub struct Dev {
    /// GDI 名 \\.\DISPLAYn
    pub name: String,
    /// 显示设备键（\Registry\Machine\System\...）
    key: String,
    pub attached: bool,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn utf16(b: &[u16]) -> String {
    let n = b.iter().position(|c| *c == 0).unwrap_or(b.len());
    String::from_utf16_lossy(&b[..n])
}

/// 我们的显示设备（按系统列出的顺序）。
pub fn devices() -> Vec<Dev> {
    let mut v = Vec::new();
    for i in 0.. {
        let mut dd = DISPLAY_DEVICEW { cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
        if !unsafe { EnumDisplayDevicesW(PCWSTR::null(), i, &mut dd, 0) }.as_bool() {
            break;
        }
        if utf16(&dd.DeviceString) == DESC || utf16(&dd.DeviceID).eq_ignore_ascii_case(HARDWARE_ID) {
            v.push(Dev { name: utf16(&dd.DeviceName), key: utf16(&dd.DeviceKey), attached: dd.StateFlags.0 & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP != 0 });
        }
    }
    v
}

/// 把要的分辨率写进显示设备键（小端口每次报模式表时读）
fn write_mode(key: &str, w: u32, h: u32) -> Result<(), String> {
    let sub = key.strip_prefix("\\Registry\\Machine\\").or_else(|| key.strip_prefix("\\REGISTRY\\MACHINE\\")).ok_or_else(|| format!("看不懂的设备键：{key}"))?;
    unsafe {
        let mut k = HKEY::default();
        RegOpenKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(wide(sub).as_ptr()), Some(0), KEY_SET_VALUE, &mut k).ok().map_err(|e| format!("打开设备键失败：{e}"))?;
        let r1 = RegSetValueExW(k, PCWSTR(wide("LemonGalaxy.Width").as_ptr()), Some(0), REG_DWORD, Some(&w.to_le_bytes()));
        let r2 = RegSetValueExW(k, PCWSTR(wide("LemonGalaxy.Height").as_ptr()), Some(0), REG_DWORD, Some(&h.to_le_bytes()));
        let _ = RegCloseKey(k);
        r1.ok().and(r2.ok()).map_err(|e| format!("写设备键失败：{e}"))
    }
}

/// 写注册表并生效：先对这块屏 CDS_UPDATEREGISTRY | CDS_NORESET，再整体应用一次
fn apply(name: &str, dm: &DEVMODEW) -> i32 {
    let n = wide(name);
    unsafe {
        let r = ChangeDisplaySettingsExW(PCWSTR(n.as_ptr()), Some(dm), None, CDS_UPDATEREGISTRY | CDS_NORESET, None);
        if r != DISP_CHANGE_SUCCESSFUL {
            return r.0;
        }
        ChangeDisplaySettingsExW(PCWSTR::null(), None, None, CDS_TYPE(0), None).0
    }
}

fn current(name: &str) -> Option<(u32, u32)> {
    let n = wide(name);
    let mut m = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
    unsafe { EnumDisplaySettingsExW(PCWSTR(n.as_ptr()), ENUM_CURRENT_SETTINGS, &mut m, ENUM_DISPLAY_SETTINGS_FLAGS(0)).as_bool() }.then_some((m.dmPelsWidth, m.dmPelsHeight)).filter(|(w, h)| *w > 0 && *h > 0)
}

/// 接入桌面（放在现有桌面右边），返回实际分辨率。
pub fn attach(dev: &Dev, width: u32, height: u32) -> Result<(u32, u32), String> {
    let (w, h) = (width.clamp(640, 3840), height.clamp(480, 2400));
    write_mode(&dev.key, w, h)?;
    let n = wide(&dev.name);
    let x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) + GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    // 先按模式号 0 列一次：GDI 重新向驱动要模式表（含刚写进去的分辨率）
    let mut probe = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
    unsafe {
        let _ = EnumDisplaySettingsExW(PCWSTR(n.as_ptr()), ENUM_DISPLAY_SETTINGS_MODE(0), &mut probe, ENUM_DISPLAY_SETTINGS_FLAGS(0));
    }
    let fallback = STD_MODES.iter().copied().find(|(sw, sh)| *sw <= w && *sh <= h).unwrap_or((1024, 768));
    let mut last = 0;
    for (mw, mh) in [(w, h), fallback] {
        let mut dm = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, dmBitsPerPel: 32, dmPelsWidth: mw, dmPelsHeight: mh, ..Default::default() };
        dm.dmFields = DM_POSITION | DM_PELSWIDTH | DM_PELSHEIGHT | DM_BITSPERPEL;
        dm.Anonymous1.Anonymous2.dmPosition.x = x;
        dm.Anonymous1.Anonymous2.dmPosition.y = 0;
        last = apply(&dev.name, &dm);
        if last == DISP_CHANGE_SUCCESSFUL.0 {
            return Ok(current(&dev.name).unwrap_or((mw, mh)));
        }
    }
    Err(format!("接入虚拟屏 {} 失败（ChangeDisplaySettingsEx {last}）", dev.name))
}

/// 从桌面断开。
pub fn detach(name: &str) -> Result<(), String> {
    let mut dm = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
    dm.dmFields = DM_POSITION | DM_PELSWIDTH | DM_PELSHEIGHT;
    match apply(name, &dm) {
        0 => Ok(()),
        r => Err(format!("断开虚拟屏 {name} 失败（ChangeDisplaySettingsEx {r}）")),
    }
}

/// 代理启动时：上次没断开的我们的屏（代理被杀、重启后注册表里还接着）都断开。
pub fn detach_stale(log: &dyn Fn(String)) {
    for d in devices().into_iter().filter(|d| d.attached) {
        match detach(&d.name) {
            Ok(()) => log(format!("断开上次留下的虚拟屏 {}", d.name)),
            Err(e) => log(e),
        }
    }
}

/// 加一块：找一个没接入、也没被占用的设备接入，等它出现在屏幕列表里。
pub fn add(used: &[String], width: u32, height: u32) -> Result<(String, DisplayInfo), String> {
    let all = devices();
    if all.is_empty() {
        return Err("没有找到 XP 虚拟屏驱动（运行 lemon-galaxy-worker install-display-driver 安装）".into());
    }
    let dev = all.into_iter().find(|d| !d.attached && !used.iter().any(|u| u.eq_ignore_ascii_case(&d.name))).ok_or("虚拟屏已用完")?;
    let t0 = Instant::now();
    let (w, h) = attach(&dev, width, height)?;
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(m) = monitors().into_iter().find(|m| m.device.eq_ignore_ascii_case(&dev.name)) {
            let mut info = m.info();
            info.is_virtual = true;
            tracing::info!("已加 XP 虚拟屏 {}（{w}x{h}，用时 {} ms）", dev.name, t0.elapsed().as_millis());
            return Ok((dev.name, info));
        }
        if Instant::now() > end {
            let _ = detach(&dev.name);
            return Err("新屏没有出现在屏幕列表里".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
