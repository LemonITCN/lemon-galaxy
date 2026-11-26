// XP 兼容版把 XP 虚拟屏驱动（driver/xp，llvm-mingw 编）嵌进 exe：单文件 CLI 也能装驱动。
// 编译时设 LEMON_GALAXY_XP_DRIVER_DIR 指到 driver/xp/build.sh 的输出目录；没设时嵌进去的是空文件（不带驱动）。
use std::path::{Path, PathBuf};

const FILES: [&str; 3] = ["lgxpmp.sys", "lgxpdd.dll", "LemonGalaxyDisplayXP.inf"];

fn main() {
    println!("cargo:rerun-if-env-changed=LEMON_GALAXY_XP_DRIVER_DIR");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let dir = std::env::var("LEMON_GALAXY_XP_DRIVER_DIR").ok().filter(|d| !d.is_empty());
    for f in FILES {
        let data = match &dir {
            Some(d) => {
                let p = Path::new(d).join(f);
                println!("cargo:rerun-if-changed={}", p.display());
                std::fs::read(&p).unwrap_or_else(|e| panic!("读不到 XP 虚拟屏驱动文件 {}：{e}", p.display()))
            }
            None => Vec::new(),
        };
        std::fs::write(out.join(f), data).expect("写 OUT_DIR");
    }
}
