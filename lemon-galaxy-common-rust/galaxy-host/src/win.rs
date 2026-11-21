//! Windows 专用：账号查询、以指定账号启动进程（服务以 SYSTEM 运行时）、作业对象、伪终端（ConPTY）、管道。
//!
//! 以指定账号运行的取令牌顺序：
//! 1. 该账号已登录在某个会话里：取会话令牌（WTSQueryUserToken）；
//! 2. 否则用 S4U 登录（不需要密码，只能访问本机资源，访问网络共享需要另行认证）；
//! 3. 管理员账号拿到受限令牌时换成完整令牌（与 Windows 自带 OpenSSH 一致，远程终端默认是提升后的）。

#![allow(clippy::missing_safety_doc, non_snake_case)]

use parking_lot::Mutex;
use std::ffi::c_void;
use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::ptr::{null, null_mut};
use std::sync::Arc;
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, LUID, WAIT_OBJECT_0};
use windows_sys::Win32::Security::Authentication::Identity::{
    KerbS4ULogon, LsaDeregisterLogonProcess, LsaFreeReturnBuffer, LsaLogonUser, LsaLookupAuthenticationPackage, LsaRegisterLogonProcess, MsV1_0S4ULogon, KERB_S4U_LOGON, LSA_STRING, LSA_UNICODE_STRING, MSV1_0_S4U_LOGON,
};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, AllocateLocallyUniqueId, DuplicateTokenEx, GetTokenInformation, LookupAccountNameW, LookupAccountSidW, LookupPrivilegeValueW, SecurityImpersonation, SetTokenInformation, TokenElevation, TokenElevationType,
    TokenLinkedToken, TokenPrimary, TokenSessionId, TokenUser, LUID_AND_ATTRIBUTES, QUOTA_LIMITS, SE_PRIVILEGE_ENABLED, SID_NAME_USE, TOKEN_ALL_ACCESS, TOKEN_ELEVATION, TOKEN_LINKED_TOKEN, TOKEN_PRIVILEGES, TOKEN_QUERY,
    TOKEN_SOURCE, TOKEN_USER, SECURITY_ATTRIBUTES, TOKEN_ADJUST_PRIVILEGES, TOKEN_DUPLICATE,
};
use windows_sys::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::RemoteDesktop::{WTSActive, WTSDomainName, WTSEnumerateSessionsW, WTSFreeMemory, WTSQuerySessionInformationW, WTSQueryUserToken, WTSUserName, WTS_SESSION_INFOW};
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess, GetExitCodeProcess, InitializeProcThreadAttributeList, OpenProcessToken, ResumeThread, TerminateProcess, UpdateProcThreadAttribute,
    WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, INFINITE, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};
use windows_sys::Win32::UI::Shell::{LoadUserProfileW, UnloadUserProfile, PROFILEINFOW};

const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x20002;
const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x20016;
const PI_NOUI: u32 = 1;
const SYSTEM_SID: &str = "S-1-5-18";

pub fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

unsafe fn from_wide_ptr(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
}

fn from_wide_buf(b: &[u16]) -> String {
    let n = b.iter().position(|c| *c == 0).unwrap_or(b.len());
    String::from_utf16_lossy(&b[..n])
}

