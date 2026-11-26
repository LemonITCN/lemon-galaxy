//! 路径与版本、被控服务的状态、安装 / 卸载 / 启动服务（docs/spec.md 3.0.2，与 WPF 版 Setup/Setup.cs 同一套做法）。
//!
//! 界面以普通权限运行，这些都在以管理员身份运行的子命令里做（`elevation.rs`）：
//!   --install [--no-autostart] [--no-tray]   复制到 Program Files\LemonGalaxy\Worker、装服务、所有用户登录后显示托盘、
//!                                             开始菜单、「添加或删除程序」里的卸载入口
//!   --uninstall                               停掉并删除服务、删除程序文件与入口（账号与设置保留）
//!   --start-service                           启动已安装的服务

use crate::settings::{log_error, log_info};
use crate::tr;
use crate::win::{from_wide, pcw, wide};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::Registry::*;
use windows::Win32::System::Services::*;
use windows::Win32::System::Threading::{GetCurrentProcessId, OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE};
use windows::Win32::UI::Shell::SHGetFolderPathW;

pub const SERVICE_NAME: &str = "LemonGalaxyWorker";
pub const REPO_URL: &str = "https://github.com/LemonITCN/lemon-galaxy";
pub const LICENSE_URL: &str = "https://github.com/LemonITCN/lemon-galaxy/blob/main/LICENSE";
const UNINSTALL_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\LemonGalaxyWorker";
const RUN_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "LemonGalaxyWorker";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ---------------- 路径与版本 ----------------

const CSIDL_COMMON_PROGRAMS: i32 = 0x17;
const CSIDL_LOCAL_APPDATA: i32 = 0x1c;
const CSIDL_COMMON_APPDATA: i32 = 0x23;
const CSIDL_PROGRAM_FILES: i32 = 0x26;

/// 系统文件夹（SHGetFolderPathW，XP 起都有）
pub fn folder(csidl: i32) -> Option<PathBuf> {
    let mut buf = [0u16; 260];
    unsafe { SHGetFolderPathW(None, csidl, None, 0, &mut buf).ok()? };
    let s = from_wide(&buf);
    (!s.is_empty()).then(|| PathBuf::from(s))
}

pub fn exe_path() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("LemonGalaxyWorker.exe"))
}

pub fn base_dir() -> PathBuf {
    exe_path().parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."))
}

/// 安装位置：与 `lemon-galaxy-worker install-service` 复制到的目录相同（按环境变量 ProgramFiles，同一位数的程序得到同一个目录）
pub fn install_dir() -> PathBuf {
    let pf = std::env::var_os("ProgramFiles").map(PathBuf::from).or_else(|| folder(CSIDL_PROGRAM_FILES)).unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
    pf.join("LemonGalaxy").join("Worker")
}

pub fn installed_exe() -> PathBuf {
    install_dir().join("LemonGalaxyWorker.exe")
}

/// 被控服务的数据目录（账号、设置、日志），卸载时保留
pub fn service_data_dir() -> PathBuf {
    folder(CSIDL_COMMON_APPDATA).unwrap_or_else(|| PathBuf::from(r"C:\ProgramData")).join("LemonGalaxy").join("worker")
}

/// 界面自己的设置与日志（每个 Windows 用户一份）
pub fn ui_data_dir() -> PathBuf {
    folder(CSIDL_LOCAL_APPDATA).or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from)).unwrap_or_else(std::env::temp_dir).join("LemonGalaxy").join("worker-ui")
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches('\\').replace('/', "\\").to_lowercase();
    norm(a) == norm(b)
}

pub fn running_installed() -> bool {
    same_path(&base_dir(), &install_dir())
}

pub fn app_version() -> &'static str {
    galaxy_proto::APP_VERSION
}

/// 已安装那份界面的版本（没装为 None）
pub fn installed_version() -> Option<String> {
    let p = installed_exe();
    if !p.exists() {
        return None;
    }
    file_version(&p).or_else(|| Some(String::new()))
}

