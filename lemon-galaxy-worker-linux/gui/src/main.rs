//! LemonGalaxy Linux 被控端的图形界面与托盘（docs/linux-desktop.md 第 8 节）。
//!
//! 界面只是被控服务的遥控器：所有操作经本机控制接口（第 7 节）交给服务做，服务没运行时引导安装。
//!
//! ```text
//! lemon-galaxy-worker-gui          打开主界面（已在运行就切到前面）
//! lemon-galaxy-worker-gui --tray   只显示托盘（登录后自启动用）
//! ```

// 消息类型与服务端共用一份源文件
#[path = "../../../lemon-galaxy-common-rust/galaxy-worker/src/control.rs"]
#[allow(dead_code)]
mod control;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod desktop;
mod i18n;

#[cfg(target_os = "linux")]
mod app;
#[cfg(target_os = "linux")]
mod client;
#[cfg(target_os = "linux")]
mod tray;
#[cfg(target_os = "linux")]
mod ui;

fn main() {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("lemon-galaxy-worker-gui {}", galaxy_proto::APP_VERSION);
        return;
    }
    #[cfg(target_os = "linux")]
    std::process::exit(app::run());
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("{}", tr!("lemon-galaxy-worker-gui 只能在 Linux 上运行。", "lemon-galaxy-worker-gui runs on Linux only."));
        std::process::exit(1);
    }
}