/// 自动关闭的句柄。
pub struct Handle(pub HANDLE);
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Handle {
    pub fn raw(&self) -> HANDLE {
        self.0
    }
    pub fn valid(&self) -> bool {
        !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE
    }
    /// 交出所有权（不再关闭）。
    pub fn into_raw(self) -> HANDLE {
        let h = self.0;
        std::mem::forget(self);
        h
    }
    pub fn into_file(self) -> std::fs::File {
        unsafe { std::fs::File::from_raw_handle(self.into_raw() as _) }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        if self.valid() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

// ---------------- 账号 ----------------

#[derive(Clone, Debug)]
pub struct Account {
    pub name: String,
    pub domain: String,
    pub sid: String,
}

impl Account {
    pub fn is_system(&self) -> bool {
        self.sid == SYSTEM_SID
    }
    pub fn qualified(&self) -> String {
        if self.domain.is_empty() {
            self.name.clone()
        } else {
            format!("{}\\{}", self.domain, self.name)
        }
    }
}

unsafe fn sid_to_string(sid: *mut c_void) -> String {
    let mut p: *mut u16 = null_mut();
    if ConvertSidToStringSidW(sid, &mut p) == 0 {
        return String::new();
    }
    let s = from_wide_ptr(p);
    LocalFree(p as _);
    s
}

unsafe fn account_of_sid(sid: *mut c_void) -> Option<Account> {
    let mut name = [0u16; 256];
    let mut domain = [0u16; 256];
    let (mut nl, mut dl) = (name.len() as u32, domain.len() as u32);
    let mut use_: SID_NAME_USE = 0;
    if LookupAccountSidW(null(), sid, name.as_mut_ptr(), &mut nl, domain.as_mut_ptr(), &mut dl, &mut use_) == 0 {
        return None;
    }
    Some(Account { name: from_wide_buf(&name), domain: from_wide_buf(&domain), sid: sid_to_string(sid) })
}

/// 按名字查账号（「用户名」或「域\用户名」；SYSTEM 也可以）。
pub fn lookup_account(name: &str) -> Option<Account> {
    let n = wide(name);
    unsafe {
        let mut sid = vec![0u8; 256];
        let mut sl = sid.len() as u32;
        let mut domain = [0u16; 256];
        let mut dl = domain.len() as u32;
        let mut use_: SID_NAME_USE = 0;
        if LookupAccountNameW(null(), n.as_ptr(), sid.as_mut_ptr() as _, &mut sl, domain.as_mut_ptr(), &mut dl, &mut use_) == 0 {
            return None;
        }
        // 用 SID 反查，得到规范大小写的名字
        account_of_sid(sid.as_mut_ptr() as _)
    }
}

unsafe fn token_info(token: HANDLE, class: i32) -> Option<Vec<u8>> {
    let mut len = 0u32;
    GetTokenInformation(token, class, null_mut(), 0, &mut len);
    if len == 0 {
        return None;
    }
    let mut buf = vec![0u8; len as usize];
    if GetTokenInformation(token, class, buf.as_mut_ptr() as _, len, &mut len) == 0 {
        return None;
    }
    Some(buf)
}

fn process_token(access: u32) -> Option<Handle> {
    unsafe {
        let mut t: HANDLE = null_mut();
        if OpenProcessToken(GetCurrentProcess(), access, &mut t) == 0 {
            return None;
        }
        Some(Handle(t))
    }
}

/// 当前进程的账号。
pub fn current_account() -> Option<Account> {
    let t = process_token(TOKEN_QUERY)?;
    unsafe {
        let buf = token_info(t.raw(), TokenUser)?;
        let tu = &*(buf.as_ptr() as *const TOKEN_USER);
        account_of_sid(tu.User.Sid)
    }
}

pub fn is_system() -> bool {
    current_account().map(|a| a.is_system()).unwrap_or(false)
}

/// 当前进程是否以管理员身份（已提升）运行。
pub fn is_elevated() -> bool {
    let Some(t) = process_token(TOKEN_QUERY) else { return false };
    unsafe { token_info(t.raw(), TokenElevation).map(|b| (*(b.as_ptr() as *const TOKEN_ELEVATION)).TokenIsElevated != 0).unwrap_or(false) }
}

/// 账号的用户目录（注册表 ProfileList）。
pub fn profile_dir(sid: &str) -> Option<String> {
    if sid == SYSTEM_SID {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        return Some(format!("{root}\\System32\\config\\systemprofile"));
    }
    let out = std::process::Command::new("reg")
        .args(["query", &format!("HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\ProfileList\\{sid}"), "/v", "ProfileImagePath"])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    let line = s.lines().find(|l| l.contains("ProfileImagePath"))?;
    let v = line.split("REG_EXPAND_SZ").nth(1).or_else(|| line.split("REG_SZ").nth(1))?.trim().to_string();
    Some(expand_env(&v))
}

fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('%') {
            Some(j) => {
                let name = &after[..j];
                out.push_str(&std::env::var(name).unwrap_or_else(|_| format!("%{name}%")));
                rest = &after[j + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

// ---------------- 令牌 ----------------

fn enable_privilege(token: HANDLE, name: &str) -> bool {
    unsafe {
        let mut luid = LUID { LowPart: 0, HighPart: 0 };
        let n = wide(name);
        if LookupPrivilegeValueW(null(), n.as_ptr(), &mut luid) == 0 {
            return false;
        }
        let tp = TOKEN_PRIVILEGES { PrivilegeCount: 1, Privileges: [LUID_AND_ATTRIBUTES { Luid: luid, Attributes: SE_PRIVILEGE_ENABLED }] };
        AdjustTokenPrivileges(token, 0, &tp, 0, null_mut(), null_mut()) != 0 && io::Error::last_os_error().raw_os_error() != Some(1300)
    }
}

/// SYSTEM 服务启动其他账号的进程前打开需要的特权（只需一次）。
pub fn enable_service_privileges() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if let Some(t) = process_token(TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY) {
            for p in ["SeTcbPrivilege", "SeAssignPrimaryTokenPrivilege", "SeIncreaseQuotaPrivilege", "SeBackupPrivilege", "SeRestorePrivilege"] {
                let _ = enable_privilege(t.raw(), p);
            }
        }
    });
}

/// 以某账号运行用的令牌，析构时卸载用户配置。
pub struct UserToken {
    token: Handle,
    profile: HANDLE,
}
unsafe impl Send for UserToken {}
unsafe impl Sync for UserToken {}

impl UserToken {
    pub fn raw(&self) -> HANDLE {
        self.token.raw()
    }
}

impl Drop for UserToken {
    fn drop(&mut self) {
        if !self.profile.is_null() {
            unsafe {
                UnloadUserProfile(self.token.raw(), self.profile);
            }
        }
    }
}

unsafe fn to_primary(t: HANDLE) -> io::Result<Handle> {
    let mut out: HANDLE = null_mut();
    if DuplicateTokenEx(t, TOKEN_ALL_ACCESS, null(), SecurityImpersonation, TokenPrimary, &mut out) == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Handle(out))
}

/// 受限令牌（UAC 过滤）换成完整令牌。
unsafe fn elevate(t: Handle) -> Handle {
    // TokenElevationType: 1 Default, 2 Full, 3 Limited
    let limited = token_info(t.raw(), TokenElevationType).map(|b| *(b.as_ptr() as *const i32) == 3).unwrap_or(false);
    if !limited {
        return t;
    }
    match token_info(t.raw(), TokenLinkedToken) {
        Some(b) => {
            let linked = Handle((*(b.as_ptr() as *const TOKEN_LINKED_TOKEN)).LinkedToken);
            to_primary(linked.raw()).unwrap_or(t)
        }
        None => t,
    }
}

fn session_token(acc: &Account) -> Option<Handle> {
    unsafe {
        let mut p: *mut WTS_SESSION_INFOW = null_mut();
        let mut n = 0u32;
        if WTSEnumerateSessionsW(null_mut(), 0, 1, &mut p, &mut n) == 0 {
            return None;
        }
        let sessions = std::slice::from_raw_parts(p, n as usize).iter().map(|s| (s.SessionId, s.State)).collect::<Vec<_>>();
        WTSFreeMemory(p as _);
        // 活动会话优先
        let mut ordered = sessions.clone();
        ordered.sort_by_key(|(_, st)| if *st == WTSActive { 0 } else { 1 });
        for (sid, _) in ordered {
            let q = |class| {
                let mut b: *mut u16 = null_mut();
                let mut len = 0u32;
                if WTSQuerySessionInformationW(null_mut(), sid, class, &mut b, &mut len) == 0 {
                    return String::new();
                }
                let s = from_wide_ptr(b);
                WTSFreeMemory(b as _);
                s
            };
            let user = q(WTSUserName);
            if user.is_empty() || !user.eq_ignore_ascii_case(&acc.name) {
                continue;
            }
            let dom = q(WTSDomainName);
            if !acc.domain.is_empty() && !dom.is_empty() && !dom.eq_ignore_ascii_case(&acc.domain) {
                continue;
            }
            let mut t: HANDLE = null_mut();
            if WTSQueryUserToken(sid, &mut t) != 0 {
                let t = Handle(t);
                return to_primary(t.raw()).ok();
            }
        }
        None
    }
}

fn lsa_string(s: &'static [u8]) -> LSA_STRING {
    LSA_STRING { Length: s.len() as u16, MaximumLength: s.len() as u16, Buffer: s.as_ptr() as *mut u8 }
}

/// S4U 登录（无密码）。本机账号走 MSV1_0，域账号走 Kerberos。
fn s4u_token(acc: &Account) -> io::Result<Handle> {
    unsafe {
        let mut lsa: HANDLE = null_mut();
        let mut mode = 0u32;
        let proc_name = lsa_string(b"LemonGalaxy");
        let st = LsaRegisterLogonProcess(&proc_name, &mut lsa, &mut mode);
        if st != 0 {
            return Err(io::Error::other(format!("LsaRegisterLogonProcess 0x{st:08x}（需要以 SYSTEM 运行）")));
        }
        struct Lsa(HANDLE);
        impl Drop for Lsa {
            fn drop(&mut self) {
                unsafe {
                    LsaDeregisterLogonProcess(self.0);
                }
            }
        }
        let lsa = Lsa(lsa);
        let computer = std::env::var("COMPUTERNAME").unwrap_or_default();
        let local = acc.domain.is_empty() || acc.domain.eq_ignore_ascii_case(&computer) || acc.domain.eq_ignore_ascii_case("BUILTIN");
        let pkg_name = if local { lsa_string(b"MICROSOFT_AUTHENTICATION_PACKAGE_V1_0") } else { lsa_string(b"Kerberos") };
        let mut pkg = 0u32;
        let st = LsaLookupAuthenticationPackage(lsa.0, &pkg_name, &mut pkg);
        if st != 0 {
            return Err(io::Error::other(format!("LsaLookupAuthenticationPackage 0x{st:08x}")));
        }
        let user_w: Vec<u16> = acc.name.encode_utf16().collect();
        let dom_w: Vec<u16> = if local { ".".encode_utf16().collect() } else { acc.domain.encode_utf16().collect() };
        // 结构体后面紧跟字符串内容，整块交给 LSA
        let head = if local { std::mem::size_of::<MSV1_0_S4U_LOGON>() } else { std::mem::size_of::<KERB_S4U_LOGON>() };
        let total = head + (user_w.len() + dom_w.len()) * 2;
        let mut buf = vec![0u8; total];
        let base = buf.as_mut_ptr();
        let up = base.add(head) as *mut u16;
        std::ptr::copy_nonoverlapping(user_w.as_ptr(), up, user_w.len());
        let dp = up.add(user_w.len());
        std::ptr::copy_nonoverlapping(dom_w.as_ptr(), dp, dom_w.len());
        let us = LSA_UNICODE_STRING { Length: (user_w.len() * 2) as u16, MaximumLength: (user_w.len() * 2) as u16, Buffer: up };
        let ds = LSA_UNICODE_STRING { Length: (dom_w.len() * 2) as u16, MaximumLength: (dom_w.len() * 2) as u16, Buffer: dp };
        if local {
            let l = &mut *(base as *mut MSV1_0_S4U_LOGON);
            l.MessageType = MsV1_0S4ULogon;
            l.Flags = 0;
            l.UserPrincipalName = us;
            l.DomainName = ds;
        } else {
            let l = &mut *(base as *mut KERB_S4U_LOGON);
            l.MessageType = KerbS4ULogon;
            l.Flags = 0;
            l.ClientUpn = us;
            l.ClientRealm = ds;
        }
        let mut src: TOKEN_SOURCE = std::mem::zeroed();
        for (i, b) in b"LGalaxy\0".iter().enumerate() {
            src.SourceName[i] = *b as _;
        }
        AllocateLocallyUniqueId(&mut src.SourceIdentifier);
        let origin = lsa_string(b"LemonGalaxy");
        let mut profile: *mut c_void = null_mut();
        let mut profile_len = 0u32;
        let mut logon_id = LUID { LowPart: 0, HighPart: 0 };
        let mut token: HANDLE = null_mut();
        let mut quotas: QUOTA_LIMITS = std::mem::zeroed();
        let mut sub = 0i32;
        // 3 = Network（OpenSSH 的 S4U 也用这个登录类型）
        let st = LsaLogonUser(lsa.0, &origin, windows_sys::Win32::Security::Authentication::Identity::SECURITY_LOGON_TYPE::Network, pkg, base as _, total as u32, null(), &src, &mut profile, &mut profile_len, &mut logon_id, &mut token, &mut quotas, &mut sub);
        if !profile.is_null() {
            LsaFreeReturnBuffer(profile);
        }
        if st != 0 {
            return Err(io::Error::other(format!("S4U 登录失败 0x{st:08x}/0x{sub:08x}")));
        }
        let t = Handle(token);
        to_primary(t.raw())
    }
}

/// 取以某账号运行用的令牌。
pub fn token_for(acc: &Account) -> io::Result<UserToken> {
    enable_service_privileges();
    let t = match session_token(acc) {
        Some(t) => t,
        None => s4u_token(acc)?,
    };
    let t = unsafe { elevate(t) };
    // 加载用户配置（HKCU、用户环境变量）
    let mut name = wide(&acc.name);
    let mut pi: PROFILEINFOW = unsafe { std::mem::zeroed() };
    pi.dwSize = std::mem::size_of::<PROFILEINFOW>() as u32;
    pi.dwFlags = PI_NOUI;
    pi.lpUserName = name.as_mut_ptr();
    let profile = unsafe {
        if LoadUserProfileW(t.raw(), &mut pi) != 0 {
            pi.hProfile
        } else {
            null_mut()
        }
    };
    Ok(UserToken { token: t, profile })
}

/// 复制本进程（SYSTEM）的令牌并放到指定会话里，用于在用户桌面上启动画面代理。
pub fn system_token_in_session(session: u32) -> io::Result<Handle> {
    enable_service_privileges();
    let t = process_token(TOKEN_DUPLICATE | TOKEN_QUERY).ok_or_else(io::Error::last_os_error)?;
    unsafe {
        let p = to_primary(t.raw())?;
        if SetTokenInformation(p.raw(), TokenSessionId, &session as *const u32 as _, 4) == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(p)
    }
}

// ---------------- 环境变量 ----------------

/// 子进程环境：有令牌时取该账号的环境，否则继承本进程；再叠加 `extra`。返回 UTF-16 环境块。
pub fn env_block(token: Option<HANDLE>, extra: &[(String, String)]) -> Vec<u16> {
    let mut vars: Vec<(String, String)> = Vec::new();
    match token {
        Some(t) => unsafe {
            let mut p: *mut c_void = null_mut();
            if CreateEnvironmentBlock(&mut p, t, 0) != 0 {
                let mut q = p as *const u16;
                loop {
                    let s = from_wide_ptr(q);
                    if s.is_empty() {
                        break;
                    }
                    q = q.add(s.encode_utf16().count() + 1);
                    if let Some(i) = s[1..].find('=').map(|i| i + 1) {
                        vars.push((s[..i].to_string(), s[i + 1..].to_string()));
                    }
                }
                DestroyEnvironmentBlock(p);
            }
        },
        None => {
            for (k, v) in std::env::vars_os() {
                vars.push((k.to_string_lossy().to_string(), v.to_string_lossy().to_string()));
            }
        }
    }
    for (k, v) in extra {
        vars.retain(|(x, _)| !x.eq_ignore_ascii_case(k));
        vars.push((k.clone(), v.clone()));
    }
    vars.sort_by_key(|(k, _)| k.to_uppercase());
    let mut out = Vec::new();
    for (k, v) in vars {
        out.extend(format!("{k}={v}").encode_utf16());
        out.push(0);
    }
    out.push(0);
    out
}

// ---------------- 命令行 ----------------

/// 按 Windows 规则给单个参数加引号。
pub fn quote_arg(a: &str) -> String {
    if !a.is_empty() && !a.chars().any(|c| matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '"')) {
        return a.to_string();
    }
    let mut s = String::from("\"");
    let mut bs = 0;
    for c in a.chars() {
        match c {
            '\\' => bs += 1,
            '"' => {
                s.push_str(&"\\".repeat(bs * 2 + 1));
                s.push('"');
                bs = 0;
            }
            _ => {
                s.push_str(&"\\".repeat(bs));
                s.push(c);
                bs = 0;
            }
        }
    }
    s.push_str(&"\\".repeat(bs * 2));
    s.push('"');
    s
}