/// exe 的产品版本（版本信息资源里的 ProductVersion，去掉 + 后面的部分）
pub fn file_version(p: &Path) -> Option<String> {
    let w = wide(&p.to_string_lossy());
    unsafe {
        let n = GetFileVersionInfoSizeW(pcw(&w), None);
        if n == 0 {
            return None;
        }
        let mut buf = vec![0u8; n as usize];
        GetFileVersionInfoW(pcw(&w), None, n, buf.as_mut_ptr() as _).ok()?;
        let mut ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        let q = wide(r"\VarFileInfo\Translation");
        let tag = if VerQueryValueW(buf.as_ptr() as _, pcw(&q), &mut ptr, &mut len).as_bool() && len >= 4 {
            let t = std::slice::from_raw_parts(ptr as *const u16, 2);
            format!("{:04x}{:04x}", t[0], t[1])
        } else {
            "040904b0".to_string()
        };
        let q = wide(&format!(r"\StringFileInfo\{tag}\ProductVersion"));
        if !VerQueryValueW(buf.as_ptr() as _, pcw(&q), &mut ptr, &mut len).as_bool() || len == 0 {
            return None;
        }
        let s = from_wide(std::slice::from_raw_parts(ptr as *const u16, len as usize));
        Some(s.split('+').next().unwrap_or("").trim().to_string())
    }
}

/// 系统名称（「关于」、概览里显示）
pub fn os_name() -> String {
    let k = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let name = reg_get_sz(HKEY_LOCAL_MACHINE, k, "ProductName", false).unwrap_or_else(|| "Windows".into());
    let sp = reg_get_sz(HKEY_LOCAL_MACHINE, k, "CSDVersion", false).unwrap_or_default();
    let build = reg_get_sz(HKEY_LOCAL_MACHINE, k, "CurrentBuildNumber", false).or_else(|| reg_get_sz(HKEY_LOCAL_MACHINE, k, "CurrentBuild", false)).unwrap_or_default();
    let disp = reg_get_sz(HKEY_LOCAL_MACHINE, k, "DisplayVersion", false).unwrap_or_default();
    let mut name = name;
    if build.parse::<u32>().unwrap_or(0) >= 22000 {
        name = name.replace("Windows 10", "Windows 11");
    }
    let mut s = name;
    for extra in [disp, sp] {
        if !extra.is_empty() {
            s.push(' ');
            s.push_str(&extra);
        }
    }
    if !build.is_empty() {
        s.push_str(&format!(" ({build})"));
    }
    s
}

// ---------------- 注册表 ----------------

/// HKLM 下按 64 位视图读写（32 位程序跑在 64 位系统上时，与 WPF 版、64 位版写在同一处）
fn view(root: HKEY) -> REG_SAM_FLAGS {
    if root == HKEY_LOCAL_MACHINE && crate::win::compat::is_wow64() {
        KEY_WOW64_64KEY
    } else {
        REG_SAM_FLAGS(0)
    }
}

pub fn reg_get_sz(root: HKEY, path: &str, name: &str, wow: bool) -> Option<String> {
    let p = wide(path);
    let n = wide(name);
    let sam = if wow { KEY_READ | view(root) } else { KEY_READ };
    unsafe {
        let mut k = HKEY::default();
        if RegOpenKeyExW(root, pcw(&p), None, sam, &mut k) != ERROR_SUCCESS {
            return None;
        }
        let mut ty = REG_VALUE_TYPE::default();
        let mut len = 0u32;
        let mut out = None;
        if RegQueryValueExW(k, pcw(&n), None, Some(&mut ty), None, Some(&mut len)) == ERROR_SUCCESS && (ty == REG_SZ || ty == REG_EXPAND_SZ) {
            let mut buf = vec![0u16; len as usize / 2 + 1];
            if RegQueryValueExW(k, pcw(&n), None, None, Some(buf.as_mut_ptr() as *mut u8), Some(&mut len)) == ERROR_SUCCESS {
                out = Some(from_wide(&buf));
            }
        }
        let _ = RegCloseKey(k);
        out
    }
}

fn reg_open_write(root: HKEY, path: &str) -> Result<HKEY, String> {
    let p = wide(path);
    let mut k = HKEY::default();
    let r = unsafe { RegCreateKeyExW(root, pcw(&p), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE | KEY_READ | view(root), None, &mut k, None) };
    if r != ERROR_SUCCESS {
        return Err(format!("{path}: {}", r.to_hresult().message()));
    }
    Ok(k)
}

pub fn reg_set_sz(root: HKEY, path: &str, name: &str, value: &str) -> Result<(), String> {
    let k = reg_open_write(root, path)?;
    let n = wide(name);
    let v = wide(value);
    let bytes = unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2) };
    let r = unsafe { RegSetValueExW(k, pcw(&n), None, REG_SZ, Some(bytes)) };
    unsafe {
        let _ = RegCloseKey(k);
    }
    if r != ERROR_SUCCESS {
        return Err(format!("{path}\\{name}: {}", r.to_hresult().message()));
    }
    Ok(())
}

