//! Windows 本机控制接口要用的系统信息：命名管道的访问控制、按管道认出对方是谁（令牌）、
//! 对方是不是坐在屏幕前（会话状态）、可选的系统用户（ProfileList）。见 docs/spec.md 3.0.2。

use std::ffi::c_void;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, ERROR_SUCCESS, HANDLE};
use windows_sys::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, ConvertStringSidToSidW, SDDL_REVISION_1};
use windows_sys::Win32::Security::{GetTokenInformation, LookupAccountSidW, RevertToSelf, SECURITY_ATTRIBUTES, SID_NAME_USE, TOKEN_GROUPS, TOKEN_QUERY, TOKEN_USER, TokenGroups, TokenSessionId, TokenUser};
use windows_sys::Win32::System::Pipes::{GetNamedPipeClientProcessId, ImpersonateNamedPipeClient};
use windows_sys::Win32::System::Registry::{RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ};
use windows_sys::Win32::System::RemoteDesktop::{WTSActive, WTSConnectState, WTSFreeMemory, WTSQuerySessionInformationW, WTS_CURRENT_SERVER_HANDLE};
use windows_sys::Win32::System::Threading::{GetCurrentThread, OpenThreadToken};

pub const SYSTEM_SID: &str = "S-1-5-18";
const ADMINS_SID: &str = "S-1-5-32-544";
const SE_GROUP_ENABLED: u32 = 0x4;
const SE_GROUP_USE_FOR_DENY_ONLY: u32 = 0x10;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
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

/// 安全描述符（SDDL 生成，LocalFree 释放）。
pub struct SecurityDescriptor(*mut c_void);

unsafe impl Send for SecurityDescriptor {}
unsafe impl Sync for SecurityDescriptor {}

impl SecurityDescriptor {
    pub fn from_sddl(sddl: &str) -> std::io::Result<Self> {
        let w = wide(sddl);
        let mut sd: *mut c_void = null_mut();
        if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(w.as_ptr(), SDDL_REVISION_1, &mut sd, null_mut()) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self(sd))
    }

    /// 给 CreateNamedPipe 用的 SECURITY_ATTRIBUTES（指向自己，自己要活得比管道创建调用长）
    pub fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: self.0, bInheritHandle: 0 }
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { LocalFree(self.0 as _) };
        }
    }
}

/// 控制管道的访问控制。
/// 系统服务：属主 SYSTEM；SYSTEM、管理员组完全控制；登录过的用户（Authenticated Users）只能读写，
/// 不能再建同名管道实例（去掉 FILE_CREATE_PIPE_INSTANCE），也就抢不了这个名字。鉴权按连上来的令牌另做。
/// 普通用户前台运行：只给这个用户和 SYSTEM。
pub fn pipe_sddl(system: bool, owner_sid: &str) -> String {
    if system {
        "O:SYG:SYD:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x12019b;;;AU)".into()
    } else {
        format!("D:P(A;;GA;;;{owner_sid})(A;;GA;;;SY)")
    }
}

/// 管道那头是谁（按对方的令牌）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PipePeer {
    /// 「域\用户名」（本机账号是「计算机名\用户名」，NT AUTHORITY 下的只写名字），与 SysUser::display_name 一致
    pub name: String,
    pub sid: String,
    /// LocalSystem
    pub system: bool,
    /// 管理员组在令牌里生效（已提升的管理员，或关了 UAC 的管理员）
    pub elevated_admin: bool,
    /// 是管理员组成员（没提升时管理员组只用于拒绝，也算）
    pub admin_member: bool,
    /// 对方进程所在的会话
    pub session: u32,
    pub pid: Option<u32>,
}

struct Token(HANDLE);