pub fn cmdline(program: &str, args: &[&str]) -> String {
    let mut s = quote_arg(program);
    for a in args {
        s.push(' ');
        s.push_str(&quote_arg(a));
    }
    s
}

/// PowerShell 用 -EncodedCommand 传脚本，避开所有引号问题，并把输出编码设为 UTF-8。
pub fn powershell_args(script: &str) -> Vec<String> {
    let full = format!("[Console]::OutputEncoding=[Text.Encoding]::UTF8;$OutputEncoding=[Text.Encoding]::UTF8;{script}");
    let bytes: Vec<u8> = full.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    use base64::Engine;
    vec!["-NoProfile".into(), "-NonInteractive".into(), "-ExecutionPolicy".into(), "Bypass".into(), "-EncodedCommand".into(), base64::engine::general_purpose::STANDARD.encode(bytes)]
}

pub fn system_dir() -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    format!("{root}\\System32")
}

pub fn powershell_exe() -> String {
    format!("{}\\WindowsPowerShell\\v1.0\\powershell.exe", system_dir())
}

pub fn cmd_exe() -> String {
    format!("{}\\cmd.exe", system_dir())
}

// ---------------- 管道 ----------------

/// 创建匿名管道，返回 (读端, 写端)；`child_side` 指定哪一端要被子进程继承。
pub fn pipe(child_reads: bool) -> io::Result<(Handle, Handle)> {
    unsafe {
        let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: null_mut(), bInheritHandle: 1 };
        let (mut r, mut w): (HANDLE, HANDLE) = (null_mut(), null_mut());
        if CreatePipe(&mut r, &mut w, &sa, 64 * 1024) == 0 {
            return Err(io::Error::last_os_error());
        }
        let (r, w) = (Handle(r), Handle(w));
        // 父进程那一端不可继承
        let parent = if child_reads { w.raw() } else { r.raw() };
        SetHandleInformation(parent, HANDLE_FLAG_INHERIT, 0);
        Ok((r, w))
    }
}