pub fn reg_set_dword(root: HKEY, path: &str, name: &str, value: u32) -> Result<(), String> {
    let k = reg_open_write(root, path)?;
    let n = wide(name);
    let r = unsafe { RegSetValueExW(k, pcw(&n), None, REG_DWORD, Some(&value.to_le_bytes())) };
    unsafe {
        let _ = RegCloseKey(k);
    }
    if r != ERROR_SUCCESS {
        return Err(format!("{path}\\{name}"));
    }
    Ok(())
}

pub fn reg_delete_value(root: HKEY, path: &str, name: &str) {
    let p = wide(path);
    let n = wide(name);
    unsafe {
        let mut k = HKEY::default();
        if RegOpenKeyExW(root, pcw(&p), None, KEY_SET_VALUE | view(root), &mut k) == ERROR_SUCCESS {
            let _ = RegDeleteValueW(k, pcw(&n));
            let _ = RegCloseKey(k);
        }
    }
}

/// 删一个没有子键的键。64 位视图要 RegDeleteKeyExW（XP 32 位没有，运行时找；那里也没有 64 位视图）
fn reg_delete_key(root: HKEY, path: &str) {
    let p = wide(path);
    let wow = view(root);
    unsafe {
        if wow.0 != 0 {
            type F = unsafe extern "system" fn(HKEY, PCWSTR, u32, u32) -> u32;
            let m = windows::Win32::System::LibraryLoader::GetModuleHandleW(windows::core::w!("advapi32.dll"));
            if let Ok(m) = m {
                if let Some(f) = windows::Win32::System::LibraryLoader::GetProcAddress(m, windows::core::s!("RegDeleteKeyExW")) {
                    let f: F = std::mem::transmute(f);
                    f(root, pcw(&p), wow.0, 0);
                    return;
                }
            }
        }
        let _ = RegDeleteKeyW(root, pcw(&p));
    }
}

/// 安装程序写了「所有用户登录后显示托盘」
pub fn machine_tray_entry() -> bool {
    reg_get_sz(HKEY_LOCAL_MACHINE, RUN_KEY, RUN_VALUE, true).is_some()
}

/// 本用户登录后显示托盘：装好的程序由 HKLM\…\Run 对所有用户启动（--tray），本用户关掉时启动后马上退出（设置里记着）；
/// 没装（直接从解压目录运行）时写本用户的 HKCU\…\Run。
pub fn set_user_tray(settings: &mut crate::settings::AppSettings, on: bool) {
    settings.tray_at_login = on;
    settings.save();
    let r = if on && !machine_tray_entry() {
        reg_set_sz(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, &format!("\"{}\" --tray", exe_path().display()))
    } else {
        reg_delete_value(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
        Ok(())
    };
    if let Err(e) = r {
        log_error(&format!("改登录启动项失败：{e}"));
    }
}

// ---------------- 服务状态 ----------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ServiceState {
    Unknown,
    NotInstalled,
    Stopped,
    Starting,
    Running,
}

struct Sc(SC_HANDLE);

impl Drop for Sc {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseServiceHandle(self.0);
        }
    }
}

fn open_service(access: u32) -> Result<Sc, windows::core::Error> {
    unsafe {
        let m = Sc(OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT)?);
        let n = wide(SERVICE_NAME);
        Ok(Sc(OpenServiceW(m.0, pcw(&n), access)?))
    }
}

pub fn query_service() -> ServiceState {
    match open_service(SERVICE_QUERY_STATUS) {
        Ok(s) => {
            let mut st = SERVICE_STATUS::default();
            if unsafe { QueryServiceStatus(s.0, &mut st) }.is_err() {
                return ServiceState::Unknown;
            }
            match st.dwCurrentState {
                SERVICE_RUNNING => ServiceState::Running,
                SERVICE_START_PENDING | SERVICE_CONTINUE_PENDING => ServiceState::Starting,
                _ => ServiceState::Stopped,
            }
        }
        // ERROR_SERVICE_DOES_NOT_EXIST = 1060
        Err(e) if e.code() == windows::Win32::Foundation::ERROR_SERVICE_DOES_NOT_EXIST.to_hresult() => ServiceState::NotInstalled,
        Err(_) => ServiceState::Unknown,
    }
}

