//! LemonGalaxy Windows 被控端。
//!
//! 进程结构：
//! - 服务进程（LocalSystem，`service` 命令由服务管理器启动）：联网、账号、终端、文件、命令执行；
//! - 画面代理（`__lg-session-agent`）：由服务放进当前控制台会话，负责截屏编码、键鼠注入、剪贴板，
//!   经标准输入输出与服务通信。锁屏、UAC 安全桌面时也能看到和操作；
//! - 文件子进程（`__lg-file-helper`）：以账号指定的系统用户身份读写文件。
//!
//! 命令行与 Linux 被控端相同（见 galaxy-worker），另有 `service`、`selftest-capture`。

#[cfg(windows)]
mod win;

#[cfg(windows)]
fn main() {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("lemon-galaxy-worker-windows 只能在 Windows 上运行");
    std::process::exit(1);
}