impl Drop for Token {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// 令牌信息，放在 8 字节对齐的缓冲里（里面有指针；SID 指针指向缓冲内部，缓冲要活到用完为止）
unsafe fn token_info(token: HANDLE, class: i32) -> Option<Vec<u64>> {
    let mut len = 0u32;
    GetTokenInformation(token, class, null_mut(), 0, &mut len);
    if len == 0 {
        return None;
    }
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    if GetTokenInformation(token, class, buf.as_mut_ptr() as _, len, &mut len) == 0 {
        return None;
    }
    Some(buf)
}

unsafe fn sid_string(sid: *mut c_void) -> String {
    let mut p: *mut u16 = null_mut();
    if ConvertSidToStringSidW(sid, &mut p) == 0 {
        return String::new();
    }
    let s = from_wide_ptr(p);
    LocalFree(p as _);
    s
}

/// SID 对应的「域\用户名」（与 SysUser::display_name 同样的写法）
unsafe fn sid_display_name(sid: *mut c_void) -> Option<String> {
    let mut name = [0u16; 256];
    let mut domain = [0u16; 256];
    let (mut nl, mut dl) = (name.len() as u32, domain.len() as u32);
    let mut use_: SID_NAME_USE = 0;
    if LookupAccountSidW(null(), sid, name.as_mut_ptr(), &mut nl, domain.as_mut_ptr(), &mut dl, &mut use_) == 0 {
        return None;
    }
    Some(display_name(&from_wide_buf(&domain), &from_wide_buf(&name)))
}

pub fn display_name(domain: &str, name: &str) -> String {
    if domain.is_empty() || domain.eq_ignore_ascii_case("NT AUTHORITY") {
        name.to_string()
    } else {
        format!("{domain}\\{name}")
    }
}

/// 管道那头的令牌：先模拟对方（只要识别级），打开线程令牌，马上恢复自己。
/// 对方还没写过数据时模拟会失败（ERROR_CANNOT_IMPERSONATE），所以要在读到第一行之后调用。
pub fn identify_pipe_client(pipe: HANDLE) -> std::io::Result<PipePeer> {
    unsafe {
        if ImpersonateNamedPipeClient(pipe) == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut t: HANDLE = null_mut();
        // OpenAsSelf：按服务自己的身份做访问检查（识别级的令牌不能用来打开自己）
        let ok = OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut t);
        let err = std::io::Error::last_os_error();
        // 无论如何马上恢复，后面的事都以服务自己的身份做
        if RevertToSelf() == 0 {
            // 恢复不了是严重问题：这个线程还顶着别人的身份，不能再用
            panic!("RevertToSelf failed: {}", std::io::Error::last_os_error());
        }
        if ok == 0 {
            return Err(err);
        }
        let token = Token(t);
        let mut peer = peer_from_token(token.0).ok_or_else(|| std::io::Error::other("token"))?;
        let mut pid = 0u32;
        if GetNamedPipeClientProcessId(pipe, &mut pid) != 0 {
            peer.pid = Some(pid);
        }
        Ok(peer)
    }
}

unsafe fn peer_from_token(token: HANDLE) -> Option<PipePeer> {
    let user = token_info(token, TokenUser)?;
    let tu = &*(user.as_ptr() as *const TOKEN_USER);
    let sid = sid_string(tu.User.Sid);
    let name = sid_display_name(tu.User.Sid).unwrap_or_else(|| sid.clone());
    let mut peer = PipePeer { system: sid == SYSTEM_SID, sid, name, ..Default::default() };
    if let Some(g) = token_info(token, TokenGroups) {
        let tg = &*(g.as_ptr() as *const TOKEN_GROUPS);
        let groups = std::slice::from_raw_parts(tg.Groups.as_ptr(), tg.GroupCount as usize);
        for ga in groups {
            if sid_string(ga.Sid) == ADMINS_SID {
                peer.admin_member = true;
                if ga.Attributes & SE_GROUP_ENABLED != 0 && ga.Attributes & SE_GROUP_USE_FOR_DENY_ONLY == 0 {
                    peer.elevated_admin = true;
                }
            }
        }
    }
    if let Some(s) = token_info(token, TokenSessionId) {
        peer.session = *(s.as_ptr() as *const u32);
    }
    Some(peer)
}

