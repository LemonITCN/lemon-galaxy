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
    Ok(Some(UpdateInfo { version: r.version, file_name: r.file_name, size: r.size, sha256: r.sha256.to_ascii_lowercase(), url: r.url, notes: r.notes }))
}

/// 下载到 `dest`，核对 SHA-256（不一致删掉并报错）。`progress(已下载, 总数)`。
pub async fn download(api: &Api, info: &UpdateInfo, dest: &Path, progress: &(dyn Fn(u64, u64) + Send + Sync)) -> Result<()> {
    let part = dest.with_extension("part");
    let resp = api.get_stream(&info.url).await?;
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