/// 把阻塞的读句柄变成异步读（后台线程搬运）。
pub fn async_reader(h: Handle) -> tokio::io::DuplexStream {
    use tokio::io::AsyncWriteExt;
    let (a, mut b) = tokio::io::duplex(128 * 1024);
    let rt = tokio::runtime::Handle::current();
    std::thread::spawn(move || {
        let mut f = h.into_file();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match f.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if rt.block_on(b.write_all(&buf[..n])).is_err() {
                        break;
                    }
                }
            }
        }
        let _ = rt.block_on(b.shutdown());
    });
    a
}

/// 把阻塞的写句柄变成异步写；丢掉返回值即关闭管道。
pub fn async_writer(h: Handle) -> tokio::io::DuplexStream {
    use tokio::io::AsyncReadExt;
    let (a, mut b) = tokio::io::duplex(128 * 1024);
    let rt = tokio::runtime::Handle::current();
    std::thread::spawn(move || {
        let mut f = h.into_file();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match rt.block_on(b.read(&mut buf)) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if f.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            }
        }
    });
    a
}

// ---------------- 进程 ----------------

pub struct Spawn<'a> {
    pub cmdline: String,
    pub cwd: Option<String>,
    pub env: Vec<u16>,
    pub token: Option<HANDLE>,
    /// 子进程的标准输入、输出、错误（可继承的句柄）
    pub stdio: Option<[HANDLE; 3]>,
    pub pseudo_console: Option<*mut c_void>,
    /// None 继承本进程；Some("") 让系统为该登录会话建非交互窗口站；Some("winsta0\\default") 用户桌面
    pub desktop: Option<&'a str>,
    pub no_window: bool,
}

