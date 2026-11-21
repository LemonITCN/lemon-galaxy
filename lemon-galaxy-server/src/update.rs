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

/// 从发布文件名里取版本号（第一个形如 0.1.0.5 的段）。
fn version_in_name(name: &str) -> Option<String> {
    name.split(['-', '_']).find(|p| p.contains('.') && p.split('.').all(|x| !x.is_empty() && x.chars().all(|c| c.is_ascii_digit()))).map(|s| s.to_string())
}

/// 后台手动上传：更新文件 + 同一版的 SHA256SUMS.txt + SHA256SUMS.txt.sig（服务器连不上 GitHub 时用）。
pub async fn upload(State(st): State<St>, crate::auth::Admin(s): crate::auth::Admin, mut mp: axum::extract::Multipart) -> AppResult<Json<serde_json::Value>> {
    let bad = |m: String| AppError::bad_request(m).lang(s.lang);
    let tmp_dir = updates_dir(&st).join(".upload");
    tokio::fs::create_dir_all(&tmp_dir).await.map_err(|e| bad(e.to_string()))?;
    let tmp = tmp_dir.join(format!("{}", galaxy_proto::now_ms()));
    let (mut name, mut got_sha, mut size) = (String::new(), String::new(), 0u64);
    let (mut sums, mut sig_b64) = (String::new(), String::new());
    while let Some(mut field) = mp.next_field().await.map_err(|e| bad(e.to_string()))? {
        match field.name().unwrap_or("") {
            "file" => {
                name = field.file_name().unwrap_or("").rsplit(['/', '\\']).next().unwrap_or("").to_string();
                let mut f = tokio::fs::File::create(&tmp).await.map_err(|e| bad(e.to_string()))?;
                let mut h = Sha256::new();
                while let Some(c) = field.chunk().await.map_err(|e| bad(e.to_string()))? {
                    h.update(&c);
                    size += c.len() as u64;
                    f.write_all(&c).await.map_err(|e| bad(e.to_string()))?;
                }
                f.flush().await.map_err(|e| bad(e.to_string()))?;
                got_sha = hex::encode(h.finalize());
            }
            "sums" => sums = field.text().await.map_err(|e| bad(e.to_string()))?,
            "signature" => sig_b64 = field.text().await.map_err(|e| bad(e.to_string()))?.trim().to_string(),
            _ => {}
        }
    }
    let r = async {
        let sig = base64::engine::general_purpose::STANDARD.decode(&sig_b64).map_err(|_| "签名文件格式不对".to_string())?;
        galaxy_crypto::verify(&RELEASE_SIGN_PUBKEY, RELEASE_SIGN_DOMAIN, sums.as_bytes(), &sig).map_err(|_| "清单签名无效（不是 LemonGalaxy 官方发布，或清单与签名不是同一版）".to_string())?;
        let want = parse_sums(&sums).into_iter().find(|(n, _)| n == &name).map(|(_, h)| h).ok_or_else(|| format!("{name} 不在校验清单里"))?;
        if want != got_sha {
            return Err(format!("{name} 的 SHA-256 与清单不符"));
        }
        let (component, platform, arch) = classify_asset(&name).ok_or_else(|| format!("认不出 {name} 是哪个端的更新包"))?;
        let version = version_in_name(&name).ok_or_else(|| format!("{name} 里没有版本号"))?;
        Ok::<_, String>((component, platform, arch, version))
    }
    .await;
    let (component, platform, arch, version) = match r {
        Ok(x) => x,
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(bad(e));
        }
    };
    let dir = updates_dir(&st).join(&version);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| bad(e.to_string()))?;
    tokio::fs::rename(&tmp, dir.join(&name)).await.map_err(|e| bad(e.to_string()))?;
    let _ = tokio::fs::write(dir.join(SUMS_FILE), &sums).await;
    let _ = tokio::fs::write(dir.join(SUMS_SIG_FILE), &sig_b64).await;
    let now = galaxy_proto::now_ms();
    let (v, n, comp, plat, ar, sha, sb) = (version.clone(), name.clone(), component.to_string(), platform.to_string(), arch.clone(), got_sha.clone(), sig_b64.clone());
    let id: i64 = st
        .db
        .call(move |c| {
            c.execute("DELETE FROM update_packages WHERE version = ?1 AND file_name = ?2", params![v, n])?;
            c.execute(
                "INSERT INTO update_packages(component, platform, arch, version, file_name, size, sha256, signature, signature_ok, source, created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,1,'manual',?9)",
                params![comp, plat, ar, v, n, size as i64, sha, sb, now],
            )?;
            Ok(c.last_insert_rowid())
        })
        .await?;
    crate::audit::log(&st, &s.actor(), "update.upload", crate::audit::Target::new("update", id, &name), serde_json::json!({ "version": version, "size": size })).await;
    Ok(Json(serde_json::json!({
        "id": id, "component": component, "platform": platform, "arch": arch, "version": version, "file_name": name,
        "size": size, "sha256": got_sha, "signature_ok": true, "source": "manual", "created_at": now,
    })))
}

