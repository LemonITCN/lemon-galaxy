//! 本机控制接口的命名管道（docs/spec.md 3.0.2）：打开、核对属主、重叠方式读写。
//!
//! 用重叠 I/O：同步打开的句柄上所有读写是串行的，一个线程阻塞在 ReadFile 时另一个线程的 WriteFile 也会卡住。
//! 读和写各用自己的事件，等的时候用 GetOverlappedResult / WaitForSingleObject（XP 都有）。
//! 连上后先核对管道的属主：系统服务的管道必须属于 SYSTEM 或管理员组（前台运行的是本人），
//! 防止别的程序抢先建同名管道冒充服务骗密码；以「识别」级别连接（服务能认人，不能冒用身份）。

use crate::win::{from_wide, pcw, wide};
use std::io;
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, LocalFree, ERROR_IO_PENDING, ERROR_PIPE_BUSY, GENERIC_READ, GENERIC_WRITE, HANDLE, HLOCAL, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Security::Authorization::{ConvertSidToStringSidW, GetSecurityInfo, SE_KERNEL_OBJECT};
use windows::Win32::Security::{GetTokenInformation, TokenUser, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, TOKEN_QUERY, TOKEN_USER};
use windows::Win32::Storage::FileSystem::{CreateFileW, ReadFile, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_OVERLAPPED, FILE_SHARE_NONE, OPEN_EXISTING, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT};
use windows::Win32::System::Pipes::WaitNamedPipeW;
use windows::Win32::System::Threading::{CreateEventW, GetCurrentProcess, OpenProcessToken, ResetEvent, WaitForSingleObject};
use windows::Win32::System::IO::{CancelIo, GetOverlappedResult, OVERLAPPED};

pub use crate::control::SYSTEM_PIPE;

const SYSTEM_SID: &str = "S-1-5-18";
const ADMINS_SID: &str = "S-1-5-32-544";
const PREFIX: &str = r"\\.\pipe\";

/// 去掉 `\\.\pipe\` 前缀
pub fn short_name(name: &str) -> &str {
    if name.len() >= PREFIX.len() && name[..PREFIX.len()].eq_ignore_ascii_case(PREFIX) {
        &name[PREFIX.len()..]
    } else {
        name
    }
}

pub fn full_name(name: &str) -> String {
    format!("{PREFIX}{}", short_name(name))
}

/// 按顺序找服务：环境变量 LEMON_GALAXY_CONTROL_SOCKET（\\.\pipe\名字）> 系统服务 > 本用户前台运行的服务。
/// 返回 (完整管道名, 是不是系统服务的)
pub fn candidates() -> Vec<(String, bool)> {
    if let Some(v) = std::env::var("LEMON_GALAXY_CONTROL_SOCKET").ok().filter(|v| !v.trim().is_empty()) {
        return vec![(full_name(v.trim()), false)];
    }
    let mut v = vec![(SYSTEM_PIPE.to_string(), true)];
    if let Some(sid) = current_user_sid() {
        v.push((crate::control::user_pipe_name(&sid), false));
    }
    v
}

pub fn is_system_pipe(name: &str) -> bool {
    short_name(name).eq_ignore_ascii_case(short_name(SYSTEM_PIPE))
}

/// 当前用户的 SID 字符串
pub fn current_user_sid() -> Option<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        let mut len = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
        let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
        let ok = GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr() as _), len, &mut len).is_ok();
        let _ = CloseHandle(token);
        if !ok {
            return None;
        }
        let tu = &*(buf.as_ptr() as *const TOKEN_USER);
        sid_string(tu.User.Sid)
    }
}

unsafe fn sid_string(sid: PSID) -> Option<String> {
    let mut p = PWSTR::null();
    unsafe {
        ConvertSidToStringSidW(sid, &mut p).ok()?;
        let s = from_wide(std::slice::from_raw_parts(p.0, wcslen(p.0)));
        LocalFree(Some(HLOCAL(p.0 as _)));
        Some(s)
    }
}

unsafe fn wcslen(p: *const u16) -> usize {
    let mut n = 0;
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    n
}

/// 事件对象（手动复位）
struct Event(HANDLE);

impl Event {
    fn new() -> io::Result<Self> {
        unsafe { CreateEventW(None, true, false, None).map(Event).map_err(|e| io::Error::other(e.message())) }
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// 打开着的管道。句柄存成整数，读写线程之间共享（Arc），最后一个放手时关掉
pub struct Pipe {
    h: isize,
}

unsafe impl Send for Pipe {}
unsafe impl Sync for Pipe {}

impl Drop for Pipe {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle());
        }
    }
}

pub enum OpenError {
    /// 没有这个管道（服务没运行）
    Absent,
    /// 管道在，但属主不对或读不到属主
    Suspicious(String),
}

impl Pipe {
    fn handle(&self) -> HANDLE {
        HANDLE(self.h as _)
    }