pub struct Child {
    pub process: Handle,
    pub pid: u32,
    job: Option<Handle>,
}

impl Child {
    /// 等待退出，返回退出码。
    pub fn wait_blocking(&self) -> Option<i32> {
        unsafe {
            if WaitForSingleObject(self.process.raw(), INFINITE) != WAIT_OBJECT_0 {
                return None;
            }
            let mut code = 0u32;
            if GetExitCodeProcess(self.process.raw(), &mut code) == 0 {
                return None;
            }
            Some(code as i32)
        }
    }

    pub fn try_wait(&self) -> Option<i32> {
        unsafe {
            if WaitForSingleObject(self.process.raw(), 0) != WAIT_OBJECT_0 {
                return None;
            }
            let mut code = 0u32;
            GetExitCodeProcess(self.process.raw(), &mut code);
            Some(code as i32)
        }
    }

    pub async fn wait(self: &Arc<Self>) -> Option<i32> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || me.wait_blocking()).await.ok().flatten()
    }

    /// 结束整棵进程树。
    pub fn kill(&self) {
        unsafe {
            match &self.job {
                Some(j) => {
                    TerminateJobObject(j.raw(), 1);
                }
                None => {
                    TerminateProcess(self.process.raw(), 1);
                }
            }
        }
    }
}

