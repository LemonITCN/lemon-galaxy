//! LemonGalaxy Windows 被控端桌面端的原生版（Rust + Win32），给 XP SP3 / Vista / 7 用：不依赖 .NET，双击就能用。
//! 功能与 WPF 版（`lemon-galaxy-worker-windows/gui`）、Linux 版（`lemon-galaxy-worker-linux/gui`）对齐，
//! 界面只是被控服务的遥控器：所有操作经本机控制接口（命名管道，docs/spec.md 3.0.2）交给服务做。
//!
//! ```text
//! LemonGalaxyWorker.exe                         打开主窗口（已在运行就把窗口切到前面）
//! LemonGalaxyWorker.exe --tray                  登录后自启：只放托盘（本用户关了「登录后在托盘显示」时直接退出）
//! LemonGalaxyWorker.exe --replace               接替同一位置正在运行的旧实例（安装、更新后用）
//! LemonGalaxyWorker.exe --version               打印版本
//! 以管理员身份运行的子命令（界面经 UAC 调用；XP 上是管理员就直接运行）：
//!   --elevate <钥匙> --pipe <管道>               把钥匙交给服务，授权界面那个连接几分钟内直接改配置
//!   --install [--no-autostart] [--no-tray]      安装（复制到 Program Files、装服务、托盘自启、开始菜单、卸载入口）
//!   --start-service                             启动服务
//!   --uninstall [--confirmed] [--keep <进程号>]  卸载（「添加或删除程序」里调用；没提权时自己提权）
//!   以上都可带 --result <文件>：失败原因写进这个文件（提权后的进程可能是另一个账号，读不到调用方的输出）
//! ```

#![cfg_attr(windows, windows_subsystem = "windows")]

// 消息类型与服务端共用一份源文件
#[path = "../../../lemon-galaxy-common-rust/galaxy-worker/src/control.rs"]
#[allow(dead_code)]
mod control;
#[cfg_attr(not(windows), allow(dead_code))]
mod i18n;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod client;
#[cfg(windows)]
mod elevation;
#[cfg(windows)]
mod pipe;
#[cfg(windows)]
mod settings;
#[cfg(windows)]
mod setup;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
mod ui;
#[cfg(windows)]
mod win;

fn main() {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("LemonGalaxyWorker {}", galaxy_proto::APP_VERSION);
        return;
    }
    #[cfg(windows)]
    std::process::exit(app::main());
    #[cfg(not(windows))]
    {
        eprintln!("{}", tr!("LemonGalaxyWorker 只能在 Windows 上运行。", "LemonGalaxyWorker runs on Windows only."));
        std::process::exit(1);
    }
}