fn wait_state(s: &Sc, want: SERVICE_STATUS_CURRENT_STATE, secs: u64) -> bool {
    let until = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < until {
        let mut st = SERVICE_STATUS::default();
        if unsafe { QueryServiceStatus(s.0, &mut st) }.is_err() {
            return false;
        }
        if st.dwCurrentState == want {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    false
}

fn stop_service() {
    if let Ok(s) = open_service(SERVICE_STOP | SERVICE_QUERY_STATUS) {
        let mut st = SERVICE_STATUS::default();
        if unsafe { ControlService(s.0, SERVICE_CONTROL_STOP, &mut st) }.is_ok() {
            wait_state(&s, SERVICE_STOPPED, 20);
        }
    }
}

// ---------------- 以管理员身份运行的部分 ----------------

const FILES: [&str; 3] = ["LemonGalaxyWorker.exe", "lemon-galaxy-worker.exe", "LICENSE.txt"];

/// `--install`。返回退出码；失败原因写进 `result`
pub fn install_main(autostart: bool, tray: bool, result: Option<&str>) -> i32 {
    match install(autostart, tray) {
        Ok(()) => 0,
        Err(e) => {
            log_error(&format!("安装失败：{e}"));
            crate::elevation::write_result(result, &e);
            1
        }
    }
}

fn install(autostart: bool, tray: bool) -> Result<(), String> {
    let src = base_dir();
    let dst = install_dir();
    std::fs::create_dir_all(&dst).map_err(|e| format!("{}: {e}", dst.display()))?;
    if !same_path(&src, &dst) {
        stop_service();
        for f in FILES {
            let from = src.join(f);
            if from.exists() {
                copy_replacing(&from, &dst.join(f))?;
            }
        }
        let res = src.join("Resources");
        if res.is_dir() {
            copy_dir(&res, &dst.join("Resources"))?;
        }
    }
    // 服务：install-service 发现程序已在 Program Files 时不再复制，只建 / 改服务并启动（XP 上顺带装虚拟屏驱动）
    let worker = dst.join("lemon-galaxy-worker.exe");
    if !worker.exists() {
        return Err(tr!("找不到 lemon-galaxy-worker.exe，请重新下载完整的安装包。", "lemon-galaxy-worker.exe was not found. Please download the full package again.").into());
    }
    let (code, out) = run_hidden(&worker, &["install-service"]);
    if code != 0 {
        return Err(if out.is_empty() { format!("install-service: {code}") } else { out });
    }
    if !autostart {
        if let Err(e) = set_demand_start() {
            log_error(&format!("改服务启动类型失败：{e}"));
        }
    }
    let exe = dst.join("LemonGalaxyWorker.exe");
    let exe_s = exe.display().to_string();
    if tray {
        reg_set_sz(HKEY_LOCAL_MACHINE, RUN_KEY, RUN_VALUE, &format!("\"{exe_s}\" --tray"))?;
    } else {
        reg_delete_value(HKEY_LOCAL_MACHINE, RUN_KEY, RUN_VALUE);
    }
    let lnk = create_shortcut(&exe);
    let k = UNINSTALL_KEY;
    let m = HKEY_LOCAL_MACHINE;
    if let Some(l) = &lnk {
        reg_set_sz(m, k, "LemonGalaxyShortcut", &l.display().to_string())?;
    }
    reg_set_sz(m, k, "DisplayName", tr!("LemonGalaxy 被控端", "LemonGalaxy Worker"))?;
    reg_set_sz(m, k, "DisplayVersion", app_version())?;
    reg_set_sz(m, k, "Publisher", "LemonIT.CN")?;
    reg_set_sz(m, k, "InstallLocation", &dst.display().to_string())?;
    reg_set_sz(m, k, "DisplayIcon", &format!("{exe_s},0"))?;
    reg_set_sz(m, k, "UninstallString", &format!("\"{exe_s}\" --uninstall"))?;
    reg_set_sz(m, k, "URLInfoAbout", REPO_URL)?;
    reg_set_dword(m, k, "NoModify", 1)?;
    reg_set_dword(m, k, "NoRepair", 1)?;
    reg_set_dword(m, k, "EstimatedSize", (dir_size(&dst) / 1024) as u32)?;
    log_info(&format!("已安装到 {}", dst.display()));
    Ok(())
}

fn set_demand_start() -> Result<(), String> {
    let s = open_service(SERVICE_CHANGE_CONFIG).map_err(|e| e.message())?;
    unsafe { ChangeServiceConfigW(s.0, ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE), SERVICE_DEMAND_START, SERVICE_ERROR(SERVICE_NO_CHANGE), PCWSTR::null(), PCWSTR::null(), None, PCWSTR::null(), PCWSTR::null(), PCWSTR::null(), PCWSTR::null()) }.map_err(|e| e.message())
}

/// `--start-service`
pub fn start_service_main(result: Option<&str>) -> i32 {
    let r = (|| -> Result<(), String> {
        let s = open_service(SERVICE_START | SERVICE_QUERY_STATUS).map_err(|e| e.message())?;
        let mut st = SERVICE_STATUS::default();
        unsafe { QueryServiceStatus(s.0, &mut st) }.map_err(|e| e.message())?;
        if st.dwCurrentState != SERVICE_RUNNING {
            unsafe { StartServiceW(s.0, None) }.map_err(|e| e.message())?;
            if !wait_state(&s, SERVICE_RUNNING, 20) {
                return Err(tr!("服务没有在 20 秒内启动。", "The service did not start within 20 seconds.").into());
            }
        }
        Ok(())
    })();
    match r {
        Ok(()) => 0,
        Err(e) => {
            crate::elevation::write_result(result, &e);
            1
        }
    }
}

/// `--uninstall`（已确认、已是管理员）。`keep`：不关的界面进程（发起卸载的那个，它自己提示后退出）
pub fn uninstall_main(result: Option<&str>, keep: u32) -> i32 {
    let r = (|| -> Result<(), String> {
        // 别的用户、别的会话里开着的界面一起关掉（文件要删）
        kill_others("LemonGalaxyWorker.exe", keep);
        let dst = install_dir();
        let worker = dst.join("lemon-galaxy-worker.exe");
        if worker.exists() {
            let (code, out) = run_hidden(&worker, &["uninstall-service"]);
            log_info(&format!("uninstall-service → {code} {out}"));
        } else {
            delete_service();
        }
        reg_delete_value(HKEY_LOCAL_MACHINE, RUN_KEY, RUN_VALUE);
        reg_delete_value(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
        if let Some(l) = reg_get_sz(HKEY_LOCAL_MACHINE, UNINSTALL_KEY, "LemonGalaxyShortcut", true) {
            if l.to_ascii_lowercase().ends_with(".lnk") {
                let _ = std::fs::remove_file(&l);
            }
        }
        reg_delete_key(HKEY_LOCAL_MACHINE, UNINSTALL_KEY);
        if dst.exists() {
            remove_dir_later(&dst);
        }
        log_info("已卸载");
        Ok(())
    })();
    match r {
        Ok(()) => 0,
        Err(e) => {
            crate::elevation::write_result(result, &e);
            1
        }
    }
}

fn delete_service() {
    stop_service();
    if let Ok(s) = open_service(0x10000 /* DELETE */) {
        unsafe {
            let _ = DeleteService(s.0);
        }
    }
}

/// 自己（和发起卸载的界面）还在运行，删不掉：交给后台的 cmd 每 2 秒试一次，删掉为止（最多约 10 分钟，
/// 界面上的「已卸载」提示框开着时程序文件还占着）。不登记「重启时删除」：卸载后不重启又装回来时会把新装的删掉
fn remove_dir_later(dir: &Path) {
    let d = dir.display().to_string();
    let sys = std::env::var("SystemRoot").map(|r| PathBuf::from(r).join("System32")).unwrap_or_else(|_| PathBuf::from(r"C:\Windows\System32"));
    let cmd = sys.join("cmd.exe");
    let script = format!("for /l %i in (1,1,300) do @(if exist \"{d}\" (rd /s /q \"{d}\" 2>nul & ping -n 3 127.0.0.1 >nul))");
    let spawned = std::process::Command::new(&cmd).arg("/c").raw_arg(&script).current_dir(&sys).creation_flags(CREATE_NO_WINDOW).spawn();
    if let Err(e) = spawned {
        log_error(&format!("启动删除程序目录的命令失败：{e}"));
    }
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    out.push(dir.to_path_buf());
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_files(&p, out);
            } else {
                out.push(p);
            }
        }
    }
}

