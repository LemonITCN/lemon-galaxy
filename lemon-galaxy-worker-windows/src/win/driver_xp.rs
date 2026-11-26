//! XP 虚拟屏驱动（driver/xp，XPDM）的安装与卸载。驱动文件编译时嵌在 XP 兼容版 exe 里（build.rs），
//! 安装时放到 %ProgramData%\LemonGalaxy\driver-xp\ 再装：建根设备 Root\LemonGalaxyDisplayXP，
//! UpdateDriverForPlugAndPlayDevices 装上（XP 会弹「没有通过 Windows 徽标测试」，要点「仍然继续」，
//! 所以只能在有人看着的桌面上装）。不用重启，装好后系统里多出 2～3 块不接入桌面的显示设备。

use galaxy_worker::t;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use windows::core::{GUID, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiCallClassInstaller, SetupDiCreateDeviceInfoList, SetupDiCreateDeviceInfoW, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceRegistryPropertyW, SetupDiGetINFClassW,
    SetupDiGetDeviceInstallParamsW, SetupDiSetDeviceRegistryPropertyW, SetupUninstallOEMInfW, UpdateDriverForPlugAndPlayDevicesW, DICD_GENERATE_ID, DIF_REGISTERDEVICE, DIF_REMOVE, DIGCF_ALLCLASSES, DI_NEEDREBOOT, DI_NEEDRESTART, HDEVINFO,
    INSTALLFLAG_FORCE, SPDRP_HARDWAREID, SP_DEVINFO_DATA, SP_DEVINSTALL_PARAMS_W, SUOI_FORCEDELETE,
};

use super::vdisplay_xp::HARDWARE_ID;
const INF: &str = "LemonGalaxyDisplayXP.inf";

const FILES: [(&str, &[u8]); 3] = [
    ("lgxpmp.sys", include_bytes!(concat!(env!("OUT_DIR"), "/lgxpmp.sys"))),
    ("lgxpdd.dll", include_bytes!(concat!(env!("OUT_DIR"), "/lgxpdd.dll"))),
    (INF, include_bytes!(concat!(env!("OUT_DIR"), "/LemonGalaxyDisplayXP.inf"))),
];