/// 本机服务器的发布文件名，以及包里的程序名（只发布 Linux 与 Windows 版）。
pub fn server_asset(version: &str, os: &str, arch: &str) -> Option<(String, &'static str)> {
    match os {
        "linux" => Some((format!("lemon-galaxy-server-{version}-linux-{arch}.tar.gz"), "lemon-galaxy-server")),
        "windows" => Some((format!("lemon-galaxy-server-{version}-windows-{arch}.zip"), "lemon-galaxy-server.exe")),
        _ => None,
    }
}

pub struct SelfUpdated {
    pub version: String,
    /// 被替换的程序（只检查时为空）
    pub path: PathBuf,
}

/// 命令行 `self-update`：服务器自己升级（从 GitHub 取最新发布，验发布签名与 SHA-256，替换程序）。
/// 装了系统服务就替换服务用的那一份，之后由调用方重启服务。
pub async fn self_update(repo: &str, check_only: bool) -> anyhow::Result<Option<SelfUpdated>> {
    let http = reqwest::Client::builder().user_agent(format!("LemonGalaxy/{}", galaxy_proto::APP_VERSION)).connect_timeout(Duration::from_secs(15)).build()?;
    let rel: GhRelease = http.get(format!("https://api.github.com/repos/{repo}/releases/latest")).header("accept", "application/vnd.github+json").send().await?.error_for_status()?.json().await?;
    let version = rel.tag_name.trim_start_matches('v').to_string();
    if !version_newer(&version, galaxy_proto::APP_VERSION) {
        return Ok(None);
    }
    if check_only {
        return Ok(Some(SelfUpdated { version, path: PathBuf::new() }));
    }
    let (name, program) = server_asset(&version, std::env::consts::OS, galaxy_proto::update::this_arch()).ok_or_else(|| anyhow::anyhow!("自动升级只支持 Linux 与 Windows 版服务器，其他系统请手动下载替换"))?;
    let url_of = |n: &str| rel.assets.iter().find(|a| a.name == n).map(|a| a.browser_download_url.clone());
    let asset = url_of(&name).ok_or_else(|| anyhow::anyhow!("发布 {version} 里没有 {name}"))?;
    let sums = http.get(url_of(SUMS_FILE).ok_or_else(|| anyhow::anyhow!("没有 {SUMS_FILE}"))?).send().await?.error_for_status()?.text().await?;
    let sig_b64 = http.get(url_of(SUMS_SIG_FILE).ok_or_else(|| anyhow::anyhow!("发布 {version} 没有签名，不升级"))?).send().await?.error_for_status()?.text().await?;
    let sig = base64::engine::general_purpose::STANDARD.decode(sig_b64.trim())?;
    galaxy_crypto::verify(&RELEASE_SIGN_PUBKEY, RELEASE_SIGN_DOMAIN, sums.as_bytes(), &sig).map_err(|_| anyhow::anyhow!("发布签名无效，已拒绝"))?;
    let want = parse_sums(&sums).into_iter().find(|(n, _)| n == &name).map(|(_, h)| h).ok_or_else(|| anyhow::anyhow!("{name} 不在清单里"))?;
    let bytes = http.get(asset).timeout(Duration::from_secs(1800)).send().await?.error_for_status()?.bytes().await?;
    anyhow::ensure!(hex::encode(Sha256::digest(&bytes)) == want, "下载的文件校验不对");
    let bin = extract_program(&name, &bytes, program)?;
    let target = match crate::service::update_target() {
        Some(p) => p,
        None => std::env::current_exe()?,
    };
    replace_program(&target, &bin).map_err(|e| {
        let hint = if e.kind() == std::io::ErrorKind::PermissionDenied { "（需要管理员 / root 权限）" } else { "" };
        anyhow::anyhow!("替换 {} 失败：{e}{hint}", target.display())
    })?;
    Ok(Some(SelfUpdated { version, path: target }))
}

