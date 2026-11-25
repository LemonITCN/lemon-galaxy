//! LemonGalaxy Linux 被控端。命令与说明见 galaxy-worker；进程结构见 docs/linux-desktop.md。

mod desktop;
mod os;

use std::sync::Arc;

fn main() {
    // 一次性打印的命令输出接到 head 等程序时，对方先关管道就安静退出，不报错。
    // 常驻的 run（含 `--data-dir X run` 写法）与内部子进程（__ 开头）保持 Rust 默认的忽略 SIGPIPE：
    // 恢复默认的话，给已断开的对端（本机控制接口的界面、管道）写数据会让进程被系统直接杀掉、不留日志
    if !std::env::args().skip(1).any(|a| a == "run" || a.starts_with("__")) {
        unsafe {
            signal(13, 0);
        }
    }
    galaxy_worker::main(Arc::new(os::LinuxOs));
}

extern "C" {
    /// signal(SIGPIPE=13, SIG_DFL=0)
    fn signal(sig: i32, handler: usize) -> usize;
}

