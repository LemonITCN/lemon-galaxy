// 完整版本号：CI 构建时为标签版本（LEMONGALAXY_BUILD_VERSION）或「Cargo 版本.构建序号」（LEMONGALAXY_BUILD_NUMBER），
// 本机构建为 Cargo 版本。自动更新靠它比较新旧，发布文件名用的是同一个版本号。
fn main() {
    println!("cargo:rerun-if-env-changed=LEMONGALAXY_BUILD_VERSION");
    println!("cargo:rerun-if-env-changed=LEMONGALAXY_BUILD_NUMBER");
    let base = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let tag = std::env::var("LEMONGALAXY_BUILD_VERSION").unwrap_or_default();
    let tag = tag.trim().trim_start_matches('v');
    let num = std::env::var("LEMONGALAXY_BUILD_NUMBER").unwrap_or_default();
    let v = if !tag.is_empty() {
        tag.to_string()
    } else if !num.trim().is_empty() {
        format!("{base}.{}", num.trim())
    } else {
        base
    };
    println!("cargo:rustc-env=LEMONGALAXY_APP_VERSION={v}");
}
