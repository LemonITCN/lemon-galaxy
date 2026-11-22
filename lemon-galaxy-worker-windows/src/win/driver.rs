//! 虚拟屏驱动（LemonGalaxy Display，IddCx 用户态驱动）的安装与卸载，管理员运行：
//!
//! ```text
//! lemon-galaxy-worker install-display-driver [--dev] [驱动包目录]
//! lemon-galaxy-worker uninstall-display-driver
//! ```
//!
//! 驱动包目录里要有 LemonGalaxyDisplay.inf / .cat / .dll（driver/ 下用 EWDK 或 CI 编出来的
//! build\<平台>\Release\LemonGalaxyDisplay）。不写目录时依次找程序旁边的 `driver\`、`LemonGalaxyDisplay\`。
//!
//! - 正式包（签过名，目录里没有测试证书）：建根设备 `Root\LemonGalaxyDisplay`（与 devcon install 相同：
//!   SetupDiCreateDeviceInfo + DIF_REGISTERDEVICE），再 UpdateDriverForPlugAndPlayDevices 把包放进驱动库并装上；
//!   已有设备时只更新驱动。
//! - 测试签名包（目录里有 sign.ps1 导出的 LemonGalaxyTest.cer）：只在开发机上装，要加 `--dev` 或设环境变量
//!   `LEMON_GALAXY_DEV_DRIVER=1`，先把测试证书放进本机“受信任的根”和“受信任的发布者”（UMDF 驱动这样就能加载，
//!   不用开测试模式），和 driver/tools/install.ps1 一样。
//! - install-service 时程序旁边有驱动包就顺带装（复制到安装目录的 driver\ 下），测试包同样要开发标志。
//!
//! 只按自己的硬件 ID、INF 内容和证书主题操作，不碰其它虚拟屏驱动（LemonLink Display、GameViewer 等）。

use galaxy_worker::t;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use windows::core::{GUID, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    DiUninstallDevice, SetupDiCallClassInstaller, SetupDiCreateDeviceInfoList, SetupDiCreateDeviceInfoW, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW, SetupDiGetDeviceRegistryPropertyW,
    SetupDiGetINFClassW, SetupDiOpenDevRegKey, SetupDiSetDeviceRegistryPropertyW, SetupUninstallOEMInfW, UpdateDriverForPlugAndPlayDevicesW, DICD_GENERATE_ID, DICS_FLAG_GLOBAL, DIF_REGISTERDEVICE, DIGCF_ALLCLASSES, DIREG_DRV, HDEVINFO,
    INSTALLFLAG_FORCE, SPDRP_HARDWAREID, SP_DEVINFO_DATA, SUOI_FORCEDELETE,
};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Registry::{RegCloseKey, RegGetValueW, KEY_READ, RRF_RT_REG_SZ};

pub const HARDWARE_ID: &str = "Root\\LemonGalaxyDisplay";
const INF_NAME: &str = "LemonGalaxyDisplay.inf";
const TEST_CERT: &str = "LemonGalaxyTest.cer";
const TEST_CERT_SUBJECT: &str = "LemonGalaxy Test Driver Signing";
pub const DEV_ENV: &str = "LEMON_GALAXY_DEV_DRIVER";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn utf16_str(b: &[u16]) -> String {
    let n = b.iter().position(|c| *c == 0).unwrap_or(b.len());
    String::from_utf16_lossy(&b[..n])
}

/// 开发标志：命令行 --dev 或环境变量 LEMON_GALAXY_DEV_DRIVER=1。
pub fn dev_flag(args: &[String]) -> bool {
    args.iter().any(|a| a == "--dev") || std::env::var(DEV_ENV).map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false)
}

/// 程序旁边的驱动包目录（没有时为 None）。
pub fn package_next_to_exe() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    ["driver", "LemonGalaxyDisplay"].iter().map(|d| dir.join(d)).find(|d| d.join(INF_NAME).is_file())
}

pub fn is_test_package(pkg: &Path) -> bool {
    pkg.join(TEST_CERT).is_file()
}

// ---------------- 设备 ----------------

struct DevSet(HDEVINFO);

