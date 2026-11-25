//! Linux 远程桌面实现（docs/linux-desktop.md）。
//!
//! - proxy.rs：服务这一侧的平台层（找会话、管画面代理、转请求）
//! - agent.rs：画面代理（会话里运行：截屏、编码、键鼠、剪贴板、声音）
//! - session.rs：找图形会话；vdesktop.rs：虚拟桌面
//! - x11/、wayland/：画面后端；encode/：编码（NVENC、VA-API、openh264）；audio.rs：声音
//! - clipoffer.rs + fuse.rs：复制粘贴文件，粘贴时才传（FUSE 虚拟目录，没有 FUSE 时粘贴时先下载）
//! - ibus.rs：Wayland 下输入文字（中日韩等）经 IBus 输入法引擎提交

mod actions;
mod agent;
mod audio;
mod backend;
mod clipfmt;
mod clipoffer;
pub mod encode;
mod fuse;
mod hidmap;
mod ibus;
mod inhibit;
mod ipc;
mod proxy;
mod session;
mod vdesktop;
mod wayland;
mod x11;

use galaxy_host::platform::HostPlatform;
use std::path::Path;
use std::sync::{Arc, OnceLock};

/// 被控端数据目录（读 host.json 里画面相关的设置用）：命令行 --data-dir > 环境变量 > 系统默认
pub fn data_dir() -> std::path::PathBuf {
    let args: Vec<String> = std::env::args().collect();
    let arg = args.iter().position(|a| a == "--data-dir").and_then(|i| args.get(i + 1)).map(std::path::PathBuf::from);
    galaxy_worker::data_dir(&crate::os::LinuxOs, arg)
}

pub fn platform() -> Arc<dyn HostPlatform> {
    static P: OnceLock<Arc<proxy::LinuxPlatform>> = OnceLock::new();
    P.get_or_init(proxy::LinuxPlatform::new).clone()
}

fn zh() -> bool {
    galaxy_worker::zh()
}

pub fn extra_command(cmd: &str, rest: &[String], _dir: &Path) -> Option<i32> {
    match cmd {
        "__lg-session-agent" => Some(agent::main()),
        "__lg-ibus-guard" => Some(ibus::guard_main()),
        "__lg-selftest" => Some(agent::selftest(rest.first().and_then(|s| s.parse().ok()).unwrap_or(3))),
        "selftest-capture" => Some(selftest_cmd(rest)),
        "desktop" => Some(desktop_cmd(rest)),
        "login-screen" => Some(login_screen_cmd(rest)),
        "unlock-on-connect" => Some(unlock_on_connect_cmd(rest)),
        _ => None,
    }
}

pub fn extra_help() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("desktop", "查看图形会话与虚拟桌面；desktop start <用户> [宽x高] 开虚拟桌面，desktop stop <用户> 关掉", "Show graphical sessions and virtual desktops; `desktop start <user> [WxH]` / `desktop stop <user>`"),
        ("login-screen", "login-screen x11 on|off：GDM 登录界面改用 X11（远程才能看到登录界面），重启或注销后生效", "login-screen x11 on|off: make the GDM login screen use X11 so it can be viewed remotely"),
        ("unlock-on-connect", "unlock-on-connect on|off：远程连入时自动解锁（GNOME 等 Wayland 锁屏时禁止远程桌面），远程结束后重新锁上；默认关", "unlock-on-connect on|off: unlock the screen when someone connects (Wayland desktops block remote desktop while locked), lock again afterwards; off by default"),
        ("selftest-capture", "画面自检：找到的会话、屏幕、截屏与各编码器耗时、声音", "Capture self-test: session, screens, capture and encoder timing, audio"),
    ]
}

/// 自检：在会话里跑 `__lg-selftest`（root 时以会话用户身份）
fn selftest_cmd(rest: &[String]) -> i32 {
    // selftest-capture [秒数] [--user <用户>]：指定用户时测他的虚拟桌面
    let user = rest.iter().position(|a| a == "--user").and_then(|i| rest.get(i + 1)).cloned();
    let secs = rest.iter().find(|a| a.parse::<u64>().is_ok()).cloned().unwrap_or_else(|| "3".into());
    let target = match user {
        Some(u) => match galaxy_host::sysuser::SysUser::lookup(&u).and_then(|x| vdesktop::session_for(x.uid)) {
            Some(s) => Some(s),
            None => {
                println!("{}", if zh() { "这个用户没有虚拟桌面（先运行 desktop start <用户>）" } else { "This user has no virtual desktop (run `desktop start <user>` first)" });
                return 1;
            }
        },
        None => session::find_seat().filter(|s| !(s.greeter && s.kind == session::Kind::Wayland)).or_else(vdesktop::any_session).or_else(session::from_own_env),
    };
    let Some(s) = target else {
        println!("{}", if zh() { "没有找到可看的图形会话（没有人登录桌面）。" } else { "No graphical session found." });
        match vdesktop::availability() {
            Ok(a) => println!("{}{a}", if zh() { "可以开虚拟桌面：" } else { "Virtual desktop available: " }),
            Err(e) => println!("{e}"),
        }
        return 1;
    };
    println!("{}{}", if zh() { "会话：" } else { "Session: " }, s.describe());
    for (k, v) in &s.env {
        println!("  {k}={v}");
    }
    let (mut cmd, backend) = match proxy::session_command(&s, "__lg-selftest") {
        Ok(c) => c,
        Err(e) => {
            println!("{e}");
            return 1;
        }
    };
    println!("{}{backend}\n", if zh() { "后端：" } else { "Backend: " });
    cmd.arg(secs);
    match cmd.status() {
        Ok(st) => st.code().unwrap_or(1),
        Err(e) => {
            println!("{e}");
            1
        }
    }
}

