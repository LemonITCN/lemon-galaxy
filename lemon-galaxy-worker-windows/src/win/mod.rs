mod agent;
mod audio;
mod capture;
mod clipboard;
mod clipole;
mod compat;
mod driver;
mod driver_xp;
mod encode;
mod input;
mod ipc;
mod platform;
mod privacy;
mod service;
mod vdisplay;
mod vdisplay_xp;
mod wavein;
#[cfg(lemon_galaxy_xp)]
mod xp;

use galaxy_host::platform::HostPlatform;
use galaxy_proto::Platform;
use galaxy_worker::WorkerOs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct WindowsOs;

impl WindowsOs {
    pub fn default_dir(&self, admin: bool) -> PathBuf {
        if admin {
            let pd = std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\ProgramData"));
            pd.join("LemonGalaxy").join("worker")
        } else {
            let la = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            la.join("LemonGalaxy").join("worker")
        }
    }
}

fn reg_value(name: &str) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};
    let key: Vec<u16> = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\0".encode_utf16().collect();
    let val: Vec<u16> = format!("{name}\0").encode_utf16().collect();
    unsafe {
        let mut buf = [0u16; 256];
        let mut len = (buf.len() * 2) as u32;
        if RegGetValueW(HKEY_LOCAL_MACHINE, PCWSTR(key.as_ptr()), PCWSTR(val.as_ptr()), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut len)).is_ok() {
            let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
            return Some(String::from_utf16_lossy(&buf[..n]));
        }
        let mut d = 0u32;
        let mut len = 4u32;
        if RegGetValueW(HKEY_LOCAL_MACHINE, PCWSTR(key.as_ptr()), PCWSTR(val.as_ptr()), RRF_RT_REG_DWORD, None, Some(&mut d as *mut u32 as _), Some(&mut len)).is_ok() {
            return Some(d.to_string());
        }
    }
    None
}

/// exe 的产品版本（版本信息资源里的 ProductVersion，去掉 + 后面的部分）；读不到返回 None
fn file_version(p: &Path) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
    let w: Vec<u16> = p.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
    unsafe {
        let n = GetFileVersionInfoSizeW(PCWSTR(w.as_ptr()), None);
        if n == 0 {
            return None;
        }
        let mut buf = vec![0u8; n as usize];
        GetFileVersionInfoW(PCWSTR(w.as_ptr()), None, n, buf.as_mut_ptr() as _).ok()?;
        let mut ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        let q: Vec<u16> = "\\VarFileInfo\\Translation\0".encode_utf16().collect();
        let tag = if VerQueryValueW(buf.as_ptr() as _, PCWSTR(q.as_ptr()), &mut ptr, &mut len).as_bool() && len >= 4 {
            let t = std::slice::from_raw_parts(ptr as *const u16, 2);
            format!("{:04x}{:04x}", t[0], t[1])
        } else {
            "040904b0".to_string()
        };
        let q: Vec<u16> = format!("\\StringFileInfo\\{tag}\\ProductVersion\0").encode_utf16().collect();
        if !VerQueryValueW(buf.as_ptr() as _, PCWSTR(q.as_ptr()), &mut ptr, &mut len).as_bool() || len == 0 {
            return None;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(ptr as *const u16, len as usize));
        let v = s.trim_end_matches('\0').split('+').next().unwrap_or("").trim().to_string();
        (!v.is_empty()).then_some(v)
    }
}

/// 改卸载项里的版本号（WPF 版界面按 64 位视图写、原生版按程序自己的位数写：两个视图里有就都改）
fn set_uninstall_version(version: &str) {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_SET_VALUE, KEY_WOW64_32KEY, KEY_WOW64_64KEY, REG_SZ};
    let key: Vec<u16> = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\LemonGalaxyWorker\0".encode_utf16().collect();
    let name: Vec<u16> = "DisplayVersion\0".encode_utf16().collect();
    let val: Vec<u16> = version.encode_utf16().chain(Some(0)).collect();
    let bytes = unsafe { std::slice::from_raw_parts(val.as_ptr() as *const u8, val.len() * 2) };
    for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
        unsafe {
            let mut h = HKEY::default();
            if RegOpenKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(key.as_ptr()), None, KEY_SET_VALUE | view, &mut h).is_ok() {
                let _ = RegSetValueExW(h, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes));
                let _ = RegCloseKey(h);
            }
        }
    }
}

impl WorkerOs for WindowsOs {
    fn platform(&self) -> Platform {
        Platform::Windows
    }