pub fn spawn(s: Spawn) -> io::Result<Child> {
    unsafe {
        let mut attr_count = 0;
        if s.pseudo_console.is_some() {
            attr_count += 1;
        }
        let inherit: Vec<HANDLE> = s.stdio.map(|a| {
            let mut v: Vec<HANDLE> = Vec::new();
            for h in a {
                if !v.contains(&h) {
                    v.push(h);
                }
            }
            v
        }).unwrap_or_default();
        if !inherit.is_empty() {
            attr_count += 1;
        }
        let mut size = 0usize;
        InitializeProcThreadAttributeList(null_mut(), attr_count, 0, &mut size);
        let mut attr_buf = vec![0u8; size.max(64)];
        let attrs = attr_buf.as_mut_ptr() as *mut c_void;
        if InitializeProcThreadAttributeList(attrs, attr_count, 0, &mut size) == 0 {
            return Err(io::Error::last_os_error());
        }
        struct AttrGuard(*mut c_void);
        impl Drop for AttrGuard {
            fn drop(&mut self) {
                unsafe { DeleteProcThreadAttributeList(self.0) }
            }
        }
        let _guard = AttrGuard(attrs);
        let hpc_holder = s.pseudo_console.unwrap_or(null_mut());
        if s.pseudo_console.is_some() && UpdateProcThreadAttribute(attrs, 0, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, hpc_holder as *const c_void, std::mem::size_of::<*mut c_void>(), null_mut(), null()) == 0 {
            return Err(io::Error::last_os_error());
        }
        if !inherit.is_empty() && UpdateProcThreadAttribute(attrs, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, inherit.as_ptr() as *const c_void, inherit.len() * std::mem::size_of::<HANDLE>(), null_mut(), null()) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut si: STARTUPINFOEXW = std::mem::zeroed();
        si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        si.lpAttributeList = attrs;
        let mut desktop_w = s.desktop.map(wide);
        if let Some(d) = desktop_w.as_mut() {
            si.StartupInfo.lpDesktop = d.as_mut_ptr();
        }
        if let Some([i, o, e]) = s.stdio {
            si.StartupInfo.dwFlags |= STARTF_USESTDHANDLES;
            si.StartupInfo.hStdInput = i;
            si.StartupInfo.hStdOutput = o;
            si.StartupInfo.hStdError = e;
        }
        let mut cmd = wide(&s.cmdline);
        let cwd = s.cwd.as_deref().filter(|c| std::path::Path::new(c).is_dir()).map(wide);
        let cwd_ptr = cwd.as_ref().map(|c| c.as_ptr()).unwrap_or(null());
        let mut flags = CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED;
        if s.no_window {
            flags |= CREATE_NO_WINDOW;
        }
        let inherit_handles = if inherit.is_empty() { 0 } else { 1 };
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let env_ptr = s.env.as_ptr() as *const c_void;
        let ok = match s.token {
            Some(t) => CreateProcessAsUserW(t, null(), cmd.as_mut_ptr(), null(), null(), inherit_handles, flags, env_ptr, cwd_ptr, &si.StartupInfo, &mut pi),
            None => CreateProcessW(null(), cmd.as_mut_ptr(), null(), null(), inherit_handles, flags, env_ptr, cwd_ptr, &si.StartupInfo, &mut pi),
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let process = Handle(pi.hProcess);
        let thread = Handle(pi.hThread);
        // 放进作业对象，结束时连子进程一起结束
        let job = {
            let j = CreateJobObjectW(null(), null());
            if j.is_null() {
                None
            } else {
                let j = Handle(j);
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(j.raw(), JobObjectExtendedLimitInformation, &info as *const _ as _, std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32);
                if AssignProcessToJobObject(j.raw(), process.raw()) != 0 {
                    Some(j)
                } else {
                    None
                }
            }
        };
        ResumeThread(thread.raw());
        drop(desktop_w);
        Ok(Child { process, pid: pi.dwProcessId, job })
    }
}

/// 以管道方式启动（命令执行、文件子进程）。返回子进程与异步的标准输入 / 输出 / 错误。
pub fn spawn_piped(cmdline: String, cwd: Option<String>, token: Option<&UserToken>, extra_env: &[(String, String)]) -> io::Result<(Arc<Child>, tokio::io::DuplexStream, tokio::io::DuplexStream, tokio::io::DuplexStream)> {
    let (in_r, in_w) = pipe(true)?;
    let (out_r, out_w) = pipe(false)?;
    let (err_r, err_w) = pipe(false)?;
    let t = token.map(|t| t.raw());
    let child = spawn(Spawn {
        cmdline,
        cwd,
        env: env_block(t, extra_env),
        token: t,
        stdio: Some([in_r.raw(), out_w.raw(), err_w.raw()]),
        pseudo_console: None,
        desktop: if t.is_some() { Some("") } else { None },
        no_window: true,
    })?;
    // 子进程那一端在父进程里关掉，子进程退出时才能读到结束
    drop(in_r);
    drop(out_w);
    drop(err_w);
    Ok((Arc::new(child), async_writer(in_w), async_reader(out_r), async_reader(err_r)))
}

// ---------------- 伪终端 ----------------

#[repr(C)]
#[derive(Clone, Copy)]
struct Coord {
    x: i16,
    y: i16,
}

type CreatePcFn = unsafe extern "system" fn(Coord, HANDLE, HANDLE, u32, *mut *mut c_void) -> i32;
type ResizePcFn = unsafe extern "system" fn(*mut c_void, Coord) -> i32;
type ClosePcFn = unsafe extern "system" fn(*mut c_void);

struct ConPtyApi {
    create: CreatePcFn,
    resize: ResizePcFn,
    close: ClosePcFn,
}

/// ConPTY 需要 Windows 10 1809 / Server 2019 以上，动态加载，老系统退回简化终端。
fn conpty_api() -> Option<&'static ConPtyApi> {
    static API: std::sync::OnceLock<Option<ConPtyApi>> = std::sync::OnceLock::new();
    API.get_or_init(|| unsafe {
        let k = wide("kernel32.dll");
        let m = GetModuleHandleW(k.as_ptr());
        if m.is_null() {
            return None;
        }
        let c = GetProcAddress(m, b"CreatePseudoConsole\0".as_ptr())?;
        let r = GetProcAddress(m, b"ResizePseudoConsole\0".as_ptr())?;
        let x = GetProcAddress(m, b"ClosePseudoConsole\0".as_ptr())?;
        Some(ConPtyApi { create: std::mem::transmute::<_, CreatePcFn>(c), resize: std::mem::transmute::<_, ResizePcFn>(r), close: std::mem::transmute::<_, ClosePcFn>(x) })
    })
    .as_ref()
}