fn desktop_cmd(rest: &[String]) -> i32 {
    if rest.first().map(|s| s.as_str()) == Some("start") {
        let Some(name) = rest.get(1) else {
            eprintln!("lemon-galaxy-worker desktop start <user> [WxH]");
            return 2;
        };
        let (w, h, _) = galaxy_host::accounts::parse_headless_display(rest.get(2).map(|s| s.as_str()).unwrap_or("")).unwrap_or((1920, 1080, 60));
        // 虚拟桌面的进程脱离本命令继续运行，被控服务按状态文件认领
        return match vdesktop::ensure(name, w, h) {
            Ok(s) => {
                println!("{}{}", if zh() { "已开：" } else { "Started: " }, s.describe());
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        };
    }
    if rest.first().map(|s| s.as_str()) == Some("stop") {
        let Some(name) = rest.get(1) else {
            eprintln!("lemon-galaxy-worker desktop stop <user>");
            return 2;
        };
        let Some(u) = galaxy_host::sysuser::SysUser::lookup(name) else {
            eprintln!("{} {name}", if zh() { "系统用户不存在：" } else { "No such user:" });
            return 1;
        };
        // 虚拟桌面属于服务进程：这里按状态文件找到进程结束它（服务下次检查时会发现它没了）
        vdesktop::stop(u.uid);
        println!("{}", if zh() { "已关闭。" } else { "Closed." });
        return 0;
    }
    match session::find_seat() {
        Some(s) => println!("{}{}", if zh() { "屏幕上：" } else { "On screen: " }, s.describe()),
        None => println!("{}", if zh() { "屏幕上：没有图形会话" } else { "On screen: no graphical session" }),
    }
    let users = vdesktop::users();
    if users.is_empty() {
        println!("{}", if zh() { "虚拟桌面：没有" } else { "Virtual desktops: none" });
    }
    for uid in users {
        if let Some(s) = vdesktop::session_for(uid) {
            println!("{}{}", if zh() { "虚拟桌面：" } else { "Virtual desktop: " }, s.describe());
        }
    }
    match vdesktop::availability() {
        Ok(a) => println!("{}{a}", if zh() { "可开虚拟桌面：" } else { "Can start virtual desktops: " }),
        Err(e) => println!("{e}"),
    }
    0
}

/// GDM 登录界面用不用 X11（docs/linux-desktop.md 第 4 节）
fn login_screen_cmd(rest: &[String]) -> i32 {
    use galaxy_worker::gdm::{self, GdmState};
    let show = || match gdm::state() {
        GdmState::NotGdm => println!("{}", if zh() { "登录界面不是 GDM，不需要设置（LightDM、SDDM 用 X11，远程能看到）" } else { "The login screen is not GDM; nothing to change." }),
        GdmState::Gdm { path, x11, x11_available } => {
            println!("{}{}（{}）", if zh() { "GDM 登录界面使用 X11：" } else { "GDM login screen uses X11: " }, if x11 { "on" } else { "off" }, path.display());
            if !x11_available {
                println!("{}", if zh() { "这个系统没有 X11 桌面会话（GNOME 49 起已去掉），不能切换；没人登录时远程会自动使用虚拟桌面。" } else { "This system has no X11 desktop session (removed since GNOME 49); remote access uses a virtual desktop when nobody is signed in." });
            }
        }
    };
    let on = match (rest.first().map(|s| s.as_str()), rest.get(1).map(|s| s.as_str())) {
        (Some("x11"), Some("on")) => true,
        (Some("x11"), Some("off")) => false,
        _ => {
            show();
            return 0;
        }
    };
    if !galaxy_host::sysuser::is_root() {
        eprintln!("{}", if zh() { "要 root：sudo lemon-galaxy-worker login-screen x11 on|off" } else { "Run as root." });
        return 1;
    }
    match gdm::set(on) {
        Ok(_) => {
            show();
            println!("{}", if zh() { "重启电脑（或注销后回到登录界面）生效。" } else { "Takes effect after reboot or logout." });
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// 远程连入时自动解锁（docs/linux-desktop.md 第 4 节）
fn unlock_on_connect_cmd(rest: &[String]) -> i32 {
    let path = data_dir().join("host.json");
    let mut st = galaxy_host::accounts::HostState::load(&path);
    let label = if zh() { "远程连入时自动解锁" } else { "Unlock when someone connects" };
    match rest.first().map(|s| s.as_str()) {
        Some("on") => st.unlock_on_connect = true,
        Some("off") => st.unlock_on_connect = false,
        _ => {
            println!("{label}: {}", if st.unlock_on_connect { "on" } else { "off" });
            return 0;
        }
    }
    if let Err(e) = st.save(&path) {
        eprintln!("{e}");
        return 1;
    }
    println!("{label}: {}", if st.unlock_on_connect { "on" } else { "off" });
    0
}
