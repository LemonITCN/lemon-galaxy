//! 以管理员身份运行本程序的一个子命令并等它结束。界面本身一直以普通权限运行。
//!
//! Vista / 7：`ShellExecuteExW` 的 `runas`（弹「用户账户控制」；标准用户输管理员密码）。
//! XP 没有 UAC：已经是管理员就直接运行（不弹任何东西）；不是管理员时 `runas` 弹 XP 的「运行身份」框选管理员账号。
//! 子命令把结果（成功为空，失败为原因）写进调用方指定的文件：提权后的进程可能是另一个账号，读不到调用方的输出。

use crate::win::{pcw, wide};
use std::path::PathBuf;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, HWND};
use windows::Win32::Security::{AllocateAndInitializeSid, CheckTokenMembership, FreeSid, PSID, SECURITY_NT_AUTHORITY};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

#[derive(Clone, Debug)]
pub struct ElevResult {
    pub ok: bool,
    /// 在「用户账户控制」（XP 的「运行身份」）里点了取消
    pub cancelled: bool,
    pub error: Option<String>,
}

/// 当前进程是已生效的管理员（XP 上的管理员账号；Vista 起是已提升的管理员）。
/// CheckTokenMembership 对 UAC 过滤后的令牌（管理员组只用于拒绝）返回否，正好是要的意思
pub fn is_admin() -> bool {
    unsafe {
        let mut sid = PSID::default();
        // SECURITY_BUILTIN_DOMAIN_RID = 32，DOMAIN_ALIAS_RID_ADMINS = 544
        if AllocateAndInitializeSid(&SECURITY_NT_AUTHORITY, 2, 32, 544, 0, 0, 0, 0, 0, 0, &mut sid).is_err() {
            return false;
        }
        let mut member = windows::core::BOOL(0);
        let ok = CheckTokenMembership(None, sid, &mut member).is_ok() && member.as_bool();
        FreeSid(sid);
        ok
    }
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', ""))
}

/// 以管理员身份运行 `exe args… --result 临时文件`，等它结束（会阻塞，在后台线程调用）。`owner`：UAC 框的父窗口
pub fn run_blocking(args: &[String], owner: isize) -> ElevResult {
    let result = std::env::temp_dir().join(format!("lemon-galaxy-worker-ui-{}-{}.txt", std::process::id(), galaxy_proto::now_ms()));
    let r = run_inner(args, owner, &result);
    let _ = std::fs::remove_file(&result);
    r
}

fn run_inner(args: &[String], owner: isize, result: &PathBuf) -> ElevResult {
    let mut all: Vec<String> = args.iter().map(|a| quote(a)).collect();
    all.push("--result".into());
    all.push(quote(&result.display().to_string()));
    let params = wide(&all.join(" "));
    let exe = wide(&crate::setup::exe_path().display().to_string());
    // XP 上已经是管理员：直接运行；其余情况走 runas（Vista 起已提升时也不会再弹框）
    let runas = wide("runas");
    let verb = if crate::win::compat::legacy() && is_admin() { PCWSTR::null() } else { pcw(&runas) };
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            hwnd: HWND(owner as _),
            lpVerb: verb,
            lpFile: pcw(&exe),
            lpParameters: pcw(&params),
            nShow: SW_HIDE.0,
            ..Default::default()
        };
        let started = ShellExecuteExW(&mut info);
        CoUninitialize();
        if let Err(e) = started {
            if e.code() == ERROR_CANCELLED.to_hresult() {
                return ElevResult { ok: false, cancelled: true, error: None };
            }
            return ElevResult { ok: false, cancelled: false, error: Some(e.message()) };
        }
        // 32 位上这个结构体是 1 字节对齐的：字段先拷出来再用
        let process = { info.hProcess };
        if process.0.is_null() {
            return ElevResult { ok: false, cancelled: false, error: Some("ShellExecuteEx".into()) };
        }
        WaitForSingleObject(process, INFINITE);
        let mut code = 1u32;
        let _ = GetExitCodeProcess(process, &mut code);
        let _ = CloseHandle(process);
        if code == 0 {
            return ElevResult { ok: true, cancelled: false, error: None };
        }
        let text = std::fs::read_to_string(result).map(|s| s.trim().to_string()).unwrap_or_default();
        ElevResult { ok: false, cancelled: false, error: Some(if text.is_empty() { format!("exit {code}") } else { text }) }
    }
}

/// 子命令把失败原因写回调用方
pub fn write_result(path: Option<&str>, error: &str) {
    if let Some(p) = path.filter(|p| !p.is_empty()) {
        let _ = std::fs::write(p, error);
    }
}

/// `--elevate`：以管理员身份运行时把钥匙交给服务，授权界面那个连接几分钟内直接改配置。返回退出码
pub fn elevate_main(key: &str, pipe: &str, result: Option<&str>) -> i32 {
    use crate::control::{Request, Response};
    let reqs = [Request::Hello { lang: crate::i18n::lang_tag().into() }, Request::Elevate { key: key.to_string() }];
    match crate::pipe::call_once(pipe, &reqs, 15_000) {
        Ok(rs) => match rs.last() {
            Some(Response::Ok { .. }) => 0,
            Some(Response::Error { message, .. }) => {
                write_result(result, message);
                2
            }
            _ => {
                write_result(result, "elevate");
                2
            }
        },
        Err(e) => {
            write_result(result, &e);
            3
        }
    }
}