pub fn has_conpty() -> bool {
    conpty_api().is_some()
}

struct Hpc(*mut c_void);
unsafe impl Send for Hpc {}

/// 终端里的进程。
pub struct PtyProc {
    hpc: Mutex<Option<Hpc>>,
    input: Mutex<Option<std::fs::File>>,
    pub child: Arc<Child>,
    /// 没有 ConPTY 时的简化模式：本地回显、按行发送
    line_mode: bool,
    line: Mutex<Vec<u8>>,
    echo: Mutex<Option<std::sync::mpsc::Sender<Vec<u8>>>>,
}

impl PtyProc {
    /// 启动终端进程，返回进程与输出读取端。简化模式下回显也写进输出。
    pub fn spawn(cmdline: String, cwd: Option<String>, token: Option<&UserToken>, extra_env: &[(String, String)], cols: u16, rows: u16) -> io::Result<(Arc<PtyProc>, std::sync::mpsc::Receiver<Vec<u8>>)> {
        let t = token.map(|t| t.raw());
        let env = env_block(t, extra_env);
        let desktop = if t.is_some() { Some("") } else { None };
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        match conpty_api() {
            Some(api) => unsafe {
                let (in_r, in_w) = pipe(true)?;
                let (out_r, out_w) = pipe(false)?;
                let mut hpc: *mut c_void = null_mut();
                let hr = (api.create)(Coord { x: cols.max(10) as i16, y: rows.max(2) as i16 }, in_r.raw(), out_w.raw(), 0, &mut hpc);
                if hr < 0 {
                    return Err(io::Error::other(format!("CreatePseudoConsole 0x{hr:08x}")));
                }
                // ConPTY 已复制这两端
                drop(in_r);
                drop(out_w);
                let child = match spawn(Spawn { cmdline, cwd, env, token: t, stdio: None, pseudo_console: Some(hpc), desktop, no_window: false }) {
                    Ok(c) => c,
                    Err(e) => {
                        (api.close)(hpc);
                        return Err(e);
                    }
                };
                let p = Arc::new(PtyProc {
                    hpc: Mutex::new(Some(Hpc(hpc))),
                    input: Mutex::new(Some(in_w.into_file())),
                    child: Arc::new(child),
                    line_mode: false,
                    line: Mutex::new(Vec::new()),
                    echo: Mutex::new(None),
                });
                spawn_output_reader(out_r, tx);
                // 进程退出后关闭伪终端，读取端随之结束
                let p2 = p.clone();
                std::thread::spawn(move || {
                    p2.child.wait_blocking();
                    p2.close_console();
                });
                Ok((p, rx))
            },
            None => {
                let (in_r, in_w) = pipe(true)?;
                let (out_r, out_w) = pipe(false)?;
                let child = spawn(Spawn { cmdline, cwd, env, token: t, stdio: Some([in_r.raw(), out_w.raw(), out_w.raw()]), pseudo_console: None, desktop, no_window: true })?;
                drop(in_r);
                drop(out_w);
                let p = Arc::new(PtyProc {
                    hpc: Mutex::new(None),
                    input: Mutex::new(Some(in_w.into_file())),
                    child: Arc::new(child),
                    line_mode: true,
                    line: Mutex::new(Vec::new()),
                    echo: Mutex::new(Some(tx.clone())),
                });
                let _ = tx.send("此系统不支持伪终端（需要 Windows 10 1809 / Server 2019 或更新），终端为简化模式：按行发送，不支持全屏程序。\r\n".as_bytes().to_vec());
                spawn_output_reader(out_r, tx);
                let p2 = p.clone();
                std::thread::spawn(move || {
                    p2.child.wait_blocking();
                    p2.close_console();
                });
                Ok((p, rx))
            }
        }
    }