fn dir_size(dir: &Path) -> u64 {
    let mut files = vec![];
    collect_files(dir, &mut files);
    files.iter().filter_map(|f| std::fs::metadata(f).ok()).filter(|m| m.is_file()).map(|m| m.len()).sum()
}

/// 目标文件正被运行（旧版界面开着、会话代理还没退）时先改名挪开，运行中的程序照常跑完；下次启动时删（`clean_old_files`）
fn copy_replacing(from: &Path, to: &Path) -> Result<(), String> {
    if std::fs::copy(from, to).is_ok() {
        return Ok(());
    }
    let name = to.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let old = if name.to_ascii_lowercase().ends_with(".exe") { to.with_file_name(format!("{}.old.exe", &name[..name.len() - 4])) } else { to.with_file_name(format!("{name}.old")) };
    let _ = std::fs::remove_file(&old);
    std::fs::rename(to, &old).map_err(|e| format!("{}: {e}", to.display()))?;
    std::fs::copy(from, to).map(|_| ()).map_err(|e| format!("{}: {e}", to.display()))
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for e in std::fs::read_dir(from).map_err(|e| e.to_string())?.flatten() {
        let p = e.path();
        let t = to.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &t)?;
        } else {
            copy_replacing(&p, &t)?;
        }
    }
    Ok(())
}