impl Drop for DevSet {
    fn drop(&mut self) {
        unsafe {
            let _ = SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

/// 我们的根设备（含没在运行的）：(设备集, 设备数据, 实例 ID)。
fn our_devices() -> Result<(DevSet, Vec<(SP_DEVINFO_DATA, String)>), String> {
    unsafe {
        let set = SetupDiGetClassDevsW(None, PCWSTR(wide("ROOT").as_ptr()), None, DIGCF_ALLCLASSES).map_err(|e| format!("枚举设备失败：{e}"))?;
        let set = DevSet(set);
        let mut out = Vec::new();
        for i in 0.. {
            let mut d = SP_DEVINFO_DATA { cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
            if SetupDiEnumDeviceInfo(set.0, i, &mut d).is_err() {
                break;
            }
            let mut buf = [0u8; 1024];
            if SetupDiGetDeviceRegistryPropertyW(set.0, &d, SPDRP_HARDWAREID, None, Some(&mut buf), None).is_err() {
                continue;
            }
            let ids: Vec<u16> = buf.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let ours = ids.split(|c| *c == 0).any(|s| String::from_utf16_lossy(s).eq_ignore_ascii_case(HARDWARE_ID));
            if !ours {
                continue;
            }
            let mut id = [0u16; 256];
            let _ = SetupDiGetDeviceInstanceIdW(set.0, &d, Some(&mut id), None);
            out.push((d, utf16_str(&id)));
        }
        Ok((set, out))
    }
}

/// 设备正在用的驱动包（oemNN.inf）。
fn inf_in_use(set: &DevSet, d: &SP_DEVINFO_DATA) -> Option<String> {
    unsafe {
        let key = SetupDiOpenDevRegKey(set.0, d, DICS_FLAG_GLOBAL.0, 0, DIREG_DRV, KEY_READ.0).ok()?;
        let mut buf = [0u16; 260];
        let mut len = (buf.len() * 2) as u32;
        let r = RegGetValueW(key, PCWSTR::null(), PCWSTR(wide("InfPath").as_ptr()), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut len));
        let _ = RegCloseKey(key);
        r.is_ok().then(|| utf16_str(&buf))
    }
}

/// 建根设备节点（与 devcon install 相同），返回实例 ID。
fn create_root_device(inf: &Path) -> Result<String, String> {
    unsafe {
        let inf_w = wide(&inf.to_string_lossy());
        let mut guid = GUID::zeroed();
        let mut class = [0u16; 64];
        SetupDiGetINFClassW(PCWSTR(inf_w.as_ptr()), &mut guid, &mut class, None).map_err(|e| format!("读取 INF 设备类失败：{e}"))?;
        let set = DevSet(SetupDiCreateDeviceInfoList(Some(&guid), None).map_err(|e| format!("{e}"))?);
        let mut d = SP_DEVINFO_DATA { cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
        SetupDiCreateDeviceInfoW(set.0, PCWSTR(class.as_ptr()), &guid, PCWSTR::null(), None, DICD_GENERATE_ID, Some(&mut d)).map_err(|e| format!("建设备失败：{e}"))?;
        // 硬件 ID 是 REG_MULTI_SZ：两个 0 结尾
        let hwid: Vec<u8> = HARDWARE_ID.encode_utf16().chain([0, 0]).flat_map(|c| c.to_le_bytes()).collect();
        SetupDiSetDeviceRegistryPropertyW(set.0, &mut d, SPDRP_HARDWAREID, Some(&hwid)).map_err(|e| format!("设置硬件 ID 失败：{e}"))?;
        SetupDiCallClassInstaller(DIF_REGISTERDEVICE, set.0, Some(&d)).map_err(|e| format!("注册设备失败：{e}"))?;
        let mut id = [0u16; 256];
        let _ = SetupDiGetDeviceInstanceIdW(set.0, &d, Some(&mut id), None);
        Ok(utf16_str(&id))
    }
}

fn remove_devices() -> Result<Vec<String>, String> {
    let (set, devs) = our_devices()?;
    let mut removed = Vec::new();
    for (d, id) in devs {
        unsafe { DiUninstallDevice(HWND::default(), set.0, &d, 0, None) }.map_err(|e| format!("删设备 {id} 失败：{e}"))?;
        removed.push(id);
    }
    Ok(removed)
}

/// 驱动库里我们的包（%WINDIR%\INF\oemNN.inf，按内容认：硬件 ID 与目录文件名）。
fn our_oem_infs() -> Vec<String> {
    let windir = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Windows"));
    let Ok(rd) = std::fs::read_dir(windir.join("INF")) else { return vec![] };
    let mut v = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let lower = name.to_ascii_lowercase();
        if !(lower.starts_with("oem") && lower.ends_with(".inf")) {
            continue;
        }
        let Ok(b) = std::fs::read(e.path()) else { continue };
        let text = if b.len() >= 2 && b[0] == 0xFF && b[1] == 0xFE {
            String::from_utf16_lossy(&b[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>())
        } else {
            String::from_utf8_lossy(&b).to_string()
        }
        .to_ascii_lowercase();
        if text.contains("root\\lemongalaxydisplay") && text.contains("lemongalaxydisplay.cat") {
            v.push(name);
        }
    }
    v
}

fn delete_oem_inf(name: &str) -> bool {
    unsafe { SetupUninstallOEMInfW(PCWSTR(wide(name).as_ptr()), SUOI_FORCEDELETE, None).as_bool() }
}

fn certutil(args: &[&str]) -> Result<(), String> {
    let out = std::process::Command::new("certutil").args(args).output().map_err(|e| format!("certutil：{e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("certutil {} 失败（{}）", args.join(" "), out.status))
    }
}

fn wait_interface(timeout: Duration) -> bool {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if super::vdisplay::driver_present() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    false
}

/// 安装或更新驱动。返回说明文字。
pub fn install(pkg: &Path, dev: bool) -> Result<String, String> {
    if !galaxy_host::sysuser::is_admin() {
        return Err(t!("需要以管理员身份运行。", "Please run as administrator.").into());
    }
    let pkg = std::fs::canonicalize(pkg).map_err(|e| format!("{}：{e}", pkg.display()))?;
    let pkg = PathBuf::from(pkg.to_string_lossy().trim_start_matches("\\\\?\\"));
    let inf = pkg.join(INF_NAME);
    for f in [INF_NAME, "LemonGalaxyDisplay.cat", "LemonGalaxyDisplay.dll"] {
        if !pkg.join(f).is_file() {
            return Err(format!("{} {}", t!("驱动包不完整，缺少", "Driver package incomplete, missing"), pkg.join(f).display()));
        }
    }
    let mut notes = Vec::new();
    if is_test_package(&pkg) {
        if !dev {
            return Err(t!(
                "这是测试签名的驱动包，只能装在开发机上：加 --dev 或设环境变量 LEMON_GALAXY_DEV_DRIVER=1（会把测试证书放进本机受信任的根和受信任的发布者）。",
                "This driver package is test-signed (development only): pass --dev or set LEMON_GALAXY_DEV_DRIVER=1 (imports the test certificate into the machine's Root and TrustedPublisher stores)."
            )
            .into());
        }
        let cer = pkg.join(TEST_CERT).to_string_lossy().to_string();
        for store in ["Root", "TrustedPublisher"] {
            certutil(&["-f", "-addstore", store, &cer])?;
        }
        notes.push(t!("已信任测试证书（受信任的根、受信任的发布者）", "Test certificate trusted (Root, TrustedPublisher)").to_string());
    }
    // 驱动宿主（LOCAL SERVICE）写日志的目录
    let pd = std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\ProgramData"));
    let logs = pd.join("LemonGalaxy").join("Logs");
    let _ = std::fs::create_dir_all(&logs);
    let _ = std::process::Command::new("icacls").arg(&logs).args(["/grant", "*S-1-5-19:(OI)(CI)M", "/q"]).output();

    let (_, existing) = our_devices()?;
    let created = if existing.is_empty() {
        let id = create_root_device(&inf)?;
        notes.push(format!("{} {id}", t!("已建设备", "Created device")));
        Some(id)
    } else {
        notes.push(format!("{} {}", t!("更新已有设备", "Updating device"), existing.iter().map(|x| x.1.as_str()).collect::<Vec<_>>().join(", ")));
        None
    };
    let mut reboot = windows::core::BOOL(0);
    let r = unsafe { UpdateDriverForPlugAndPlayDevicesW(None, PCWSTR(wide(HARDWARE_ID).as_ptr()), PCWSTR(wide(&inf.to_string_lossy()).as_ptr()), INSTALLFLAG_FORCE, Some(&mut reboot)) };
    if let Err(e) = r {
        if created.is_some() {
            let _ = remove_devices();
        }
        return Err(format!("{}：{e}（0x{:08X}）", t!("安装驱动失败", "Driver installation failed"), e.code().0 as u32));
    }
    if reboot.as_bool() {
        notes.push(t!("需要重启后生效", "A reboot is required").into());
    }
    // 驱动库里只留正在用的那份
    if let Ok((set, devs)) = our_devices() {
        let in_use: Vec<String> = devs.iter().filter_map(|(d, _)| inf_in_use(&set, d)).map(|s| s.to_ascii_lowercase()).collect();
        for old in our_oem_infs() {
            if !in_use.contains(&old.to_ascii_lowercase()) && delete_oem_inf(&old) {
                notes.push(format!("{} {old}", t!("删掉旧驱动包", "Deleted old package")));
            }
        }
        notes.push(format!("{} {}", t!("驱动包", "Driver package"), in_use.join(", ")));
    }
    if wait_interface(Duration::from_secs(20)) {
        notes.push(t!("驱动已就绪", "Driver is ready").into());
        Ok(notes.join("\n"))
    } else {
        Err(format!("{}\n{}", notes.join("\n"), t!("驱动装上了但接口没有出现（看设备管理器里 LemonGalaxy Display 的状态）", "Driver installed but its interface did not appear (check LemonGalaxy Display in Device Manager)")))
    }
}

/// 卸载：删设备、删驱动包、删测试证书。
pub fn uninstall() -> Result<String, String> {
    if !galaxy_host::sysuser::is_admin() {
        return Err(t!("需要以管理员身份运行。", "Please run as administrator.").into());
    }
    let mut notes = Vec::new();
    for id in remove_devices()? {
        notes.push(format!("{} {id}", t!("已删设备", "Removed device")));
    }
    for inf in our_oem_infs() {
        if delete_oem_inf(&inf) {
            notes.push(format!("{} {inf}", t!("已删驱动包", "Deleted package")));
        } else {
            notes.push(format!("{} {inf}", t!("删不掉驱动包", "Cannot delete package")));
        }
    }
    for store in ["Root", "TrustedPublisher"] {
        if certutil(&["-delstore", store, TEST_CERT_SUBJECT]).is_ok() {
            notes.push(format!("{} {store}", t!("已删测试证书：", "Removed test certificate from")));
        }
    }
    if notes.is_empty() {
        notes.push(t!("没有安装虚拟屏驱动。", "Display driver is not installed.").into());
    }
    Ok(notes.join("\n"))
}

/// 命令行入口。
pub fn install_cmd(rest: &[String]) -> i32 {
    let pkg = rest.iter().find(|a| !a.starts_with("--")).map(PathBuf::from).or_else(package_next_to_exe);
    let Some(pkg) = pkg else {
        eprintln!("{}", t!("找不到驱动包：在命令后写驱动包目录（里面有 LemonGalaxyDisplay.inf），或放在程序旁边的 driver 目录。", "Driver package not found: pass its directory (containing LemonGalaxyDisplay.inf) or put it in a `driver` folder next to the program."));
        return 1;
    };
    match install(&pkg, dev_flag(rest)) {
        Ok(s) => {
            println!("{s}");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

pub fn uninstall_cmd() -> i32 {
    match uninstall() {
        Ok(s) => {
            println!("{s}");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// install-service 时：程序旁边有驱动包就复制到安装目录并安装（测试包要开发标志）。失败只提示，不影响服务安装。
pub fn install_with_service(target_dir: &Path) {
    let Some(src) = package_next_to_exe() else { return };
    let dst = target_dir.join("driver");
    if src != dst {
        let _ = std::fs::create_dir_all(&dst);
        if let Ok(rd) = std::fs::read_dir(&src) {
            for e in rd.flatten() {
                if e.path().is_file() {
                    let _ = std::fs::copy(e.path(), dst.join(e.file_name()));
                }
            }
        }
    }
    if is_test_package(&dst) && !dev_flag(&[]) {
        println!("{}", t!("程序旁边的虚拟屏驱动是测试签名的，没有安装（开发机上设 LEMON_GALAXY_DEV_DRIVER=1 再装服务，或运行 install-display-driver --dev）。", "The bundled display driver is test-signed and was not installed (set LEMON_GALAXY_DEV_DRIVER=1 or run install-display-driver --dev on development machines)."));
        return;
    }
    match install(&dst, dev_flag(&[])) {
        Ok(_) => println!("{}", t!("已安装虚拟屏驱动。", "Display driver installed.")),
        Err(e) => eprintln!("{}：{e}", t!("虚拟屏驱动没装上（远程桌面不受影响，只是不能加虚拟屏）", "Display driver not installed (remote desktop still works, without virtual displays)")),
    }
}
