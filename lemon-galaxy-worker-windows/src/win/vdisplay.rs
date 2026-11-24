//! 虚拟屏（画面代理这一侧）：打开 LemonGalaxy Display 驱动（driver/LemonGalaxyDisplay），按槽位加删屏，
//! 用 SetDisplayConfig 把新屏扩展进桌面，删完后恢复原显示配置。
//!
//! 一块屏属于加它的驱动句柄，句柄放在画面代理里：代理退出或崩溃（服务停止、会话切换）时驱动自动删屏，
//! 不会留下幽灵屏。控制接口见 driver/LemonGalaxyDisplay/LemonGalaxyDisplayControl.h，下面的结构按同样的布局写。
//! 移植自驱动测试工具 LemonGalaxyDisplayTest 的 Activate / FindOutput / RestoreIfChanged。

use super::capture::monitors;
use galaxy_proto::peer::DisplayInfo;
use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};
use windows::core::{GUID, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_List_SizeW, CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CR_SUCCESS};
// 这四个是 Windows 7 起才有的，经 compat 运行时加载（老系统兼容版的导入表里不能有）
use super::compat::win7::{DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig, SetDisplayConfig};
use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE,
    DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME, QDC_ALL_PATHS, QDC_ONLY_ACTIVE_PATHS, QUERY_DISPLAY_CONFIG_FLAGS, SDC_ALLOW_CHANGES, SDC_APPLY, SDC_USE_SUPPLIED_DISPLAY_CONFIG,
};
use windows::Win32::Foundation::{CloseHandle, ERROR_INSUFFICIENT_BUFFER, ERROR_NOT_READY, ERROR_SUCCESS, HANDLE, LUID};
use windows::Win32::Graphics::Gdi::{ChangeDisplaySettingsExW, EnumDisplaySettingsExW, CDS_TYPE, DEVMODEW, DISP_CHANGE_SUCCESSFUL, DM_DISPLAYFREQUENCY, DM_PELSHEIGHT, DM_PELSWIDTH, ENUM_CURRENT_SETTINGS, ENUM_DISPLAY_SETTINGS_FLAGS};
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::System::IO::DeviceIoControl;

/// 设备接口 GUID {BE3C34E4-6555-40C7-916D-A2DB09DAB193}
pub const INTERFACE_GUID: GUID = GUID::from_u128(0xbe3c34e4_6555_40c7_916d_a2db09dab193);
const VERSION: u32 = 1;
/// 驱动的槽位总数：3 块给主控 + 1 块无头兜底
const MAX_MONITORS: u32 = 4;
/// 给主控的上限（内核再留一块给无头兜底屏）
pub const CONTROLLER_MAX: u32 = 3;
const MAX_MODES: usize = 64;
const NAME_CHARS: usize = 14;

// CTL_CODE(FILE_DEVICE_UNKNOWN, 0x900 + n, METHOD_BUFFERED, FILE_READ_DATA | FILE_WRITE_DATA)
const fn ioctl(n: u32) -> u32 {
    (0x22 << 16) | (3 << 14) | ((0x900 + n) << 2)
}
const IOCTL_GET_INFO: u32 = ioctl(0);
const IOCTL_ADD: u32 = ioctl(1);
const IOCTL_REMOVE: u32 = ioctl(2);

const PATH_ACTIVE: u32 = 1;
const MODE_IDX_INVALID: u32 = 0xffff_ffff;

