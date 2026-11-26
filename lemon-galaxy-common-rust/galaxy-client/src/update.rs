//! 自动更新（客户端一侧）：向自己的服务器查询新版本，验发布签名与 SHA-256 后下载。
//! 服务器只是转发，签名私钥不在服务器上，服务器被攻破也推不了假更新。

use crate::api::Api;
use crate::error::{ClientError, Result};
use base64::Engine;
use futures::StreamExt;
use galaxy_proto::api::UpdateLatestResp;
use galaxy_proto::update::{parse_sums, RELEASE_SIGN_DOMAIN, RELEASE_SIGN_PUBKEY};
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio::io::AsyncWriteExt;

#[derive(Clone, Debug)]
pub struct UpdateInfo {
    pub version: String,
    pub file_name: String,
    pub size: u64,
    pub sha256: String,
    pub url: String,
    pub notes: String,
}

fn enc(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')).collect()
}

/// 下载地址里 `#` 后面记着当初的查询条件（各端把地址当不透明字符串原样传回来），文件被服务器清掉时据此重查。
const QUERY_MARK: &str = "#lg-update?";

/// 查询有没有比 `current` 新的版本。返回前已验证发布签名，并确认文件出现在签名的清单里。
pub async fn check(api: &Api, component: &str, platform: &str, arch: &str, current: &str) -> Result<Option<UpdateInfo>> {
    let r: UpdateLatestResp = api.get(&format!("/updates/latest?component={}&platform={}&arch={}&current={}", enc(component), enc(platform), enc(arch), enc(current))).await?;
    if !r.available {
        return Ok(None);
    }
    let sig = base64::engine::general_purpose::STANDARD.decode(r.signature.trim()).map_err(|_| ClientError::Protocol("更新包签名格式不对".into()))?;
    galaxy_crypto::verify(&RELEASE_SIGN_PUBKEY, RELEASE_SIGN_DOMAIN, r.sums.as_bytes(), &sig).map_err(|_| ClientError::Protocol("更新包签名无效，已拒绝".into()))?;
    let listed = parse_sums(&r.sums);
    if !listed.iter().any(|(n, h)| n == &r.file_name && h.eq_ignore_ascii_case(&r.sha256)) {
        return Err(ClientError::Protocol("更新包不在签名的清单里，已拒绝".into()));
    }
    let url = format!("{}{QUERY_MARK}component={}&platform={}&arch={}&current={}", r.url, enc(component), enc(platform), enc(arch), enc(current));
    Ok(Some(UpdateInfo { version: r.version, file_name: r.file_name, size: r.size, sha256: r.sha256.to_ascii_lowercase(), url, notes: r.notes }))
}

/// 地址里记着的查询条件：(component, platform, arch, current)
fn query_of(url: &str) -> Option<(String, String, String, String)> {
    let q = url.split_once(QUERY_MARK)?.1;
    let get = |k: &str| q.split('&').find_map(|kv| kv.strip_prefix(k).and_then(|v| v.strip_prefix('='))).unwrap_or("").to_string();
    Some((get("component"), get("platform"), get("arch"), get("current")))
}

/// 下载到 `dest`，核对 SHA-256（不一致删掉并报错）。`progress(已下载, 总数)`。
///
/// 服务器只镜像最近几个版本，检查之后又出了新版本时，当初给的文件可能已被清掉（404）：这时按当初的条件重新查一次，
/// 改下最新的那个（同样验过签名、核对 SHA-256）。界面上显示的版本号可能比实际装上的旧，装好后以程序自己的版本为准。
pub async fn download(api: &Api, info: &UpdateInfo, dest: &Path, progress: &(dyn Fn(u64, u64) + Send + Sync)) -> Result<()> {
    match download_one(api, info, dest, progress).await {
        Err(ClientError::NotFound(e)) => {
            let Some((component, platform, arch, current)) = query_of(&info.url) else { return Err(ClientError::NotFound(e)) };
            tracing::info!("更新包 {} 在服务器上已经没有了，重新查询", info.file_name);
            match check(api, &component, &platform, &arch, &current).await? {
                Some(fresh) if fresh.url != info.url => {
                    tracing::info!("改下 {}", fresh.file_name);
                    download_one(api, &fresh, dest, progress).await
                }
                _ => Err(ClientError::NotFound("服务器上已经没有这个版本的更新包，也没有更新的版本".into())),
            }
        }
        r => r,
    }
}

async fn download_one(api: &Api, info: &UpdateInfo, dest: &Path, progress: &(dyn Fn(u64, u64) + Send + Sync)) -> Result<()> {
    let part = dest.with_extension("part");
    let path = info.url.split('#').next().unwrap_or(&info.url);
    let resp = api.get_stream(path).await?;
    let mut f = tokio::fs::File::create(&part).await.map_err(|e| ClientError::Other(e.to_string()))?;
    let mut h = Sha256::new();
    let mut done = 0u64;
    let mut s = resp.bytes_stream();
    while let Some(c) = s.next().await {
        let c = c.map_err(|e| ClientError::Network(e.to_string()))?;
        h.update(&c);
        f.write_all(&c).await.map_err(|e| ClientError::Other(e.to_string()))?;
        done += c.len() as u64;
        progress(done, info.size);
    }
    f.flush().await.map_err(|e| ClientError::Other(e.to_string()))?;
    drop(f);
    let got = hex::encode(h.finalize());
    if got != info.sha256 {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(ClientError::Protocol(format!("下载的更新包校验不对（应为 {}，实际 {got}）", info.sha256)));
    }
    tokio::fs::rename(&part, dest).await.map_err(|e| ClientError::Other(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn query_roundtrip() {
        let url = format!("/api/v1/updates/files/12{}component=control&platform=macos&arch=&current=0.1.0.67", super::QUERY_MARK);
        assert_eq!(super::query_of(&url), Some(("control".into(), "macos".into(), "".into(), "0.1.0.67".into())));
        assert_eq!(super::query_of("/api/v1/updates/files/12"), None);
    }
}