    fn os_version(&self) -> String {
        let build: u32 = reg_value("CurrentBuild").and_then(|b| b.parse().ok()).unwrap_or(0);
        let mut name = reg_value("ProductName").unwrap_or_else(|| "Windows".into());
        // Windows 11 的注册表里仍写 Windows 10
        if build >= 22000 {
            name = name.replace("Windows 10", "Windows 11");
        }
        let disp = reg_value("DisplayVersion").or_else(|| reg_value("ReleaseId")).unwrap_or_default();
        if disp.is_empty() {
            format!("{name} ({build})")
        } else {
            format!("{name} {disp} ({build})")
        }
    }

    fn default_data_dir(&self, admin: bool) -> PathBuf {
        self.default_dir(admin)
    }

    fn default_system_user(&self) -> String {
        galaxy_host::sysuser::SysUser::current().display_name()
    }

    fn superuser_name(&self) -> &'static str {
        "SYSTEM"
    }

    fn install_service(&self, dir: &Path) -> i32 {
        self.secure_data_dir(dir);
        service::install(dir)
    }

    fn uninstall_service(&self) -> i32 {
        service::uninstall()
    }

    fn update_target(&self) -> Option<PathBuf> {
        let installed = service::install_dir().join("lemon-galaxy-worker.exe");
        installed.exists().then_some(installed)
    }

    fn restart_service(&self) -> bool {
        service::restart()
    }

    fn stop_service(&self) -> bool {
        service::stop()
    }

    fn start_service(&self) -> bool {
        service::start()
    }

    /// 安装目录里有界面（WPF 版或老系统的原生版）就是装了桌面端：自动更新换整套
    fn desktop_install(&self) -> Option<galaxy_worker::upgrade::DesktopInstall> {
        let dir = service::install_dir();
        let gui = dir.join(galaxy_worker::upgrade::DESKTOP_GUI_EXE);
        if !gui.is_file() || !dir.join(galaxy_worker::upgrade::DESKTOP_CORE_EXE).is_file() {
            return None;
        }
        Some(galaxy_worker::upgrade::DesktopInstall::WindowsDir { gui_version: file_version(&gui), dir })
    }

    /// 「应用和功能」（「添加或删除程序」）里显示的版本号：界面安装时写的卸载项还在就改成新版本
    fn desktop_updated(&self, version: &str) {
        set_uninstall_version(version);
    }

    fn service_autostart(&self) -> Option<Option<bool>> {
        Some(service::autostart())
    }

    fn set_service_autostart(&self, on: bool) -> Result<(), String> {
        service::set_autostart(on)
    }

    fn host_platform(&self) -> Option<Arc<dyn HostPlatform>> {
        Some(platform::WinPlatform::new())
    }

    fn secure_data_dir(&self, dir: &Path) {
        let _ = std::fs::create_dir_all(dir);
        // 系统数据目录（账号令牌在里面）只给 SYSTEM 与管理员组
        if galaxy_host::sysuser::is_admin() && dir.starts_with(self.default_dir(true)) {
            if let Err(e) = galaxy_host::win::restrict_to_system_and_admins(dir) {
                tracing::warn!("收紧数据目录权限失败（{}）：{e}", dir.display());
            }
        }
    }

    fn ui_zh(&self) -> Option<bool> {
        if std::env::var("LEMON_GALAXY_LANG").is_ok() {
            return None;
        }
        let lang = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
        Some(lang & 0x3ff == 0x04)
    }

    fn extra_command(&self, cmd: &str, _rest: &[String], dir: &Path) -> Option<i32> {
        match cmd {
            "service" => Some(service::run_dispatcher(dir)),
            "__lg-session-agent" => Some(agent::main()),
            "__lg-clip-helper" => Some(clipole::helper_main()),
            "selftest-capture" => Some(agent::selftest(_rest)),
            "install-display-driver" => Some(driver::install_cmd(_rest)),
            "uninstall-display-driver" => Some(driver::uninstall_cmd()),
            _ => None,
        }
    }

    fn extra_help(&self) -> Vec<(&'static str, &'static str, &'static str)> {
        vec![
            ("selftest-capture", "检查截屏与编码是否正常（--bench 测试本机每种编码器，--dump 目录 存下码流）", "Check screen capture and encoding (--bench: test every encoder, --dump DIR: save streams)"),
            ("install-display-driver", "安装虚拟屏驱动 [--dev] [驱动包目录]（管理员；测试签名包要 --dev；XP 装程序自带的 XP 驱动）", "Install the virtual display driver [--dev] [package dir] (admin; --dev for test-signed packages; on XP installs the bundled XP driver)"),
            ("uninstall-display-driver", "卸载虚拟屏驱动（删设备、驱动包与测试证书）", "Remove the virtual display driver, its package and test certificate"),
        ]
    }
}

pub fn main() {
    // 上次自动更新挪开的旧程序（还在运行的删不掉，下次再删）
    if let Ok(exe) = std::env::current_exe() {
        galaxy_worker::upgrade::files::clean_old_of(&exe);
    }
    galaxy_worker::main(Arc::new(WindowsOs));
}