#[repr(C)]
#[derive(Default, Debug, Clone, Copy)]
struct LgInfo {
    version: u32,
    max_monitors: u32,
    active_monitors: u32,
    adapter_ready: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct LgMode {
    width: u32,
    height: u32,
    refresh: u32,
}

#[repr(C)]
struct LgAddRequest {
    version: u32,
    connector: u32,
    serial: u32,
    width_mm: u32,
    height_mm: u32,
    name: [u16; NAME_CHARS],
    mode_count: u32,
    preferred_mode: u32,
    modes: [LgMode; MAX_MODES],
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct LgAddResult {
    version: u32,
    target_id: u32,
    adapter_luid_low: u32,
    adapter_luid_high: i32,
}

#[repr(C)]
struct LgRemoveRequest {
    version: u32,
    connector: u32,
}

const _: () = assert!(std::mem::size_of::<LgAddRequest>() == 824);
const _: () = assert!(std::mem::size_of::<LgAddResult>() == 16);

fn utf16_str(b: &[u16]) -> String {
    let n = b.iter().position(|c| *c == 0).unwrap_or(b.len());
    String::from_utf16_lossy(&b[..n])
}

/// 驱动的设备接口路径（以 0 结尾）；驱动没装或设备没启动时为 None。
fn interface_path() -> Option<Vec<u16>> {
    unsafe {
        let mut len = 0u32;
        if CM_Get_Device_Interface_List_SizeW(&mut len, &INTERFACE_GUID, PCWSTR::null(), CM_GET_DEVICE_INTERFACE_LIST_PRESENT) != CR_SUCCESS || len <= 1 {
            return None;
        }
        let mut buf = vec![0u16; len as usize];
        if CM_Get_Device_Interface_ListW(&INTERFACE_GUID, PCWSTR::null(), &mut buf, CM_GET_DEVICE_INTERFACE_LIST_PRESENT) != CR_SUCCESS {
            return None;
        }
        let n = buf.iter().position(|c| *c == 0).unwrap_or(0);
        if n == 0 {
            return None;
        }
        buf.truncate(n + 1);
        Some(buf)
    }
}

/// 驱动装好且设备在运行（服务进程判断能力用，不打开设备）。
pub fn driver_present() -> bool {
    interface_path().is_some()
}

struct Driver(HANDLE);

impl Drop for Driver {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

impl Driver {
    fn open() -> Result<Self, String> {
        let path = interface_path().ok_or("没有找到虚拟屏驱动（LemonGalaxy Display 没有安装或没有启动）")?;
        unsafe {
            CreateFileW(PCWSTR(path.as_ptr()), 0xC000_0000, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None)
                .map(Self)
                .map_err(|e| format!("打开虚拟屏驱动失败：{e}（要以 SYSTEM 或管理员身份运行）"))
        }
    }

    fn info(&self) -> Option<LgInfo> {
        let mut info = LgInfo::default();
        let mut n = 0u32;
        unsafe { DeviceIoControl(self.0, IOCTL_GET_INFO, None, 0, Some(&mut info as *mut _ as _), std::mem::size_of::<LgInfo>() as u32, Some(&mut n), None).ok()? };
        (n as usize == std::mem::size_of::<LgInfo>()).then_some(info)
    }

    fn add(&self, connector: u32, mode: (u32, u32, u32)) -> Result<LgAddResult, String> {
        let mut req = LgAddRequest {
            version: VERSION,
            connector,
            // 每个槽位固定的 EDID 序列号，Windows 按它记住布局
            serial: 0x4C47_0100 + connector,
            // 约 24 英寸 16:9，Windows 按它算默认缩放（100%）
            width_mm: 527,
            height_mm: 296,
            name: [0; NAME_CHARS],
            mode_count: 0,
            preferred_mode: 0,
            modes: [LgMode::default(); MAX_MODES],
        };
        for (i, c) in "LemonGalaxy".encode_utf16().take(NAME_CHARS - 1).enumerate() {
            req.name[i] = c;
        }
        let modes = common_modes(mode);
        req.mode_count = modes.len() as u32;
        req.modes[..modes.len()].copy_from_slice(&modes);
        let mut out = LgAddResult::default();
        // 驱动刚启动时适配器还没就绪（ERROR_NOT_READY），稍等重试
        for _ in 0..30 {
            let mut n = 0u32;
            let r = unsafe { DeviceIoControl(self.0, IOCTL_ADD, Some(&req as *const _ as _), std::mem::size_of::<LgAddRequest>() as u32, Some(&mut out as *mut _ as _), std::mem::size_of::<LgAddResult>() as u32, Some(&mut n), None) };
            match r {
                Ok(()) => return Ok(out),
                Err(e) if e.code() == ERROR_NOT_READY.to_hresult() => std::thread::sleep(Duration::from_millis(100)),
                Err(e) => return Err(format!("驱动加屏失败：{e}")),
            }
        }
        Err("虚拟屏驱动的适配器一直没有就绪".into())
    }