    pub fn write(&self, data: &[u8]) -> io::Result<()> {
        if !self.line_mode {
            let mut g = self.input.lock();
            return match g.as_mut() {
                Some(f) => f.write_all(data),
                None => Err(io::ErrorKind::BrokenPipe.into()),
            };
        }
        // 简化模式：本地回显，回车时整行发出
        let mut echo = Vec::new();
        let mut send = Vec::new();
        {
            let mut line = self.line.lock();
            for &b in data {
                match b {
                    b'\r' | b'\n' => {
                        echo.extend_from_slice(b"\r\n");
                        send.extend_from_slice(&line);
                        send.extend_from_slice(b"\r\n");
                        line.clear();
                    }
                    0x7f | 0x08 => {
                        if line.pop().is_some() {
                            echo.extend_from_slice(b"\x08 \x08");
                        }
                    }
                    0x03 => {
                        line.clear();
                        echo.extend_from_slice(b"^C\r\n");
                    }
                    0x1b => {}
                    _ => {
                        line.push(b);
                        echo.push(b);
                    }
                }
            }
        }
        if let Some(tx) = self.echo.lock().as_ref() {
            if !echo.is_empty() {
                let _ = tx.send(echo);
            }
        }
        if !send.is_empty() {
            let mut g = self.input.lock();
            if let Some(f) = g.as_mut() {
                f.write_all(&send)?;
            }
        }
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        if let (Some(api), Some(h)) = (conpty_api(), self.hpc.lock().as_ref()) {
            unsafe {
                (api.resize)(h.0, Coord { x: cols.max(10) as i16, y: rows.max(2) as i16 });
            }
        }
    }

    pub fn close_console(&self) {
        let h = self.hpc.lock().take();
        if let (Some(api), Some(h)) = (conpty_api(), h) {
            unsafe {
                (api.close)(h.0);
            }
        }
        self.input.lock().take();
        self.echo.lock().take();
    }

    pub fn kill(&self) {
        self.child.kill();
    }
}

fn spawn_output_reader(h: Handle, tx: std::sync::mpsc::Sender<Vec<u8>>) {
    std::thread::spawn(move || {
        let mut f = h.into_file();
        let mut buf = vec![0u8; 32 * 1024];
        loop {
            match f.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(quote_arg("abc"), "abc");
        assert_eq!(quote_arg("a b"), "\"a b\"");
        assert_eq!(quote_arg("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote_arg("C:\\x y\\"), "\"C:\\x y\\\\\"");
        assert_eq!(quote_arg(""), "\"\"");
    }

    #[test]
    fn current_user_lookup() {
        let me = current_account().expect("当前账号");
        assert!(!me.sid.is_empty());
        let again = lookup_account(&me.qualified()).expect("按名字查");
        assert_eq!(again.sid, me.sid);
    }
}