/// 上次升级挪开的旧文件（启动时顺手删）
pub fn clean_old_files() {
    if let Ok(rd) = std::fs::read_dir(base_dir()) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_ascii_lowercase();
            if n.contains(".old") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// 不弹窗口运行，返回 (退出码, 标准输出 + 标准错误)
pub fn run_hidden(exe: &Path, args: &[&str]) -> (i32, String) {
    let out = std::process::Command::new(exe).args(args).env("LEMON_GALAXY_LANG", crate::i18n::lang_tag()).current_dir(exe.parent().unwrap_or(Path::new("."))).creation_flags(CREATE_NO_WINDOW).output();
    match out {
        Ok(o) => {
            let text = format!("{}\n{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).trim().to_string();
            let code = o.status.code().unwrap_or(1);
            log_info(&format!("{} {} → {code} {text}", exe.display(), args.join(" ")));
            (code, text)
        }
        Err(e) => (1, format!("{}: {e}", exe.display())),
    }
}

/// 结束同名的其他进程（自己与 `keep` 除外）
fn kill_others(name: &str, keep: u32) {
    let me = unsafe { GetCurrentProcessId() };
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return };
        let mut pe = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut pe).is_ok();
        while ok {
            let exe = from_wide(&pe.szExeFile);
            let pid = pe.th32ProcessID;
            if exe.eq_ignore_ascii_case(name) && pid != me && pid != keep {
                if let Ok(h) = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, false, pid) {
                    let _ = TerminateProcess(h, 0);
                    WaitForSingleObject(h, 3000);
                    let _ = CloseHandle(h);
                }
            }
            ok = Process32NextW(snap, &mut pe).is_ok();
        }
        let _ = CloseHandle(snap);
    }
}

/// 开始菜单（所有用户）里的快捷方式，名字按安装时的界面语言；返回路径，卸载时按它删
fn create_shortcut(exe: &Path) -> Option<PathBuf> {
    use windows::core::Interface;
    use windows::Win32::System::Com::{CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    let dir = folder(CSIDL_COMMON_PROGRAMS)?;
    let path = dir.join(format!("{}.lnk", tr!("LemonGalaxy 被控端", "LemonGalaxy Worker")));
    let r = (|| -> windows::core::Result<()> {
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            let e = wide(&exe.display().to_string());
            let d = wide(&exe.parent().map(|p| p.display().to_string()).unwrap_or_default());
            let desc = wide(tr!("LemonGalaxy 被控端", "LemonGalaxy Worker"));
            link.SetPath(pcw(&e))?;
            link.SetWorkingDirectory(pcw(&d))?;
            link.SetIconLocation(pcw(&e), 0)?;
            link.SetDescription(pcw(&desc))?;
            let pf: IPersistFile = link.cast()?;
            let p = wide(&path.display().to_string());
            pf.Save(pcw(&p), true)?;
        }
        Ok(())
    })();
    match r {
        Ok(()) => Some(path),
        Err(e) => {
            log_error(&format!("建开始菜单快捷方式失败：{}", e.message()));
            None
        }
    }
}