/// 从发布包（.tar.gz 或 .zip）里取出程序。
fn extract_program(asset: &str, bytes: &[u8], program: &str) -> anyhow::Result<Vec<u8>> {
    if asset.ends_with(".zip") {
        return zip_extract(bytes, program);
    }
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    for e in ar.entries()? {
        let mut e = e?;
        if e.path()?.file_name().map(|n| n == program).unwrap_or(false) {
            let mut v = Vec::new();
            std::io::Read::read_to_end(&mut e, &mut v)?;
            return Ok(v);
        }
    }
    anyhow::bail!("压缩包里没有 {program}")
}

/// 替换程序：先写 .new 再换上。Windows 上运行中的程序不能覆盖但能改名，旧的改成 .old，下次启动时删掉。
fn replace_program(target: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    let new = target.with_extension("new");
    std::fs::write(&new, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(windows)]
    {
        let old = target.with_extension("old");
        let _ = std::fs::remove_file(&old);
        if target.exists() {
            if let Err(e) = std::fs::rename(target, &old) {
                let _ = std::fs::remove_file(&new);
                return Err(e);
            }
        }
        if let Err(e) = std::fs::rename(&new, target) {
            let _ = std::fs::rename(&old, target);
            return Err(e);
        }
        Ok(())
    }
    #[cfg(not(windows))]
    std::fs::rename(&new, target)
}

fn le16(b: &[u8], at: usize) -> Option<usize> {
    b.get(at..at.checked_add(2)?).map(|s| u16::from_le_bytes([s[0], s[1]]) as usize)
}

fn le32(b: &[u8], at: usize) -> Option<usize> {
    b.get(at..at.checked_add(4)?).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize)
}