/// 这台电脑能不能装 XP 驱动：XP / Server 2003（XPDM 驱动在 Vista 起的系统上和新式显卡驱动不能共存）
pub fn applies() -> bool {
    galaxy_host::win::os_version().0 == 5
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

struct DevSet(HDEVINFO);

impl Drop for DevSet {
    fn drop(&mut self) {
        unsafe {
            let _ = SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

/// 我们的根设备
fn our_devices() -> Option<(DevSet, Vec<SP_DEVINFO_DATA>)> {
    unsafe {
        let set = DevSet(SetupDiGetClassDevsW(None, PCWSTR::null(), None, DIGCF_ALLCLASSES).ok()?);
        let mut v = Vec::new();
        for i in 0.. {
            let mut d = SP_DEVINFO_DATA { cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
            if SetupDiEnumDeviceInfo(set.0, i, &mut d).is_err() {
                break;
            }
            let mut buf = [0u8; 512];
            if SetupDiGetDeviceRegistryPropertyW(set.0, &d, SPDRP_HARDWAREID, None, Some(&mut buf), None).is_err() {
                continue;
            }
            let ids: Vec<u16> = buf.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            if ids.split(|c| *c == 0).any(|s| String::from_utf16_lossy(s).eq_ignore_ascii_case(HARDWARE_ID)) {
                v.push(d);
            }
        }
        Some((set, v))
    }
}

/// 驱动文件放到 %ProgramData%\LemonGalaxy\driver-xp\，返回 INF 路径
fn extract() -> Result<PathBuf, String> {
    if FILES.iter().any(|(_, d)| d.is_empty()) {
        return Err(t!("这个版本没有带 XP 虚拟屏驱动。", "This build does not include the XP display driver.").into());
    }
    let pd = std::env::var_os("ALLUSERSPROFILE").or_else(|| std::env::var_os("ProgramData")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Documents and Settings\\All Users"));
    let dir = pd.join("LemonGalaxy").join("driver-xp");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}：{e}", dir.display()))?;
    for (name, data) in FILES {
        std::fs::write(dir.join(name), data).map_err(|e| format!("{}：{e}", dir.join(name).display()))?;
    }
    Ok(dir.join(INF))
}

fn create_root_device(inf: &str) -> Result<(), String> {
    unsafe {
        let inf_w = wide(inf);
        let mut guid = GUID::zeroed();
        let mut class = [0u16; 64];
        SetupDiGetINFClassW(PCWSTR(inf_w.as_ptr()), &mut guid, &mut class, None).map_err(|e| format!("读取 INF 设备类失败：{e}"))?;
        let set = DevSet(SetupDiCreateDeviceInfoList(Some(&guid), None).map_err(|e| format!("{e}"))?);
        let mut d = SP_DEVINFO_DATA { cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
        SetupDiCreateDeviceInfoW(set.0, PCWSTR(class.as_ptr()), &guid, PCWSTR::null(), None, DICD_GENERATE_ID, Some(&mut d)).map_err(|e| format!("建设备失败：{e}"))?;
        let hwid: Vec<u8> = HARDWARE_ID.encode_utf16().chain([0, 0]).flat_map(|c| c.to_le_bytes()).collect();
        SetupDiSetDeviceRegistryPropertyW(set.0, &mut d, SPDRP_HARDWAREID, Some(&hwid)).map_err(|e| format!("设置硬件 ID 失败：{e}"))?;
        SetupDiCallClassInstaller(DIF_REGISTERDEVICE, set.0, Some(&d)).map_err(|e| format!("注册设备失败：{e}"))
    }
}

/// 驱动库里我们的包（%WINDIR%\inf\oemNN.inf，按硬件 ID 认）
fn our_oem_infs() -> Vec<String> {
    let windir = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\WINDOWS"));
    let Ok(rd) = std::fs::read_dir(windir.join("inf")) else { return vec![] };
    rd.flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let lower = name.to_ascii_lowercase();
            (lower.starts_with("oem") && lower.ends_with(".inf")).then_some(())?;
            let text = String::from_utf8_lossy(&std::fs::read(e.path()).ok()?).to_ascii_lowercase();
            text.contains("root\\lemongalaxydisplayxp").then_some(name)
        })
        .collect()
}

/// 安装（已装过就更新驱动）。返回说明文字。
pub fn install() -> Result<String, String> {
    if !applies() {
        return Err(t!("XP 虚拟屏驱动只能装在 Windows XP / Server 2003 上。", "The XP display driver can only be installed on Windows XP / Server 2003.").into());
    }
    if !galaxy_host::sysuser::is_admin() {
        return Err(t!("需要以管理员身份运行。", "Please run as administrator.").into());
    }
    let inf = extract()?;
    let inf_s = inf.to_string_lossy().to_string();
    let mut notes = Vec::new();
    let created = our_devices().map(|(_, v)| v.is_empty()).unwrap_or(true);
    if created {
        create_root_device(&inf_s)?;
        notes.push(t!("已建设备 Root\\LemonGalaxyDisplayXP", "Created device Root\\LemonGalaxyDisplayXP").to_string());
    }
    println!("{}", t!("正在安装 XP 虚拟屏驱动：系统提示「没有通过 Windows 徽标测试」时点「仍然继续」。", "Installing the XP display driver: click \"Continue Anyway\" when Windows warns that it has not passed Windows Logo testing."));
    let mut reboot = windows::core::BOOL(0);
    let r = unsafe { UpdateDriverForPlugAndPlayDevicesW(None, PCWSTR(wide(HARDWARE_ID).as_ptr()), PCWSTR(wide(&inf_s).as_ptr()), INSTALLFLAG_FORCE, Some(&mut reboot)) };
    if let Err(e) = r {
        if created {
            let _ = remove_devices();
        }
        return Err(format!("{}：{e}（0x{:08X}）", t!("安装驱动失败（是不是点了「停止安装」？）", "Driver installation failed (was \"Stop Installation\" clicked?)"), e.code().0 as u32));
    }
    if reboot.as_bool() {
        notes.push(t!("需要重启后生效", "A reboot is required").into());
    }
    let end = Instant::now() + Duration::from_secs(15);
    while Instant::now() < end && super::vdisplay_xp::devices().is_empty() {
        std::thread::sleep(Duration::from_millis(250));
    }
    let n = super::vdisplay_xp::devices().len();
    if n > 0 {
        notes.push(format!("{} {n}", t!("驱动已就绪，可用虚拟屏：", "Driver ready, virtual displays available:")));
        Ok(notes.join("\n"))
    } else if reboot.as_bool() {
        Ok(notes.join("\n"))
    } else {
        Err(format!("{}\n{}", notes.join("\n"), t!("驱动装上了但没有出现虚拟屏（看设备管理器里 LemonGalaxy Virtual Display 的状态）", "Driver installed but no virtual display appeared (check LemonGalaxy Virtual Display in Device Manager)")))
    }
}

/// 删设备，返回 (删了几个, 要不要重启)。显示驱动卸不下来，设备要重启后才彻底没了。
fn remove_devices() -> Result<(usize, bool), String> {
    let Some((set, devs)) = our_devices() else { return Ok((0, false)) };
    let mut reboot = false;
    for d in &devs {
        unsafe { SetupDiCallClassInstaller(DIF_REMOVE, set.0, Some(d)) }.map_err(|e| format!("删设备失败：{e}"))?;
        let mut p = SP_DEVINSTALL_PARAMS_W { cbSize: std::mem::size_of::<SP_DEVINSTALL_PARAMS_W>() as u32, ..Default::default() };
        if unsafe { SetupDiGetDeviceInstallParamsW(set.0, Some(d), &mut p) }.is_ok() && p.Flags.0 & (DI_NEEDREBOOT.0 | DI_NEEDRESTART.0) != 0 {
            reboot = true;
        }
    }
    Ok((devs.len(), reboot))
}

/// 卸载：先断开接着的虚拟屏，再删设备、删驱动包。
pub fn uninstall() -> Result<String, String> {
    if !galaxy_host::sysuser::is_admin() {
        return Err(t!("需要以管理员身份运行。", "Please run as administrator.").into());
    }
    let mut notes = Vec::new();
    for d in super::vdisplay_xp::devices().into_iter().filter(|d| d.attached) {
        let _ = super::vdisplay_xp::detach(&d.name);
    }
    let (n, reboot) = remove_devices()?;
    if n > 0 {
        notes.push(format!("{} {n}", t!("已删设备", "Removed devices:")));
    }
    if reboot || !super::vdisplay_xp::devices().is_empty() {
        notes.push(t!("重启后虚拟屏才会从系统里彻底消失", "Restart to remove the virtual displays completely").into());
    }
    for inf in our_oem_infs() {
        if unsafe { SetupUninstallOEMInfW(PCWSTR(wide(&inf).as_ptr()), SUOI_FORCEDELETE, None) }.as_bool() {
            notes.push(format!("{} {inf}", t!("已删驱动包", "Deleted package")));
        }
    }
    if notes.is_empty() {
        notes.push(t!("没有安装虚拟屏驱动。", "Display driver is not installed.").into());
    }
    Ok(notes.join("\n"))
}