/// 会话是不是有人在屏幕前（控制台或远程桌面里已登录且连着）。会话 0 是服务会话，不算。
pub fn session_active(session: u32) -> bool {
    if session == 0 {
        return false;
    }
    unsafe {
        let mut buf: *mut u16 = null_mut();
        let mut len = 0u32;
        if WTSQuerySessionInformationW(WTS_CURRENT_SERVER_HANDLE, session, WTSConnectState, &mut buf, &mut len) == 0 || buf.is_null() {
            return false;
        }
        let state = *(buf as *const i32);
        WTSFreeMemory(buf as _);
        state == WTSActive
    }
}

/// 可选的系统用户：在这台电脑上登录过的账号（注册表 ProfileList，本机与域账号都有），最后加 SYSTEM。
pub fn login_users() -> Vec<String> {
    let mut v = vec![];
    unsafe {
        let path = wide("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\ProfileList");
        let mut key: HKEY = null_mut();
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, path.as_ptr(), 0, KEY_READ, &mut key) == ERROR_SUCCESS {
            let mut i = 0u32;
            loop {
                let mut name = [0u16; 256];
                let mut nl = name.len() as u32;
                if RegEnumKeyExW(key, i, name.as_mut_ptr(), &mut nl, null(), null_mut(), null_mut(), null_mut()) != ERROR_SUCCESS {
                    break;
                }
                i += 1;
                let sid = from_wide_buf(&name);
                // S-1-5-21-…：真正的用户账号（本机、域）；LocalService 等服务账号不要
                if !sid.starts_with("S-1-5-21-") {
                    continue;
                }
                let ws = wide(&sid);
                let mut psid: *mut c_void = null_mut();
                if ConvertStringSidToSidW(ws.as_ptr(), &mut psid) == 0 {
                    continue;
                }
                if let Some(n) = sid_display_name(psid) {
                    if !v.iter().any(|x: &String| x.eq_ignore_ascii_case(&n)) {
                        v.push(n);
                    }
                }
                LocalFree(psid as _);
            }
            RegCloseKey(key);
        }
    }
    v.sort_by_key(|a| a.to_lowercase());
    v.push("SYSTEM".into());
    v
}

/// 两个系统用户名是不是同一个人：「域\用户名」与只写用户名的都认（不区分大小写）。
pub fn same_user(a: &str, b: &str) -> bool {
    if a.eq_ignore_ascii_case(b) {
        return true;
    }
    let short = |s: &str| s.rsplit('\\').next().unwrap_or(s).to_ascii_lowercase();
    // 只有一边写了域时按用户名比；两边都写了域且不同，是不同账号
    if a.contains('\\') && b.contains('\\') {
        return false;
    }
    short(a) == short(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(display_name("NT AUTHORITY", "SYSTEM"), "SYSTEM");
        assert_eq!(display_name("PC", "alice"), "PC\\alice");
        assert!(same_user("PC\\Alice", "alice"));
        assert!(same_user("pc\\alice", "PC\\ALICE"));
        assert!(!same_user("PC\\alice", "CORP\\alice"));
        assert!(!same_user("alice", "bob"));
    }

    #[test]
    fn sddl_parses() {
        assert!(SecurityDescriptor::from_sddl(&pipe_sddl(true, "")).is_ok());
        let me = galaxy_host::sysuser::SysUser::current();
        assert!(!me.sid.is_empty());
        assert!(SecurityDescriptor::from_sddl(&pipe_sddl(false, &me.sid)).is_ok());
        assert!(SecurityDescriptor::from_sddl("not sddl").is_err());
    }

    #[test]
    fn users_listed() {
        let v = login_users();
        assert_eq!(v.last().map(|s| s.as_str()), Some("SYSTEM"));
    }
}
