//! 自动更新分发：定期从 GitHub 拉最新发布，验发布签名、逐个文件核对 SHA-256 后镜像到本地；
//! 客户端只从自己的服务器查询和下载（内网、访问不了 GitHub 的环境也能更新），安装前自己再验一遍。

use crate::error::{AppError, AppResult};
use crate::state::St;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Json;
use base64::Engine;
use futures::StreamExt;
use galaxy_proto::api::UpdateLatestResp;
use galaxy_proto::update::{classify_asset, parse_sums, version_newer, RELEASE_SIGN_DOMAIN, RELEASE_SIGN_PUBKEY, SUMS_FILE, SUMS_SIG_FILE};
use parking_lot::Mutex;
use r2d2_sqlite::rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// 保留最近几个版本的镜像。
const KEEP_VERSIONS: usize = 2;

#[derive(Default)]
pub struct UpdateStatus {
    pub last_check_at: AtomicI64,
    pub last_error: Mutex<String>,
    pub latest_version: Mutex<String>,
    running: tokio::sync::Mutex<()>,
}

fn updates_dir(st: &St) -> PathBuf {
    st.data_dir.join("updates")
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    size: u64,
    browser_download_url: String,
}

/// 后台定期检查。
pub async fn run(st: St) {
    tokio::time::sleep(Duration::from_secs(60)).await;
    loop {
        let (source, hours) = {
            let s = st.settings.read();
            (s.update_source.clone(), s.update_check_hours.max(1))
        };
        if source == "github" {
            if let Err(e) = check(&st).await {
                tracing::warn!("检查更新失败：{e}");
            }
        }
        tokio::time::sleep(Duration::from_secs(hours as u64 * 3600)).await;
    }
}

async fn fetch_text(st: &St, url: &str) -> anyhow::Result<Option<String>> {
    let r = st.http.get(url).timeout(Duration::from_secs(60)).send().await?;
    if r.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    Ok(Some(r.error_for_status()?.text().await?))
}

/// 立即检查一次。返回新镜像的文件数。
pub async fn check(st: &St) -> anyhow::Result<usize> {
    let _g = st.updates.running.lock().await;
    let r = check_inner(st).await;
    st.updates.last_check_at.store(galaxy_proto::now_ms(), Ordering::Relaxed);
    *st.updates.last_error.lock() = match &r {
        Ok(_) => String::new(),
        Err(e) => e.to_string(),
    };
    r
}