    fn remove(&self, connector: u32) -> Result<(), String> {
        let req = LgRemoveRequest { version: VERSION, connector };
        let mut n = 0u32;
        unsafe { DeviceIoControl(self.0, IOCTL_REMOVE, Some(&req as *const _ as _), std::mem::size_of::<LgRemoveRequest>() as u32, None, 0, Some(&mut n), None) }.map_err(|e| format!("驱动删屏失败：{e}"))
    }
}

/// 要求的模式排第一（首选），再加常见 16:9 / 16:10 分辨率 x 30/60/120/144 Hz，主控以后可以直接切。
fn common_modes(preferred: (u32, u32, u32)) -> Vec<LgMode> {
    const SIZES: [(u32, u32); 12] = [(1280, 720), (1366, 768), (1600, 900), (1920, 1080), (2560, 1440), (3200, 1800), (3840, 2160), (1280, 800), (1440, 900), (1680, 1050), (1920, 1200), (2560, 1600)];
    let mut v = vec![LgMode { width: preferred.0, height: preferred.1, refresh: preferred.2 }];
    for (w, h) in SIZES {
        for r in [30, 60, 120, 144] {
            if (w, h, r) != preferred && v.len() < MAX_MODES {
                v.push(LgMode { width: w, height: h, refresh: r });
            }
        }
    }
    v
}

// ---------------- 显示配置 ----------------

struct Config {
    paths: Vec<DISPLAYCONFIG_PATH_INFO>,
    modes: Vec<DISPLAYCONFIG_MODE_INFO>,
}

fn same_luid(a: LUID, b: LUID) -> bool {
    a.LowPart == b.LowPart && a.HighPart == b.HighPart
}

fn query(flags: QUERY_DISPLAY_CONFIG_FLAGS) -> Option<Config> {
    for _ in 0..5 {
        let (mut np, mut nm) = (0u32, 0u32);
        unsafe {
            if GetDisplayConfigBufferSizes(flags, &mut np, &mut nm) != ERROR_SUCCESS {
                return None;
            }
            let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
            let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
            let rc = QueryDisplayConfig(flags, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None);
            if rc == ERROR_INSUFFICIENT_BUFFER {
                continue;
            }
            if rc != ERROR_SUCCESS {
                return None;
            }
            paths.truncate(np as usize);
            modes.truncate(nm as usize);
            return Some(Config { paths, modes });
        }
    }
    None
}

fn source_name(p: &DISPLAYCONFIG_PATH_INFO) -> String {
    let mut s = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
    s.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
    s.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
    s.header.adapterId = p.sourceInfo.adapterId;
    s.header.id = p.sourceInfo.id;
    if unsafe { DisplayConfigGetDeviceInfo(&mut s.header) } != 0 {
        return String::new();
    }
    utf16_str(&s.viewGdiDeviceName)
}

fn target_path(p: &DISPLAYCONFIG_PATH_INFO) -> String {
    let mut t = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
    t.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
    t.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
    t.header.adapterId = p.targetInfo.adapterId;
    t.header.id = p.targetInfo.id;
    unsafe { DisplayConfigGetDeviceInfo(&mut t.header) };
    utf16_str(&t.monitorDevicePath)
}

fn is_target(p: &DISPLAYCONFIG_PATH_INFO, luid: LUID, target: u32) -> bool {
    p.targetInfo.id == target && same_luid(p.targetInfo.adapterId, luid)
}

/// 活动路径摘要 (GDI 名, 显示器路径, x, y, 宽, 高)，用来判断“恢复后是不是和原来一样”。
fn summary(c: &Config) -> BTreeSet<(String, String, i32, i32, u32, u32)> {
    let mut set = BTreeSet::new();
    for p in c.paths.iter().filter(|p| p.flags & PATH_ACTIVE != 0) {
        let (mut x, mut y, mut w, mut h) = (0, 0, 0, 0);
        let idx = unsafe { p.sourceInfo.Anonymous.modeInfoIdx } as usize;
        if let Some(m) = c.modes.get(idx).filter(|m| m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE) {
            let sm = unsafe { m.Anonymous.sourceMode };
            (x, y, w, h) = (sm.position.x, sm.position.y, sm.width, sm.height);
        }
        set.insert((source_name(p), target_path(p), x, y, w, h));
    }
    set
}

fn find_path(flags: QUERY_DISPLAY_CONFIG_FLAGS, luid: LUID, target: u32) -> Option<DISPLAYCONFIG_PATH_INFO> {
    let c = query(flags)?;
    c.paths.into_iter().find(|p| is_target(p, luid, target) && (flags != QDC_ONLY_ACTIVE_PATHS || p.flags & PATH_ACTIVE != 0))
}

/// 只把这块屏的路径加进现有拓扑（扩展），其它屏原样保留；不写进显示数据库。
fn activate(luid: LUID, target: u32) -> Result<(), String> {
    let all = query(QDC_ALL_PATHS).ok_or("读取显示配置失败")?;
    let mut next: Vec<DISPLAYCONFIG_PATH_INFO> = Vec::new();
    for p in all.paths.iter().filter(|p| p.flags & PATH_ACTIVE != 0) {
        if is_target(p, luid, target) {
            return Ok(());
        }
        next.push(*p);
    }
    let cand = all
        .paths
        .iter()
        .filter(|p| is_target(p, luid, target) && p.targetInfo.targetAvailable.as_bool())
        .find(|p| !next.iter().any(|a| a.sourceInfo.id == p.sourceInfo.id && same_luid(a.sourceInfo.adapterId, p.sourceInfo.adapterId)))
        .ok_or("新屏没有可用的显示路径")?;
    let mut added = *cand;
    added.flags = PATH_ACTIVE;
    added.sourceInfo.Anonymous.modeInfoIdx = MODE_IDX_INVALID;
    added.targetInfo.Anonymous.modeInfoIdx = MODE_IDX_INVALID;
    next.push(added);
    let rc = unsafe { SetDisplayConfig(Some(&next), Some(&all.modes), SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_ALLOW_CHANGES) };
    if rc != 0 {
        return Err(format!("扩展桌面失败（SetDisplayConfig {rc}）"));
    }
    Ok(())
}

fn wait_for(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + timeout;
    loop {
        if f() {
            return true;
        }
        if Instant::now() >= end {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn current_mode(name: &str) -> Option<(u32, u32, u32)> {
    let w: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let mut m = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
    unsafe { EnumDisplaySettingsExW(PCWSTR(w.as_ptr()), ENUM_CURRENT_SETTINGS, &mut m, ENUM_DISPLAY_SETTINGS_FLAGS(0)).as_bool() }.then_some((m.dmPelsWidth, m.dmPelsHeight, m.dmDisplayFrequency))
}

/// 改当前模式（不写注册表）。
fn set_mode(name: &str, mode: (u32, u32, u32)) -> bool {
    let w: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let mut m = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, dmPelsWidth: mode.0, dmPelsHeight: mode.1, dmDisplayFrequency: mode.2, ..Default::default() };
    m.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY;
    unsafe { ChangeDisplaySettingsExW(PCWSTR(w.as_ptr()), Some(&m), None, CDS_TYPE(0), None) == DISP_CHANGE_SUCCESSFUL }
}

/// GDI 名 `\\.\DISPLAYn` 对应的屏幕 id（与 capture::monitors 的编号一致）。
fn id_of(gdi: &str) -> Option<u32> {
    let digits: String = gdi.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
    digits.parse().ok()
}

// ---------------- 管理 ----------------

struct Slot {
    luid: LUID,
    target: u32,
    gdi: String,
}

/// 画面代理里的虚拟屏：驱动句柄与已加的屏（按槽位）。只在一个线程里用。
#[derive(Default)]
pub struct VirtualDisplays {
    driver: Option<Driver>,
    slots: HashMap<u32, Slot>,
    /// 第一块虚拟屏加之前的显示配置：最后一块删掉后若配置变了就恢复它
    baseline: Option<Config>,
}

impl VirtualDisplays {
    /// 我们的虚拟屏的 GDI 名（列屏幕时标记 is_virtual）。
    pub fn names(&self) -> Vec<String> {
        self.slots.values().map(|s| s.gdi.clone()).collect()
    }

    pub fn ids(&self) -> Vec<u32> {
        self.slots.values().filter_map(|s| id_of(&s.gdi)).collect()
    }

    /// 没有任何亮着的显示器（不算我们的虚拟屏）。
    pub fn needs_headless(&self) -> bool {
        let Some(c) = query(QDC_ONLY_ACTIVE_PATHS) else { return false };
        !c.paths.iter().any(|p| p.flags & PATH_ACTIVE != 0 && !self.slots.values().any(|s| is_target(p, s.luid, s.target)))
    }

    pub fn add(&mut self, width: u32, height: u32, refresh: u32, log: &dyn Fn(String)) -> Result<DisplayInfo, String> {
        if self.driver.is_none() {
            let d = Driver::open()?;
            if let Some(i) = d.info() {
                log(format!("虚拟屏驱动：接口版本 {}，最多 {} 块，已有 {} 块，适配器就绪 {}", i.version, i.max_monitors, i.active_monitors, i.adapter_ready));
            }
            self.driver = Some(d);
        }
        let connector = (0..MAX_MONITORS).find(|c| !self.slots.contains_key(c)).ok_or("虚拟屏槽位已用完")?;
        if self.slots.is_empty() {
            self.baseline = query(QDC_ONLY_ACTIVE_PATHS);
        }
        let t0 = Instant::now();
        let r = self.driver.as_ref().unwrap().add(connector, (width, height, refresh))?;
        let luid = LUID { LowPart: r.adapter_luid_low, HighPart: r.adapter_luid_high };
        let mut slot = Slot { luid, target: r.target_id, gdi: String::new() };
        let result = (|| {
            if !wait_for(Duration::from_secs(5), || find_path(QDC_ALL_PATHS, luid, r.target_id).is_some()) {
                return Err("新屏没有出现在显示配置里".to_string());
            }
            activate(luid, r.target_id)?;
            let mut path = None;
            if !wait_for(Duration::from_secs(5), || {
                path = find_path(QDC_ONLY_ACTIVE_PATHS, luid, r.target_id);
                path.is_some()
            }) {
                return Err("新屏没能扩展进桌面".to_string());
            }
            let gdi = source_name(path.as_ref().unwrap());
            if gdi.is_empty() {
                return Err("拿不到新屏的 GDI 名".to_string());
            }
            slot.gdi = gdi.clone();
            // 首选模式就是要求的模式；万一 Windows 选了别的，改过来
            if current_mode(&gdi).map(|m| m != (width, height, refresh)).unwrap_or(true) && !set_mode(&gdi, (width, height, refresh)) {
                log(format!("虚拟屏 {gdi} 切到 {width}x{height}@{refresh} 失败，用当前模式 {:?}", current_mode(&gdi)));
            }
            // 等 GDI 屏幕列表里出现它（采集按这个列表找屏）
            let mut found = None;
            wait_for(Duration::from_secs(3), || {
                found = monitors().into_iter().find(|m| m.device.eq_ignore_ascii_case(&gdi));
                found.is_some()
            });
            let m = found.ok_or("新屏没有出现在屏幕列表里")?;
            let mut info = m.info();
            info.is_virtual = true;
            Ok(info)
        })();
        match result {
            Ok(info) => {
                log(format!("已加虚拟屏 {}（槽位 {connector}，目标 {}，{}x{}，用时 {} ms）", slot.gdi, r.target_id, info.width, info.height, t0.elapsed().as_millis()));
                self.slots.insert(connector, slot);
                Ok(info)
            }
            Err(e) => {
                let _ = self.driver.as_ref().unwrap().remove(connector);
                self.restore_if_last(log);
                Err(e)
            }
        }
    }

    /// 按屏幕 id 删；不是我们的屏时返回 Ok（已经不在了）。
    pub fn remove(&mut self, id: u32, log: &dyn Fn(String)) -> Result<(), String> {
        let Some(connector) = self.slots.iter().find(|(_, s)| id_of(&s.gdi) == Some(id)).map(|(c, _)| *c) else { return Ok(()) };
        self.remove_slot(connector, log)
    }

    fn remove_slot(&mut self, connector: u32, log: &dyn Fn(String)) -> Result<(), String> {
        let Some(slot) = self.slots.remove(&connector) else { return Ok(()) };
        let t0 = Instant::now();
        let r = self.driver.as_ref().map(|d| d.remove(connector)).unwrap_or(Ok(()));
        // 删掉后目标可能还留在 QDC_ALL_PATHS 里，但不再活动、targetAvailable 为假
        wait_for(Duration::from_secs(5), || {
            find_path(QDC_ONLY_ACTIVE_PATHS, slot.luid, slot.target).is_none() && find_path(QDC_ALL_PATHS, slot.luid, slot.target).map(|p| !p.targetInfo.targetAvailable.as_bool()).unwrap_or(true)
        });
        log(format!("已删虚拟屏 {}（槽位 {connector}，用时 {} ms）", slot.gdi, t0.elapsed().as_millis()));
        self.restore_if_last(log);
        r
    }

    /// 最后一块虚拟屏删掉后：显示配置和加屏前不一样就恢复（原配置没有活动路径时不动）。
    fn restore_if_last(&mut self, log: &dyn Fn(String)) {
        if !self.slots.is_empty() {
            return;
        }
        let Some(base) = self.baseline.take() else { return };
        if base.paths.is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(300));
        let Some(now) = query(QDC_ONLY_ACTIVE_PATHS) else { return };
        if summary(&now) == summary(&base) {
            return;
        }
        let rc = unsafe { SetDisplayConfig(Some(&base.paths), Some(&base.modes), SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_ALLOW_CHANGES) };
        log(format!("虚拟屏都删掉了，显示配置和原来不同，已恢复（SetDisplayConfig {rc}）"));
    }

    /// 全部删掉（代理退出前）。
    pub fn clear(&mut self, log: &dyn Fn(String)) {
        let all: Vec<u32> = self.slots.keys().copied().collect();
        for c in all {
            let _ = self.remove_slot(c, log);
        }
        self.driver = None;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ioctl_codes() {
        assert_eq!(super::IOCTL_GET_INFO, 0x0022_E400);
        assert_eq!(super::IOCTL_ADD, 0x0022_E404);
        assert_eq!(super::IOCTL_REMOVE, 0x0022_E408);
        assert_eq!(super::id_of("\\\\.\\DISPLAY12"), Some(12));
        let m = super::common_modes((2560, 1440, 120));
        assert_eq!((m[0].width, m[0].height, m[0].refresh), (2560, 1440, 120));
        assert!(m.len() <= super::MAX_MODES);
    }
}