    /// 打开管道并核对属主。`system`：这是系统服务的管道（只认 SYSTEM、管理员组）；否则本人也认
    pub fn open(name: &str, system: bool, wait_ms: u32) -> Result<Pipe, OpenError> {
        let full = full_name(name);
        let w = wide(&full);
        let flags = FILE_FLAGS_AND_ATTRIBUTES(FILE_FLAG_OVERLAPPED.0 | SECURITY_SQOS_PRESENT.0 | SECURITY_IDENTIFICATION.0);
        let mut tries = 0;
        let h = loop {
            match unsafe { CreateFileW(pcw(&w), GENERIC_READ.0 | GENERIC_WRITE.0, FILE_SHARE_NONE, None, OPEN_EXISTING, flags, None) } {
                Ok(h) => break h,
                Err(e) if e.code() == ERROR_PIPE_BUSY.to_hresult() && tries < 2 => {
                    tries += 1;
                    unsafe {
                        let _ = WaitNamedPipeW(pcw(&w), wait_ms);
                    }
                }
                Err(_) => return Err(OpenError::Absent),
            }
        };
        let pipe = Pipe { h: h.0 as isize };
        match owner_sid(h) {
            Some(owner) => {
                let me = current_user_sid();
                let ok = owner == SYSTEM_SID || owner == ADMINS_SID || (!system && me.as_deref() == Some(owner.as_str()));
                if !ok {
                    return Err(OpenError::Suspicious(format!("管道 {full} 的属主不对（{owner}），不是 LemonGalaxy 被控服务")));
                }
            }
            None => return Err(OpenError::Suspicious(format!("读不到管道 {full} 的属主"))),
        }
        Ok(pipe)
    }

    /// 读一些字节，0 表示对方关了。`timeout_ms` 为 None 时一直等
    pub fn read(&self, buf: &mut [u8], timeout_ms: Option<u32>) -> io::Result<usize> {
        let ev = Event::new()?;
        self.io(&ev, timeout_ms, |ov| unsafe { ReadFile(self.handle(), Some(buf), None, Some(ov)) })
    }

    pub fn write_all(&self, mut data: &[u8], timeout_ms: Option<u32>) -> io::Result<()> {
        let ev = Event::new()?;
        while !data.is_empty() {
            let n = self.io(&ev, timeout_ms, |ov| unsafe { WriteFile(self.handle(), Some(data), None, Some(ov)) })?;
            if n == 0 {
                return Err(io::ErrorKind::WriteZero.into());
            }
            data = &data[n..];
        }
        Ok(())
    }

    fn io(&self, ev: &Event, timeout_ms: Option<u32>, start: impl FnOnce(*mut OVERLAPPED) -> windows::core::Result<()>) -> io::Result<usize> {
        let mut ov = OVERLAPPED { hEvent: ev.0, ..Default::default() };
        unsafe {
            let _ = ResetEvent(ev.0);
        }
        match start(&mut ov) {
            Ok(()) => {}
            Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {
                if let Some(ms) = timeout_ms {
                    let r = unsafe { WaitForSingleObject(ev.0, ms) };
                    if r == WAIT_TIMEOUT {
                        // CancelIo 只取消本线程发出的 I/O（XP 上也是这样），正好是这一个
                        unsafe {
                            let _ = CancelIo(self.handle());
                            let mut n = 0u32;
                            let _ = GetOverlappedResult(self.handle(), &ov, &mut n, true);
                        }
                        return Err(io::ErrorKind::TimedOut.into());
                    }
                    if r != WAIT_OBJECT_0 {
                        return Err(io::Error::last_os_error());
                    }
                }
            }
            Err(e) => return Err(broken(e)),
        }
        let mut n = 0u32;
        match unsafe { GetOverlappedResult(self.handle(), &ov, &mut n, true) } {
            Ok(()) => Ok(n as usize),
            Err(e) => Err(broken(e)),
        }
    }
}

fn broken(e: windows::core::Error) -> io::Error {
    // 管道断了（ERROR_BROKEN_PIPE 等）都当成对方关了
    io::Error::new(io::ErrorKind::BrokenPipe, e.message())
}

/// 内核对象的属主 SID
fn owner_sid(h: HANDLE) -> Option<String> {
    unsafe {
        let mut owner = PSID::default();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        let r = GetSecurityInfo(h, SE_KERNEL_OBJECT, OWNER_SECURITY_INFORMATION, Some(&mut owner), None, None, None, Some(&mut sd));
        if r.0 != 0 {
            return None;
        }
        let s = sid_string(owner);
        LocalFree(Some(HLOCAL(sd.0)));
        s
    }
}

/// 一次性的同步调用（`--elevate` 用）：连上、按顺序发几条请求、收齐回复、断开
pub fn call_once(name: &str, reqs: &[crate::control::Request], timeout_ms: u32) -> Result<Vec<crate::control::Response>, String> {
    use crate::control::*;
    let pipe = match Pipe::open(name, is_system_pipe(name), 3000) {
        Ok(p) => p,
        Err(OpenError::Absent) => return Err(crate::tr!("被控服务没有运行。", "The LemonGalaxy service is not running.").to_string()),
        Err(OpenError::Suspicious(m)) => return Err(m),
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms as u64);
    let left = || deadline.saturating_duration_since(std::time::Instant::now()).as_millis().max(1) as u32;
    let mut out = vec![];
    let mut pending = Vec::<u8>::new();
    let mut buf = vec![0u8; 64 * 1024];
    for (i, r) in reqs.iter().enumerate() {
        let id = i as u64 + 1;
        pipe.write_all(encode(&RequestLine { id, req: r.clone() }).as_bytes(), Some(left())).map_err(|e| e.to_string())?;
        'wait: loop {
            while let Some(p) = pending.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = pending.drain(..=p).collect();
                if let Ok(ServerLine::Response(rl)) = serde_json::from_slice::<ServerLine>(&line) {
                    if rl.id == id {
                        out.push(rl.resp);
                        break 'wait;
                    }
                }
            }
            if std::time::Instant::now() >= deadline {
                return Err(crate::tr!("被控服务没有回应。", "The LemonGalaxy service did not respond.").to_string());
            }
            let n = pipe.read(&mut buf, Some(left())).map_err(|e| e.to_string())?;
            if n == 0 {
                return Err(crate::tr!("和被控服务的连接断开了。", "Lost connection to the LemonGalaxy service.").to_string());
            }
            pending.extend_from_slice(&buf[..n]);
        }
    }
    Ok(out)
}