async fn check_inner(st: &St) -> anyhow::Result<usize> {
    let repo = st.settings.read().update_github_repo.clone();
    anyhow::ensure!(repo.split('/').count() == 2 && !repo.contains(".."), "仓库名不对：{repo}");
    let rel: GhRelease = st
        .http
        .get(format!("https://api.github.com/repos/{repo}/releases/latest"))
        .header("accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(30))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let version = rel.tag_name.trim_start_matches('v').to_string();
    anyhow::ensure!(!version.is_empty() && version.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'), "版本号不对：{}", rel.tag_name);
    *st.updates.latest_version.lock() = version.clone();
    let url_of = |n: &str| rel.assets.iter().find(|a| a.name == n).map(|a| a.browser_download_url.clone());
    let sums_url = url_of(SUMS_FILE).ok_or_else(|| anyhow::anyhow!("发布 {version} 没有 {SUMS_FILE}"))?;
    let sig_url = url_of(SUMS_SIG_FILE).ok_or_else(|| anyhow::anyhow!("发布 {version} 没有签名，客户端不会安装未签名的更新，已跳过"))?;
    let sums = fetch_text(st, &sums_url).await?.ok_or_else(|| anyhow::anyhow!("取不到 {SUMS_FILE}"))?;
    let sig_b64 = fetch_text(st, &sig_url).await?.ok_or_else(|| anyhow::anyhow!("取不到签名"))?.trim().to_string();
    let sig = base64::engine::general_purpose::STANDARD.decode(&sig_b64)?;
    galaxy_crypto::verify(&RELEASE_SIGN_PUBKEY, RELEASE_SIGN_DOMAIN, sums.as_bytes(), &sig).map_err(|_| anyhow::anyhow!("发布 {version} 的签名无效，已拒绝"))?;
    let listed = parse_sums(&sums);

    let dir = updates_dir(st).join(&version);
    tokio::fs::create_dir_all(&dir).await?;
    tokio::fs::write(dir.join(SUMS_FILE), &sums).await?;
    tokio::fs::write(dir.join(SUMS_SIG_FILE), &sig_b64).await?;
    tokio::fs::write(dir.join("notes.md"), &rel.body).await?;

    let mut added = 0;
    for a in &rel.assets {
        let Some((component, platform, arch)) = classify_asset(&a.name) else { continue };
        let Some(want) = listed.iter().find(|(n, _)| n == &a.name).map(|(_, h)| h.clone()) else { continue };
        let (v2, n2) = (version.clone(), a.name.clone());
        let exists: Option<i64> = st.db.call(move |c| c.query_row("SELECT id FROM update_packages WHERE version = ?1 AND file_name = ?2", params![v2, n2], |r| r.get(0)).optional()).await?;
        let path = dir.join(&a.name);
        if exists.is_some() && tokio::fs::metadata(&path).await.map(|m| m.len() == a.size).unwrap_or(false) {
            continue;
        }
        download(st, &a.browser_download_url, &path, &want).await.map_err(|e| anyhow::anyhow!("{}：{e}", a.name))?;
        let (v, n, comp, plat, ar, size, sb) = (version.clone(), a.name.clone(), component.to_string(), platform.to_string(), arch.clone(), a.size as i64, sig_b64.clone());
        let now = galaxy_proto::now_ms();
        st.db
            .call(move |c| {
                c.execute("DELETE FROM update_packages WHERE version = ?1 AND file_name = ?2", params![v, n])?;
                c.execute(
                    "INSERT INTO update_packages(component, platform, arch, version, file_name, size, sha256, signature, signature_ok, source, created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,1,'github',?9)",
                    params![comp, plat, ar, v, n, size, want, sb, now],
                )
            })
            .await?;
        added += 1;
        tracing::info!("已镜像更新包 {}", a.name);
    }
    prune(st).await;
    Ok(added)
}

async fn download(st: &St, url: &str, dest: &std::path::Path, want_sha256: &str) -> anyhow::Result<()> {
    let part = dest.with_extension("part");
    let mut f = tokio::fs::File::create(&part).await?;
    let r = st.http.get(url).timeout(Duration::from_secs(1800)).send().await?.error_for_status()?;
    let mut s = r.bytes_stream();
    let mut h = Sha256::new();
    while let Some(c) = s.next().await {
        let c = c?;
        h.update(&c);
        f.write_all(&c).await?;
    }
    f.flush().await?;
    drop(f);
    let got = hex::encode(h.finalize());
    if got != want_sha256 {
        let _ = tokio::fs::remove_file(&part).await;
        anyhow::bail!("SHA-256 不符（清单 {want_sha256}，实际 {got}）");
    }
    tokio::fs::rename(&part, dest).await?;
    Ok(())
}

/// 只留最近几个版本。
async fn prune(st: &St) {
    let versions: Vec<String> = st
        .db
        .call(|c| {
            let mut stmt = c.prepare("SELECT DISTINCT version FROM update_packages WHERE source = 'github'")?;
            let v = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
            Ok(v)
        })
        .await
        .unwrap_or_default();
    let mut sorted = versions.clone();
    sorted.sort_by(|a, b| if version_newer(a, b) { std::cmp::Ordering::Less } else if version_newer(b, a) { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Equal });
    for old in sorted.into_iter().skip(KEEP_VERSIONS) {
        let o = old.clone();
        let _ = st.db.call(move |c| c.execute("DELETE FROM update_packages WHERE version = ?1 AND source = 'github'", params![o])).await;
        let _ = tokio::fs::remove_dir_all(updates_dir(st).join(&old)).await;
    }
}

#[derive(Deserialize)]
pub struct LatestQuery {
    component: String,
    platform: String,
    #[serde(default)]
    arch: String,
    #[serde(default)]
    current: String,
}

struct Pkg {
    id: i64,
    version: String,
    file_name: String,
    size: i64,
    sha256: String,
    arch: String,
}

/// 客户端查询有没有新版本。
pub async fn latest(State(st): State<St>, _u: crate::auth::User, Query(q): Query<LatestQuery>) -> AppResult<Json<UpdateLatestResp>> {
    let (comp, plat) = (q.component.clone(), q.platform.clone());
    let rows: Vec<Pkg> = st
        .db
        .call(move |c| {
            let mut stmt = c.prepare("SELECT id, version, file_name, size, sha256, arch FROM update_packages WHERE component = ?1 AND platform = ?2 AND signature_ok = 1")?;
            let v = stmt
                .query_map(params![comp, plat], |r| Ok(Pkg { id: r.get(0)?, version: r.get(1)?, file_name: r.get(2)?, size: r.get(3)?, sha256: r.get(4)?, arch: r.get(5)? }))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(v)
        })
        .await?;
    let best = rows.into_iter().filter(|p| p.arch == "universal" || q.arch.is_empty() || p.arch == q.arch).reduce(|a, b| if version_newer(&b.version, &a.version) { b } else { a });
    let none = UpdateLatestResp { available: false, version: String::new(), url: String::new(), file_name: String::new(), size: 0, sha256: String::new(), sums: String::new(), signature: String::new(), notes: String::new() };
    let Some(p) = best else { return Ok(Json(none)) };
    if !q.current.is_empty() && !version_newer(&p.version, &q.current) {
        return Ok(Json(none));
    }
    let dir = updates_dir(&st).join(&p.version);
    let sums = tokio::fs::read_to_string(dir.join(SUMS_FILE)).await.unwrap_or_default();
    let signature = tokio::fs::read_to_string(dir.join(SUMS_SIG_FILE)).await.unwrap_or_default().trim().to_string();
    let notes = tokio::fs::read_to_string(dir.join("notes.md")).await.unwrap_or_default();
    Ok(Json(UpdateLatestResp {
        available: true,
        version: p.version,
        url: format!("/api/v1/updates/files/{}", p.id),
        file_name: p.file_name,
        size: p.size as u64,
        sha256: p.sha256,
        sums,
        signature,
        notes,
    }))
}

/// 下载更新文件。
pub async fn file(State(st): State<St>, _u: crate::auth::User, Path(id): Path<i64>) -> AppResult<Response> {
    let row: Option<(String, String)> = st.db.call(move |c| c.query_row("SELECT version, file_name FROM update_packages WHERE id = ?1", params![id], |r| Ok((r.get(0)?, r.get(1)?))).optional()).await?;
    let Some((version, name)) = row else { return Err(AppError::new(StatusCode::NOT_FOUND, "not_found")) };
    let path = updates_dir(&st).join(&version).join(&name);
    let f = tokio::fs::File::open(&path).await.map_err(|_| AppError::new(StatusCode::NOT_FOUND, "not_found"))?;
    let len = f.metadata().await.map(|m| m.len()).unwrap_or(0);
    let body = Body::from_stream(tokio_util::io::ReaderStream::with_capacity(f, 256 * 1024));
    Response::builder()
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, len)
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\""))
        .body(body)
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal").detail(e.to_string()))
}
