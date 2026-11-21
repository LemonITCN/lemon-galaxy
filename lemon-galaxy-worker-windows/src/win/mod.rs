mod agent;
mod audio;
mod capture;
mod clipboard;
mod compat;
mod encode;
mod input;
mod ipc;
mod platform;
mod privacy;
mod service;

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

    fn host_platform(&self) -> Option<Arc<dyn HostPlatform>> {
        Some(platform::WinPlatform::new())
    }

    fn secure_data_dir(&self, dir: &Path) {
        let _ = std::fs::create_dir_all(dir);
        // 系统数据目录只给 SYSTEM 与管理员组（用 SID，与系统语言无关）
        if galaxy_host::sysuser::is_admin() && dir.starts_with(self.default_dir(true)) {
            let _ = std::process::Command::new("icacls")
                .arg(dir)
                .args(["/inheritance:r", "/grant:r", "*S-1-5-18:(OI)(CI)F", "*S-1-5-32-544:(OI)(CI)F", "/q"])
                .output();
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
            "selftest-capture" => Some(agent::selftest(_rest)),
            _ => None,
        }
    }

    fn extra_help(&self) -> Vec<(&'static str, &'static str, &'static str)> {
        vec![("selftest-capture", "检查截屏与编码是否正常（--bench 测试本机每种编码器，--dump 目录 存下码流）", "Check screen capture and encoding (--bench: test every encoder, --dump DIR: save streams)")]
    }
}

pub fn main() {
    // 上次自动更新留下的旧程序
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old.exe"));
    }
    galaxy_worker::main(Arc::new(WindowsOs));
}
