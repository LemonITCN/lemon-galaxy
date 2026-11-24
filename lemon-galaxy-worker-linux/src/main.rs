//! LemonGalaxy Linux 被控端。命令与说明见 galaxy-worker；进程结构见 docs/linux-desktop.md。

mod desktop;
mod os;

use std::sync::Arc;

fn main() {
    // 命令行输出接到 head 等程序时，对方先关管道就安静退出，不报错
    if std::env::args().nth(1).map(|c| c != "run").unwrap_or(true) {
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

