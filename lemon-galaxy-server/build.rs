// 管理后台前端没构建时放一个占位目录，保证 rust-embed 能编译。
fn main() {
    let dist = std::path::Path::new("web/dist");
    if !dist.exists() {
        let _ = std::fs::create_dir_all(dist);
    }
    println!("cargo:rerun-if-changed=web/dist");
}