/// 从 zip 里取出一个文件（按文件名，忽略目录）。只支持发布包用到的格式：存储或 deflate，不含 zip64。
pub fn zip_extract(data: &[u8], want: &str) -> anyhow::Result<Vec<u8>> {
    let bad = || anyhow::anyhow!("压缩包格式不对");
    // 中央目录结束记录在末尾，后面最多跟 64 KB 注释
    let last = data.len().checked_sub(22).ok_or_else(bad)?;
    let first = last.saturating_sub(65535);
    let eocd = (first..=last).rev().find(|&i| le32(data, i) == Some(0x0605_4b50)).ok_or_else(bad)?;
    let count = le16(data, eocd + 10).ok_or_else(bad)?;
    let mut p = le32(data, eocd + 16).ok_or_else(bad)?;
    for _ in 0..count {
        anyhow::ensure!(le32(data, p) == Some(0x0201_4b50), "压缩包格式不对");
        let method = le16(data, p + 10).ok_or_else(bad)?;
        let crc = le32(data, p + 16).ok_or_else(bad)? as u32;
        let packed = le32(data, p + 20).ok_or_else(bad)?;
        let size = le32(data, p + 24).ok_or_else(bad)?;
        let name_len = le16(data, p + 28).ok_or_else(bad)?;
        let extra_len = le16(data, p + 30).ok_or_else(bad)?;
        let comment_len = le16(data, p + 32).ok_or_else(bad)?;
        let local = le32(data, p + 42).ok_or_else(bad)?;
        let name = String::from_utf8_lossy(data.get(p + 46..p + 46 + name_len).ok_or_else(bad)?).into_owned();
        p += 46 + name_len + extra_len + comment_len;
        if name.rsplit(['/', '\\']).next() != Some(want) {
            continue;
        }
        anyhow::ensure!(le32(data, local) == Some(0x0403_4b50), "压缩包格式不对");
        let start = local + 30 + le16(data, local + 26).ok_or_else(bad)? + le16(data, local + 28).ok_or_else(bad)?;
        let raw = data.get(start..start + packed).ok_or_else(bad)?;
        let out = match method {
            0 => raw.to_vec(),
            8 => {
                let mut v = Vec::with_capacity(size);
                std::io::Read::read_to_end(&mut flate2::read::DeflateDecoder::new(raw), &mut v)?;
                v
            }
            m => anyhow::bail!("{want} 用了不支持的压缩方式 {m}"),
        };
        let mut c = flate2::Crc::new();
        c.update(&out);
        anyhow::ensure!(out.len() == size && c.sum() == crc, "压缩包里的 {want} 校验不对");
        return Ok(out);
    }
    anyhow::bail!("压缩包里没有 {want}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn version_from_name() {
        assert_eq!(version_in_name("lemon-galaxy-worker-0.1.0.5-linux-x86_64").as_deref(), Some("0.1.0.5"));
        assert_eq!(version_in_name("LemonGalaxy-0.2.0-macOS.dmg").as_deref(), Some("0.2.0"));
        assert_eq!(version_in_name("nothing"), None);
    }

    #[test]
    fn server_assets_are_classified() {
        for (os, arch) in [("linux", "x86_64"), ("linux", "aarch64"), ("windows", "x86_64"), ("windows", "aarch64")] {
            let (name, _) = server_asset("0.1.0.42", os, arch).unwrap();
            assert_eq!(classify_asset(&name), Some(("server", os, arch.to_string())), "{name}");
            assert_eq!(version_in_name(&name).as_deref(), Some("0.1.0.42"));
        }
        assert_eq!(server_asset("0.1.0", "macos", "aarch64"), None);
    }

    /// 写一个最简单的 zip（每项可选存储或 deflate），与 Info-ZIP 的布局一致。
    fn make_zip(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let (mut out, mut cd) = (Vec::new(), Vec::new());
        for (name, data, deflate) in entries {
            let packed = if *deflate {
                let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(data).unwrap();
                e.finish().unwrap()
            } else {
                data.to_vec()
            };
            let mut crc = flate2::Crc::new();
            crc.update(data);
            let method: u16 = if *deflate { 8 } else { 0 };
            let off = out.len() as u32;
            out.extend(0x0403_4b50u32.to_le_bytes());
            out.extend([20u16, 0, method, 0, 0].iter().flat_map(|v| v.to_le_bytes()));
            out.extend([crc.sum(), packed.len() as u32, data.len() as u32].iter().flat_map(|v| v.to_le_bytes()));
            out.extend([name.len() as u16, 0].iter().flat_map(|v| v.to_le_bytes()));
            out.extend(name.as_bytes());
            out.extend(&packed);
            cd.extend(0x0201_4b50u32.to_le_bytes());
            cd.extend([20u16, 20, 0, method, 0, 0].iter().flat_map(|v| v.to_le_bytes()));
            cd.extend([crc.sum(), packed.len() as u32, data.len() as u32].iter().flat_map(|v| v.to_le_bytes()));
            cd.extend([name.len() as u16, 0, 0, 0, 0].iter().flat_map(|v| v.to_le_bytes()));
            cd.extend([0u32, off].iter().flat_map(|v| v.to_le_bytes()));
            cd.extend(name.as_bytes());
        }
        let cd_off = out.len() as u32;
        out.extend(&cd);
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend([0u16, 0, entries.len() as u16, entries.len() as u16].iter().flat_map(|v| v.to_le_bytes()));
        out.extend([cd.len() as u32, cd_off].iter().flat_map(|v| v.to_le_bytes()));
        out.extend(0u16.to_le_bytes());
        out
    }

    #[test]
    fn zip_roundtrip() {
        let exe: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let z = make_zip(&[("readme.txt", b"hello", false), ("dir/lemon-galaxy-server.exe", &exe, true)]);
        assert_eq!(zip_extract(&z, "lemon-galaxy-server.exe").unwrap(), exe);
        assert_eq!(zip_extract(&z, "readme.txt").unwrap(), b"hello");
        assert!(zip_extract(&z, "other.exe").is_err());
        assert_eq!(extract_program("x.zip", &z, "lemon-galaxy-server.exe").unwrap(), exe);
        // 内容被改过：CRC 对不上
        let mut broken = z.clone();
        let i = broken.len() / 2;
        broken[i] ^= 0xff;
        assert!(zip_extract(&broken, "lemon-galaxy-server.exe").is_err());
        // 截断、垃圾数据不崩
        assert!(zip_extract(&z[..z.len() - 30], "readme.txt").is_err());
        assert!(zip_extract(b"not a zip", "readme.txt").is_err());
        assert!(zip_extract(&[], "readme.txt").is_err());
    }

    #[test]
    fn tar_gz_extract() {
        let mut b = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default()));
        let mut h = tar::Header::new_gnu();
        h.set_size(4);
        h.set_mode(0o755);
        h.set_cksum();
        b.append_data(&mut h, "lemon-galaxy-server", &b"ELF!"[..]).unwrap();
        let gz = b.into_inner().unwrap().finish().unwrap();
        assert_eq!(extract_program("a.tar.gz", &gz, "lemon-galaxy-server").unwrap(), b"ELF!");
        assert!(extract_program("a.tar.gz", &gz, "other").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn replace_program_swaps_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("lg-replace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let t = dir.join("lemon-galaxy-server");
        std::fs::write(&t, b"old").unwrap();
        replace_program(&t, b"new").unwrap();
        assert_eq!(std::fs::read(&t).unwrap(), b"new");
        assert_eq!(std::fs::metadata(&t).unwrap().permissions().mode() & 0o777, 0o755);
        assert!(!t.with_extension("new").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
