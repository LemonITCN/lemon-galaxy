#![allow(deprecated)]
//! 管理后台接口 /api/v1/admin/*，见 docs/api.md 第一部分。

use crate::audit::{self, Target};
use crate::auth::{new_token, token_hash, Admin, AdminAny, Ctx, ADMIN_COOKIE};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, St};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use galaxy_proto::ws::WsMsg;
use galaxy_proto::{AiMode, DeviceId};
use r2d2_sqlite::rusqlite::{params, OptionalExtension, Row};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

pub fn routes() -> Router<St> {
    Router::new()
        .route("/setup/status", get(setup_status))
        .route("/setup", post(setup))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/me", get(me))
        .route("/totp/setup", post(totp_setup))
        .route("/totp/verify", post(totp_verify))
        .route("/password", post(change_password))
        .route("/admins", get(list_admins).post(create_admin))
        .route("/admins/{id}", axum::routing::delete(delete_admin))
        .route("/admins/{id}/reset-totp", post(reset_admin_totp))
        .route("/server-info", get(server_info))
        .route("/stats/timeline", get(stats_timeline))
        .route("/users", get(list_users).post(create_user))
        .route("/users/{id}", get(get_user).patch(update_user).delete(delete_user))
        .route("/users/{id}/reset-password", post(reset_user_password))
        .route("/users/{id}/reset-totp", post(reset_user_totp))
        .route("/users/{id}/revoke-sessions", post(revoke_user_sessions))
        .route("/users/{id}/ai-grants", get(get_grants).put(put_grants))
        .route("/devices", get(list_devices))
        .route("/devices/{id}", get(get_device).delete(delete_device))
        .route("/devices/{id}/revoke", post(revoke_device))
        .route("/connections", get(list_connections))
        .route("/connections/{id}/terminate", post(terminate_connection))
        .route("/ai/providers", get(crate::ai::admin_list_providers).post(crate::ai::admin_create_provider))
        .route(
            "/ai/providers/{id}",
            axum::routing::patch(crate::ai::admin_update_provider).delete(crate::ai::admin_delete_provider),
        )
        .route("/ai/providers/{id}/models", put(crate::ai::admin_set_models))
        .route("/ai/providers/{id}/refresh-models", post(crate::ai::admin_refresh_models))
        .route("/ai/providers/{id}/test", post(crate::ai::admin_test_provider))
        .route("/ai/usage", get(crate::ai::admin_usage))
        .route("/audit", get(list_audit))
        .route("/settings", get(get_settings).patch(patch_settings))
        .route("/updates", get(list_updates))
        .route("/updates/check", post(check_updates))
        .route("/updates/upload", post(upload_update))
        .route("/updates/{id}", axum::routing::delete(delete_update))
}

// ---------------- 工具 ----------------

pub fn valid_username(s: &str) -> bool {
    let n = s.chars().count();
    (3..=32).contains(&n) && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

pub fn hash_password(pw: &str) -> anyhow::Result<String> {
    use argon2::password_hash::{PasswordHasher, SaltString};
    let salt = SaltString::generate(&mut rand::rngs::OsRng);
    Ok(argon2::Argon2::default()
        .hash_password(pw.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .to_string())
}

pub fn verify_password(pw: &str, hash: &str) -> bool {
    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    match PasswordHash::new(hash) {
        Ok(h) => argon2::Argon2::default().verify_password(pw.as_bytes(), &h).is_ok(),
        Err(_) => false,
    }
}

pub fn totp(secret: &[u8], account: &str) -> anyhow::Result<totp_rs::TOTP> {
    totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        30,
        secret.to_vec(),
        Some("LemonGalaxy".to_string()),
        account.to_string(),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

pub fn totp_ok(secret: &[u8], account: &str, code: &str) -> bool {
    let code: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    if code.len() != 6 {
        return false;
    }
    totp(secret, account).map(|t| t.check_current(&code).is_some()).unwrap_or(false)
}

fn set_cookie(token: &str, max_age: i64) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{ADMIN_COOKIE}={token}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age={max_age}"
    ))
    .unwrap()
}

#[derive(Deserialize, Default)]
pub struct PageQ {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub q: Option<String>,
}

impl PageQ {
    pub fn limits(&self) -> (i64, i64) {
        let size = self.page_size.unwrap_or(20).clamp(1, 200);
        let page = self.page.unwrap_or(1).max(1);
        (size, (page - 1) * size)
    }
    pub fn like(&self) -> String {
        format!("%{}%", self.q.clone().unwrap_or_default().replace('%', "").replace('_', "\\_"))
    }
}

fn ok() -> Json<Value> {
    Json(json!({ "ok": true }))
}

// ---------------- 初始化与登录 ----------------

async fn admin_count(st: &AppState) -> anyhow::Result<i64> {
    st.db.call(|c| c.query_row("SELECT COUNT(*) FROM admins", [], |r| r.get(0))).await
}

async fn setup_status(State(st): State<St>) -> AppResult<Json<Value>> {
    Ok(Json(json!({ "needs_setup": admin_count(&st).await? == 0 })))
}

#[derive(Deserialize)]
struct SetupReq {
    token: String,
    username: String,
    password: String,
}

async fn setup(State(st): State<St>, ctx: Ctx, Json(req): Json<SetupReq>) -> AppResult<Json<Value>> {
    if !st.rate.check(&format!("setup:{}", ctx.ip), 10, 60) {
        return Err(AppError::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited").lang(ctx.lang));
    }
    if admin_count(&st).await? > 0 {
        return Err(AppError::conflict("setup_done").lang(ctx.lang));
    }
    let expected = st.setup_token.lock().clone();
    match expected {
        Some(t) if galaxy_crypto::ct_eq(t.as_bytes(), req.token.trim().as_bytes()) => {}
        _ => return Err(AppError::new(StatusCode::FORBIDDEN, "invalid_token").lang(ctx.lang)),
    }
    if !valid_username(&req.username) {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "invalid_username").lang(ctx.lang));
    }
    if req.password.chars().count() < 8 {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "weak_password").lang(ctx.lang));
    }
    let hash = hash_password(&req.password)?;
    let now = galaxy_proto::now_ms();
    let u = req.username.clone();
    let id = st
        .db
        .call(move |c| {
            c.execute("INSERT INTO admins(username, password_hash, created_at) VALUES(?1, ?2, ?3)", params![u, hash, now])?;
            Ok(c.last_insert_rowid())
        })
        .await?;
    *st.setup_token.lock() = None;
    let actor = audit::Actor { kind: "admin", id: id.to_string(), name: req.username.clone(), ip: ctx.ip };
    audit::log(&st, &actor, "admin.create", Target::new("admin", id, &req.username), json!({ "setup": true })).await;
    tracing::info!("已创建第一个管理员 {}", req.username);
    Ok(ok())
}

#[derive(Deserialize)]
struct LoginReq {
    username: String,
    password: String,
    totp_code: Option<String>,
}

async fn login(State(st): State<St>, ctx: Ctx, Json(req): Json<LoginReq>) -> AppResult<Response> {
    if !st.rate.check(&format!("admin-login:{}", ctx.ip), 20, 300) {
        return Err(AppError::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited").lang(ctx.lang));
    }
    if admin_count(&st).await? == 0 {
        return Err(AppError::new(StatusCode::CONFLICT, "setup_required").lang(ctx.lang));
    }
    let u = req.username.clone();
    let row = st
        .db
        .call(move |c| {
            c.query_row(
                "SELECT id, username, password_hash, totp_secret, totp_enabled FROM admins WHERE username = ?1",
                params![u],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<Vec<u8>>>(3)?,
                        r.get::<_, i64>(4)? != 0,
                    ))
                },
            )
            .optional()
        })
        .await?;
    let fail = |st: St, ctx: Ctx, name: String| async move {
        let actor = audit::Actor { kind: "admin", id: String::new(), name, ip: ctx.ip.clone() };
        audit::log(&st, &actor, "admin.login_failed", Target::none(), json!({})).await;
        AppError::new(StatusCode::UNAUTHORIZED, "invalid_credentials").lang(ctx.lang)
    };
    let Some((id, username, hash, totp_secret, totp_enabled)) = row else {
        // 仍做一次哈希，避免通过耗时判断用户是否存在
        let _ = verify_password(&req.password, "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$4H4xoEfYhR3bKtAkNOq8uNbPv7Rf3K0vV0nB3W8Tq4E");
        return Err(fail(st.clone(), ctx, req.username.clone()).await);
    };
    if !verify_password(&req.password, &hash) {
        return Err(fail(st.clone(), ctx, req.username.clone()).await);
    }
    let stage = if totp_enabled {
        let secret = totp_secret.as_deref().and_then(|b| st.keys.open(b, "admin-totp")).unwrap_or_default();
        match req.totp_code.as_deref() {
            None | Some("") => return Err(AppError::new(StatusCode::UNAUTHORIZED, "need_totp").lang(ctx.lang)),
            Some(code) if totp_ok(&secret, &username, code) => "done",
            Some(_) => return Err(AppError::new(StatusCode::UNAUTHORIZED, "totp_invalid").lang(ctx.lang)),
        }
    } else {
        "totp_setup"
    };
    let token = new_token();
    let csrf = new_token();
    let now = galaxy_proto::now_ms();
    let max_age = 12 * 3600;
    let (th, c2, ip2, stage2) = (token_hash(&token), csrf.clone(), ctx.ip.clone(), stage.to_string());
    st.db
        .call(move |c| {
            c.execute("DELETE FROM admin_sessions WHERE expires_at < ?1", params![now])?;
            c.execute(
                "INSERT INTO admin_sessions(token_hash, admin_id, csrf, stage, ip, created_at, expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![th, id, c2, stage2, ip2, now, now + max_age * 1000],
            )?;
            c.execute("UPDATE admins SET last_login_at = ?1 WHERE id = ?2", params![now, id])
        })
        .await?;
    let actor = audit::Actor { kind: "admin", id: id.to_string(), name: username.clone(), ip: ctx.ip.clone() };
    audit::log(&st, &actor, "admin.login", Target::none(), json!({ "stage": stage })).await;
    let admin = admin_json(&st, id).await?;
    let mut resp = Json(json!({ "stage": stage, "csrf_token": csrf, "admin": admin })).into_response();
    resp.headers_mut().insert(header::SET_COOKIE, set_cookie(&token, max_age));
    Ok(resp)
}

async fn logout(State(st): State<St>, AdminAny(s): AdminAny) -> AppResult<Response> {
    let th = s.token_hash.clone();
    st.db.call(move |c| c.execute("DELETE FROM admin_sessions WHERE token_hash = ?1", params![th])).await?;
    audit::log(&st, &s.actor(), "admin.logout", Target::none(), json!({})).await;
    let mut resp = ok().into_response();
    resp.headers_mut().insert(header::SET_COOKIE, set_cookie("", 0));
    Ok(resp)
}

async fn admin_json(st: &AppState, id: i64) -> anyhow::Result<Value> {
    st.db
        .call(move |c| {
            c.query_row(
                "SELECT id, username, totp_enabled, created_at, last_login_at FROM admins WHERE id = ?1",
                params![id],
                |r| {
                    Ok(json!({
                        "id": r.get::<_, i64>(0)?,
                        "username": r.get::<_, String>(1)?,
                        "totp_enabled": r.get::<_, i64>(2)? != 0,
                        "created_at": r.get::<_, i64>(3)?,
                        "last_login_at": r.get::<_, i64>(4)?,
                    }))
                },
            )
        })
        .await
}

async fn me(State(st): State<St>, AdminAny(s): AdminAny) -> AppResult<Json<Value>> {
    let admin = admin_json(&st, s.admin_id).await?;
    Ok(Json(json!({ "admin": admin, "csrf_token": s.csrf, "stage": s.stage })))
}

async fn totp_setup(State(st): State<St>, AdminAny(s): AdminAny) -> AppResult<Json<Value>> {
    let secret = totp_rs::Secret::generate_secret().to_bytes().map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let t = totp(&secret, &s.username)?;
    let enc = st.keys.seal(&secret, "admin-totp-pending");
    let id = s.admin_id;
    // 未验证前存为待定：totp_enabled 保持 0
    st.db.call(move |c| c.execute("UPDATE admins SET totp_secret = ?1 WHERE id = ?2", params![enc, id])).await?;
    Ok(Json(json!({ "secret": t.get_secret_base32(), "otpauth_uri": t.get_url() })))
}

#[derive(Deserialize)]
struct CodeReq {
    code: String,
}

async fn totp_verify(State(st): State<St>, AdminAny(s): AdminAny, Json(req): Json<CodeReq>) -> AppResult<Json<Value>> {
    let id = s.admin_id;
    let enc: Option<Vec<u8>> = st
        .db
        .call(move |c| c.query_row("SELECT totp_secret FROM admins WHERE id = ?1", params![id], |r| r.get(0)))
        .await?;
    let secret = enc
        .as_deref()
        .and_then(|b| st.keys.open(b, "admin-totp-pending").or_else(|| st.keys.open(b, "admin-totp")))
        .ok_or_else(|| AppError::bad_request("").lang(s.lang))?;
    if !totp_ok(&secret, &s.username, &req.code) {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "totp_invalid").lang(s.lang));
    }
    let final_enc = st.keys.seal(&secret, "admin-totp");
    let th = s.token_hash.clone();
    st.db
        .call(move |c| {
            c.execute("UPDATE admins SET totp_secret = ?1, totp_enabled = 1 WHERE id = ?2", params![final_enc, id])?;
            c.execute("UPDATE admin_sessions SET stage = 'done' WHERE token_hash = ?1", params![th])
        })
        .await?;
    audit::log(&st, &s.actor(), "admin.totp_bind", Target::new("admin", id, &s.username), json!({})).await;
    Ok(ok())
}

#[derive(Deserialize)]
struct PasswordReq {
    old_password: String,
    new_password: String,
}

async fn change_password(State(st): State<St>, Admin(s): Admin, Json(req): Json<PasswordReq>) -> AppResult<Json<Value>> {
    if req.new_password.chars().count() < 8 {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "weak_password").lang(s.lang));
    }
    let id = s.admin_id;
    let hash: String = st.db.call(move |c| c.query_row("SELECT password_hash FROM admins WHERE id=?1", params![id], |r| r.get(0))).await?;
    if !verify_password(&req.old_password, &hash) {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "invalid_credentials").lang(s.lang));
    }
    let nh = hash_password(&req.new_password)?;
    let th = s.token_hash.clone();
    st.db
        .call(move |c| {
            c.execute("UPDATE admins SET password_hash = ?1 WHERE id = ?2", params![nh, id])?;
            c.execute("DELETE FROM admin_sessions WHERE admin_id = ?1 AND token_hash != ?2", params![id, th])
        })
        .await?;
    audit::log(&st, &s.actor(), "admin.password", Target::new("admin", id, &s.username), json!({})).await;
    Ok(ok())
}

// ---------------- 管理员 ----------------

async fn list_admins(State(st): State<St>, Admin(_s): Admin) -> AppResult<Json<Value>> {
    let ids: Vec<i64> = st
        .db
        .call(|c| {
            let mut stmt = c.prepare("SELECT id FROM admins ORDER BY id")?;
            let v = stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<i64>, _>>()?;
            Ok(v)
        })
        .await?;
    let mut items = vec![];
    for id in ids {
        items.push(admin_json(&st, id).await?);
    }
    Ok(Json(json!({ "items": items })))
}

#[derive(Deserialize)]
struct CreateAdminReq {
    username: String,
    password: String,
}

async fn create_admin(State(st): State<St>, Admin(s): Admin, Json(req): Json<CreateAdminReq>) -> AppResult<Json<Value>> {
    if !valid_username(&req.username) {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "invalid_username").lang(s.lang));
    }
    if req.password.chars().count() < 8 {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "weak_password").lang(s.lang));
    }
    let hash = hash_password(&req.password)?;
    let now = galaxy_proto::now_ms();
    let u = req.username.clone();
    let id = st
        .db
        .call(move |c| {
            let exists: Option<i64> = c.query_row("SELECT id FROM admins WHERE username = ?1", params![u], |r| r.get(0)).optional()?;
            if exists.is_some() {
                return Ok(None);
            }
            c.execute("INSERT INTO admins(username, password_hash, created_at) VALUES(?1,?2,?3)", params![u, hash, now])?;
            Ok(Some(c.last_insert_rowid()))
        })
        .await?
        .ok_or_else(|| AppError::conflict("username_taken").lang(s.lang))?;
    audit::log(&st, &s.actor(), "admin.create", Target::new("admin", id, &req.username), json!({})).await;
    Ok(Json(admin_json(&st, id).await?))
}

async fn delete_admin(State(st): State<St>, Admin(s): Admin, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    if id == s.admin_id {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "self_delete").lang(s.lang));
    }
    if admin_count(&st).await? <= 1 {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "last_admin").lang(s.lang));
    }
    let name: Option<String> = st
        .db
        .call(move |c| {
            let n: Option<String> = c.query_row("SELECT username FROM admins WHERE id=?1", params![id], |r| r.get(0)).optional()?;
            c.execute("DELETE FROM admins WHERE id = ?1", params![id])?;
            c.execute("DELETE FROM admin_sessions WHERE admin_id = ?1", params![id])?;
            Ok(n)
        })
        .await?;
    let name = name.ok_or_else(|| AppError::not_found().lang(s.lang))?;
    audit::log(&st, &s.actor(), "admin.delete", Target::new("admin", id, name), json!({})).await;
    Ok(ok())
}

async fn reset_admin_totp(State(st): State<St>, Admin(s): Admin, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    st.db
        .call(move |c| {
            c.execute("UPDATE admins SET totp_secret = NULL, totp_enabled = 0 WHERE id = ?1", params![id])?;
            c.execute("DELETE FROM admin_sessions WHERE admin_id = ?1", params![id])
        })
        .await?;
    audit::log(&st, &s.actor(), "admin.totp_bind", Target::new("admin", id, ""), json!({ "reset": true })).await;
    Ok(ok())
}

// ---------------- 概览 ----------------

async fn server_info(State(st): State<St>, Admin(_s): Admin) -> AppResult<Json<Value>> {
    let (users, devices): (i64, i64) = st
        .db
        .call(|c| {
            Ok((
                c.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?,
                c.query_row("SELECT COUNT(*) FROM devices", [], |r| r.get(0))?,
            ))
        })
        .await?;
    Ok(Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "started_at": st.started_at,
        "address": st.client_address(),
        "fingerprint": st.keys.fingerprint,
        "public_host": st.public_host(),
        "port": st.port,
        "users": users,
        "devices": devices,
        "online_devices": st.hub.devices.read().len(),
        "active_connections": st.conns.read().len(),
        "relay_bytes_total": st.relay.total_bytes.load(std::sync::atomic::Ordering::Relaxed),
        "db_size_bytes": crate::db::Db::file_size(&st.db_path),
    })))
}

#[derive(Deserialize)]
struct DaysQ {
    days: Option<i64>,
}

async fn stats_timeline(State(st): State<St>, Admin(_s): Admin, Query(q): Query<DaysQ>) -> AppResult<Json<Value>> {
    let days = q.days.unwrap_or(7).clamp(1, 90);
    let rows: HashMap<String, (i64, i64, i64, i64)> = st
        .db
        .call(move |c| {
            let mut stmt = c.prepare("SELECT day, connections, relay_bytes, ai_tokens, logins FROM daily_stats ORDER BY day DESC LIMIT ?1")?;
            let v = stmt
                .query_map(params![days + 2], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))))?
                .collect::<Result<HashMap<_, _>, _>>()?;
            Ok(v)
        })
        .await?;
    let mut items = vec![];
    let today = time::OffsetDateTime::now_utc().date();
    for i in (0..days).rev() {
        let d = today - time::Duration::days(i);
        let key = format!("{:04}-{:02}-{:02}", d.year(), d.month() as u8, d.day());
        let (c, rb, ai, lg) = rows.get(&key).copied().unwrap_or((0, 0, 0, 0));
        items.push(json!({ "day": key, "connections": c, "relay_bytes": rb, "ai_tokens": ai, "logins": lg }));
    }
    Ok(Json(json!({ "items": items })))
}

// ---------------- 用户 ----------------

const USER_COLS: &str = "id, username, display_name, note, disabled, must_change_password, totp_enabled, require_totp,
    account_pubkey IS NOT NULL, account_epoch, ai_enabled, ai_max_mode, ai_daily_token_limit, ai_monthly_token_limit,
    created_at, last_login_at";

fn user_from_row(r: &Row) -> r2d2_sqlite::rusqlite::Result<Value> {
    Ok(json!({
        "id": r.get::<_, i64>(0)?,
        "username": r.get::<_, String>(1)?,
        "display_name": r.get::<_, String>(2)?,
        "note": r.get::<_, String>(3)?,
        "disabled": r.get::<_, i64>(4)? != 0,
        "must_change_password": r.get::<_, i64>(5)? != 0,
        "totp_enabled": r.get::<_, i64>(6)? != 0,
        "require_totp": r.get::<_, i64>(7)? != 0,
        "account_initialized": r.get::<_, bool>(8)?,
        "account_epoch": r.get::<_, i64>(9)?,
        "ai_enabled": r.get::<_, i64>(10)? != 0,
        "ai_max_mode": r.get::<_, String>(11)?,
        "ai_daily_token_limit": r.get::<_, i64>(12)?,
        "ai_monthly_token_limit": r.get::<_, i64>(13)?,
        "created_at": r.get::<_, i64>(14)?,
        "last_login_at": r.get::<_, i64>(15)?,
    }))
}

fn decorate_user(st: &AppState, mut u: Value, counts: &HashMap<i64, i64>) -> Value {
    let id = u["id"].as_i64().unwrap_or(0);
    let online = st.hub.devices.read().values().filter(|d| d.accounts.contains(&(id as u64))).count();
    u["device_count"] = json!(counts.get(&id).copied().unwrap_or(0));
    u["online_device_count"] = json!(online);
    u
}

async fn device_counts(st: &AppState) -> anyhow::Result<HashMap<i64, i64>> {
    st.db
        .call(|c| {
            let mut stmt = c.prepare("SELECT user_id, COUNT(*) FROM device_accounts WHERE status = 'active' GROUP BY user_id")?;
            let v = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<HashMap<_, _>, _>>()?;
            Ok(v)
        })
        .await
}

pub async fn user_value(st: &AppState, id: i64) -> AppResult<Value> {
    let sql = format!("SELECT {USER_COLS} FROM users WHERE id = ?1");
    let u = st
        .db
        .call(move |c| c.query_row(&sql, params![id], user_from_row).optional())
        .await?
        .ok_or_else(AppError::not_found)?;
    let counts = device_counts(st).await?;
    Ok(decorate_user(st, u, &counts))
}

async fn list_users(State(st): State<St>, Admin(_s): Admin, Query(q): Query<PageQ>) -> AppResult<Json<Value>> {
    let (limit, offset) = q.limits();
    let like = q.like();
    let sql = format!(
        "SELECT {USER_COLS} FROM users WHERE username LIKE ?1 ESCAPE '\\' OR display_name LIKE ?1 ESCAPE '\\' ORDER BY id DESC LIMIT ?2 OFFSET ?3"
    );
    let (items, total) = st
        .db
        .call(move |c| {
            let mut stmt = c.prepare(&sql)?;
            let items = stmt.query_map(params![like, limit, offset], user_from_row)?.collect::<Result<Vec<_>, _>>()?;
            let total: i64 = c.query_row(
                "SELECT COUNT(*) FROM users WHERE username LIKE ?1 ESCAPE '\\' OR display_name LIKE ?1 ESCAPE '\\'",
                params![like],
                |r| r.get(0),
            )?;
            Ok((items, total))
        })
        .await?;
    let counts = device_counts(&st).await?;
    let items: Vec<Value> = items.into_iter().map(|u| decorate_user(&st, u, &counts)).collect();
    Ok(Json(json!({ "items": items, "total": total })))
}

#[derive(Deserialize)]
struct CreateUserReq {
    username: String,
    display_name: Option<String>,
    initial_password: String,
    note: Option<String>,
    require_totp: Option<bool>,
    ai_enabled: Option<bool>,
    ai_max_mode: Option<String>,
}

async fn create_user(State(st): State<St>, Admin(s): Admin, Json(req): Json<CreateUserReq>) -> AppResult<Json<Value>> {
    let username = req.username.trim().to_string();
    if !valid_username(&username) {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "invalid_username").lang(s.lang));
    }
    if req.initial_password.chars().count() < 8 {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "weak_password").lang(s.lang));
    }
    let mode = req.ai_max_mode.clone().unwrap_or_else(|| "confirm_dangerous".into());
    if AiMode::parse(&mode).is_none() {
        return Err(AppError::bad_request("ai_max_mode").lang(s.lang));
    }
    let record = st.keys.opaque.register_with_password(&username, &req.initial_password)?;
    let now = galaxy_proto::now_ms();
    let (u2, dn, note) = (username.clone(), req.display_name.clone().unwrap_or_default(), req.note.clone().unwrap_or_default());
    let (rt, ai) = (req.require_totp.unwrap_or(false) as i64, req.ai_enabled.unwrap_or(true) as i64);
    let id = st
        .db
        .call(move |c| {
            let exists: Option<i64> = c.query_row("SELECT id FROM users WHERE username = ?1", params![u2], |r| r.get(0)).optional()?;
            if exists.is_some() {
                return Ok(None);
            }
            c.execute(
                "INSERT INTO users(username, display_name, note, opaque_record, require_totp, ai_enabled, ai_max_mode, created_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![u2, dn, note, record, rt, ai, mode, now],
            )?;
            Ok(Some(c.last_insert_rowid()))
        })
        .await?
        .ok_or_else(|| AppError::conflict("username_taken").lang(s.lang))?;
    audit::log(&st, &s.actor(), "user.create", Target::new("user", id, &username), json!({})).await;
    Ok(Json(user_value(&st, id).await?))
}

async fn get_user(State(st): State<St>, Admin(s): Admin, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    Ok(Json(user_value(&st, id).await.map_err(|e| e.lang(s.lang))?))
}

async fn username_of(st: &AppState, id: i64) -> AppResult<String> {
    st.db
        .call(move |c| c.query_row("SELECT username FROM users WHERE id = ?1", params![id], |r| r.get(0)).optional())
        .await?
        .ok_or_else(AppError::not_found)
}

async fn update_user(State(st): State<St>, Admin(s): Admin, Path(id): Path<i64>, Json(req): Json<Value>) -> AppResult<Json<Value>> {
    let name = username_of(&st, id).await.map_err(|e| e.lang(s.lang))?;
    let mut sets: Vec<(&'static str, Value)> = vec![];
    for (k, col) in [("display_name", "display_name"), ("note", "note")] {
        if let Some(v) = req.get(k).and_then(|v| v.as_str()) {
            sets.push((col, json!(v)));
        }
    }
    for k in ["disabled", "require_totp", "ai_enabled"] {
        if let Some(v) = req.get(k).and_then(|v| v.as_bool()) {
            sets.push((k, json!(v as i64)));
        }
    }
    if let Some(v) = req.get("ai_max_mode").and_then(|v| v.as_str()) {
        if AiMode::parse(v).is_none() {
            return Err(AppError::bad_request("ai_max_mode").lang(s.lang));
        }
        sets.push(("ai_max_mode", json!(v)));
    }
    for k in ["ai_daily_token_limit", "ai_monthly_token_limit"] {
        if let Some(v) = req.get(k).and_then(|v| v.as_i64()) {
            sets.push((k, json!(v.max(0))));
        }
    }
    let disabling = req.get("disabled").and_then(|v| v.as_bool()) == Some(true);
    st.db
        .call(move |c| {
            for (col, v) in &sets {
                let sql = format!("UPDATE users SET {col} = ?1 WHERE id = ?2");
                match v {
                    Value::String(s) => c.execute(&sql, params![s, id])?,
                    Value::Number(n) => c.execute(&sql, params![n.as_i64().unwrap_or(0), id])?,
                    _ => 0,
                };
            }
            if disabling {
                c.execute("DELETE FROM sessions WHERE user_id = ?1", params![id])?;
            }
            Ok(())
        })
        .await?;
    if disabling {
        st.hub.notify_user(id as u64, WsMsg::SessionRevoked);
    }
    audit::log(&st, &s.actor(), "user.update", Target::new("user", id, &name), req).await;
    Ok(Json(user_value(&st, id).await?))
}

#[derive(Deserialize)]
struct ResetPwReq {
    new_password: String,
}

async fn reset_user_password(State(st): State<St>, Admin(s): Admin, Path(id): Path<i64>, Json(req): Json<ResetPwReq>) -> AppResult<Json<Value>> {
    if req.new_password.chars().count() < 8 {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "weak_password").lang(s.lang));
    }
    let name = username_of(&st, id).await.map_err(|e| e.lang(s.lang))?;
    let record = st.keys.opaque.register_with_password(&name, &req.new_password)?;
    let devices: Vec<Vec<u8>> = st
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            tx.execute(
                "UPDATE users SET opaque_record = ?1, must_change_password = 1, account_pubkey = NULL, wrapped_secrets = NULL,
                 wrapped_nonce = NULL, revocations = NULL, revocations_sig = NULL, account_epoch = account_epoch + 1,
                 failed_logins = 0, locked_until = 0 WHERE id = ?2",
                params![record, id],
            )?;
            let devs = {
                let mut stmt = tx.prepare("SELECT device_id FROM device_accounts WHERE user_id = ?1")?;
                let v = stmt.query_map(params![id], |r| r.get::<_, Vec<u8>>(0))?.collect::<Result<Vec<_>, _>>()?;
                v
            };
            tx.execute("UPDATE device_accounts SET status = 'needs_relogin' WHERE user_id = ?1", params![id])?;
            tx.execute("DELETE FROM sessions WHERE user_id = ?1", params![id])?;
            tx.execute("DELETE FROM vault_items WHERE user_id = ?1", params![id])?;
            tx.commit()?;
            Ok(devs)
        })
        .await?;
    st.hub.notify_user(id as u64, WsMsg::AccountReset { user_id: id as u64 });
    for d in devices {
        if let Some(did) = DeviceId::from_slice(&d) {
            crate::ws::refresh_device_accounts(&st, did).await;
        }
    }
    audit::log(&st, &s.actor(), "user.reset_password", Target::new("user", id, &name), json!({})).await;
    Ok(Json(user_value(&st, id).await?))
}

async fn reset_user_totp(State(st): State<St>, Admin(s): Admin, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let name = username_of(&st, id).await.map_err(|e| e.lang(s.lang))?;
    st.db.call(move |c| c.execute("UPDATE users SET totp_secret = NULL, totp_enabled = 0 WHERE id = ?1", params![id])).await?;
    audit::log(&st, &s.actor(), "user.reset_totp", Target::new("user", id, &name), json!({})).await;
    Ok(ok())
}

async fn revoke_user_sessions(State(st): State<St>, Admin(s): Admin, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let name = username_of(&st, id).await.map_err(|e| e.lang(s.lang))?;
    let devices: Vec<Vec<u8>> = st
        .db
        .call(move |c| {
            let mut stmt = c.prepare("SELECT device_id FROM device_accounts WHERE user_id = ?1 AND status = 'active'")?;
            let v = stmt.query_map(params![id], |r| r.get::<_, Vec<u8>>(0))?.collect::<Result<Vec<_>, _>>()?;
            c.execute("DELETE FROM sessions WHERE user_id = ?1", params![id])?;
            c.execute("UPDATE device_accounts SET status = 'needs_relogin' WHERE user_id = ?1 AND status = 'active'", params![id])?;
            Ok(v)
        })
        .await?;
    st.hub.notify_user(id as u64, WsMsg::SessionRevoked);
    for d in devices {
        if let Some(did) = DeviceId::from_slice(&d) {
            crate::ws::refresh_device_accounts(&st, did).await;
        }
    }
    audit::log(&st, &s.actor(), "user.revoke_sessions", Target::new("user", id, &name), json!({})).await;
    Ok(ok())
}

async fn delete_user(State(st): State<St>, Admin(s): Admin, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let name = username_of(&st, id).await.map_err(|e| e.lang(s.lang))?;
    st.hub.notify_user(id as u64, WsMsg::SessionRevoked);
    let devices: Vec<Vec<u8>> = st
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            let devs = {
                let mut stmt = tx.prepare("SELECT device_id FROM device_accounts WHERE user_id = ?1")?;
                let v = stmt.query_map(params![id], |r| r.get::<_, Vec<u8>>(0))?.collect::<Result<Vec<_>, _>>()?;
                v
            };
            for t in ["sessions", "device_accounts", "vault_items", "ai_grants"] {
                tx.execute(&format!("DELETE FROM {t} WHERE user_id = ?1"), params![id])?;
            }
            tx.execute("DELETE FROM users WHERE id = ?1", params![id])?;
            tx.commit()?;
            Ok(devs)
        })
        .await?;
    for d in devices {
        if let Some(did) = DeviceId::from_slice(&d) {
            crate::ws::refresh_device_accounts(&st, did).await;
        }
    }
    audit::log(&st, &s.actor(), "user.delete", Target::new("user", id, &name), json!({})).await;
    Ok(ok())
}

async fn get_grants(State(st): State<St>, Admin(_s): Admin, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let items: Vec<Value> = st
        .db
        .call(move |c| {
            let mut stmt = c.prepare("SELECT provider_id, models_json FROM ai_grants WHERE user_id = ?1 ORDER BY provider_id")?;
            let v = stmt
                .query_map(params![id], |r| {
                    let models: Value = serde_json::from_str(&r.get::<_, String>(1)?).unwrap_or(json!([]));
                    Ok(json!({ "provider_id": r.get::<_, i64>(0)?, "models": models }))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(v)
        })
        .await?;
    Ok(Json(json!({ "items": items })))
}

#[derive(Deserialize)]
struct GrantItem {
    provider_id: i64,
    models: Vec<String>,
}

#[derive(Deserialize)]
struct GrantsReq {
    items: Vec<GrantItem>,
}

async fn put_grants(State(st): State<St>, Admin(s): Admin, Path(id): Path<i64>, Json(req): Json<GrantsReq>) -> AppResult<Json<Value>> {
    let name = username_of(&st, id).await.map_err(|e| e.lang(s.lang))?;
    let items: Vec<(i64, String)> = req
        .items
        .iter()
        .filter(|g| !g.models.is_empty())
        .map(|g| (g.provider_id, serde_json::to_string(&g.models).unwrap()))
        .collect();
    let detail = json!(req.items.iter().map(|g| json!({"provider_id": g.provider_id, "models": g.models})).collect::<Vec<_>>());
    st.db
        .call(move |c| {
            let tx = c.transaction()?;
            tx.execute("DELETE FROM ai_grants WHERE user_id = ?1", params![id])?;
            for (p, m) in &items {
                tx.execute("INSERT INTO ai_grants(user_id, provider_id, models_json) VALUES(?1,?2,?3)", params![id, p, m])?;
            }
            tx.commit()
        })
        .await?;
    audit::log(&st, &s.actor(), "ai.grants_update", Target::new("user", id, &name), detail).await;
    get_grants(State(st), Admin(s), Path(id)).await
}

// ---------------- 设备 ----------------

const DEVICE_COLS: &str = "id, kind, platform, os_version, arch, hostname, app_version, last_seen_at, last_ip, created_at";

fn device_from_row(r: &Row) -> r2d2_sqlite::rusqlite::Result<(Vec<u8>, Value)> {
    let id: Vec<u8> = r.get(0)?;
    Ok((
        id.clone(),
        json!({
            "id": hex::encode(&id),
            "kind": r.get::<_, String>(1)?,
            "platform": r.get::<_, String>(2)?,
            "os_version": r.get::<_, String>(3)?,
            "arch": r.get::<_, String>(4)?,
            "hostname": r.get::<_, String>(5)?,
            "app_version": r.get::<_, String>(6)?,
            "last_seen_at": r.get::<_, i64>(7)?,
            "last_ip": r.get::<_, String>(8)?,
            "created_at": r.get::<_, i64>(9)?,
        }),
    ))
}

async fn decorate_devices(st: &AppState, rows: Vec<(Vec<u8>, Value)>) -> anyhow::Result<Vec<Value>> {
    let ids: Vec<Vec<u8>> = rows.iter().map(|r| r.0.clone()).collect();
    let accts: HashMap<Vec<u8>, Vec<Value>> = st
        .db
        .call(move |c| {
            let mut m: HashMap<Vec<u8>, Vec<Value>> = HashMap::new();
            let mut stmt = c.prepare(
                "SELECT da.device_id, da.user_id, u.username, da.status, da.added_at FROM device_accounts da
                 JOIN users u ON u.id = da.user_id WHERE da.device_id = ?1 ORDER BY da.added_at",
            )?;
            for id in &ids {
                let v = stmt
                    .query_map(params![id], |r| {
                        Ok(json!({
                            "user_id": r.get::<_, i64>(1)?,
                            "username": r.get::<_, String>(2)?,
                            "status": r.get::<_, String>(3)?,
                            "added_at": r.get::<_, i64>(4)?,
                        }))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                m.insert(id.clone(), v);
            }
            Ok(m)
        })
        .await?;
    let hub = st.hub.devices.read();
    Ok(rows
        .into_iter()
        .map(|(id, mut v)| {
            let did = DeviceId::from_slice(&id).unwrap_or_default();
            let online = hub.get(&did);
            v["online"] = json!(online.is_some());
            v["assist_mode"] = json!(online.map(|o| o.assist_code.is_some()).unwrap_or(false));
            v["accounts"] = json!(accts.get(&id).cloned().unwrap_or_default());
            v
        })
        .collect())
}

#[derive(Deserialize)]
struct DeviceQ {
    page: Option<i64>,
    page_size: Option<i64>,
    q: Option<String>,
    user_id: Option<i64>,
    online: Option<bool>,
    kind: Option<String>,
}

impl DeviceQ {
    fn pq(&self) -> PageQ {
        PageQ { page: self.page, page_size: self.page_size, q: self.q.clone() }
    }
}

async fn list_devices(State(st): State<St>, Admin(_s): Admin, Query(q): Query<DeviceQ>) -> AppResult<Json<Value>> {
    let like = q.pq().like();
    let uid = q.user_id;
    let kind = q.kind.clone().filter(|k| !k.is_empty());
    let rows = st
        .db
        .call(move |c| {
            let mut sql = format!("SELECT {DEVICE_COLS} FROM devices d WHERE (hostname LIKE ?1 ESCAPE '\\' OR hex(id) LIKE upper(?1))");
            if uid.is_some() {
                sql.push_str(" AND EXISTS (SELECT 1 FROM device_accounts da WHERE da.device_id = d.id AND da.user_id = ?2)");
            } else {
                sql.push_str(" AND ?2 IS NULL");
            }
            if kind.is_some() {
                sql.push_str(" AND kind = ?3");
            } else {
                sql.push_str(" AND ?3 IS NULL");
            }
            sql.push_str(" ORDER BY last_seen_at DESC");
            let mut stmt = c.prepare(&sql)?;
            let v = stmt.query_map(params![like, uid, kind], device_from_row)?.collect::<Result<Vec<_>, _>>()?;
            Ok(v)
        })
        .await?;
    let mut items = decorate_devices(&st, rows).await?;
    if let Some(on) = q.online {
        items.retain(|d| d["online"].as_bool() == Some(on));
    }
    let total = items.len();
    let (limit, offset) = q.pq().limits();
    let items: Vec<Value> = items.into_iter().skip(offset as usize).take(limit as usize).collect();
    Ok(Json(json!({ "items": items, "total": total })))
}

async fn device_value(st: &AppState, id: DeviceId) -> AppResult<Value> {
    let raw = id.0.to_vec();
    let sql = format!("SELECT {DEVICE_COLS} FROM devices WHERE id = ?1");
    let row = st.db.call(move |c| c.query_row(&sql, params![raw], device_from_row).optional()).await?.ok_or_else(AppError::not_found)?;
    Ok(decorate_devices(st, vec![row]).await?.remove(0))
}

fn parse_dev(id: &str) -> AppResult<DeviceId> {
    DeviceId::from_hex(id).ok_or_else(|| AppError::bad_request("id"))
}

async fn get_device(State(st): State<St>, Admin(s): Admin, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let did = parse_dev(&id)?;
    Ok(Json(device_value(&st, did).await.map_err(|e| e.lang(s.lang))?))
}

#[derive(Deserialize, Default)]
struct RevokeReq {
    user_id: Option<i64>,
}

async fn revoke_device(State(st): State<St>, Admin(s): Admin, Path(id): Path<String>, body: Option<Json<RevokeReq>>) -> AppResult<Json<Value>> {
    let did = parse_dev(&id)?;
    let uid = body.and_then(|b| b.0.user_id);
    let raw = did.0.to_vec();
    st.db
        .call(move |c| {
            match uid {
                Some(u) => {
                    c.execute("UPDATE device_accounts SET status = 'revoked' WHERE device_id = ?1 AND user_id = ?2", params![raw, u])?;
                    c.execute("DELETE FROM sessions WHERE device_id = ?1 AND user_id = ?2", params![raw, u])?;
                }
                None => {
                    c.execute("UPDATE device_accounts SET status = 'revoked' WHERE device_id = ?1", params![raw])?;
                    c.execute("DELETE FROM sessions WHERE device_id = ?1", params![raw])?;
                }
            }
            Ok(())
        })
        .await?;
    crate::ws::refresh_device_accounts(&st, did).await;
    st.hub.send(&did, WsMsg::SessionRevoked);
    audit::log(&st, &s.actor(), "device.revoke", Target::new("device", did.to_hex(), ""), json!({ "user_id": uid })).await;
    Ok(Json(device_value(&st, did).await?))
}

async fn delete_device(State(st): State<St>, Admin(s): Admin, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let did = parse_dev(&id)?;
    let raw = did.0.to_vec();
    st.hub.send(&did, WsMsg::SessionRevoked);
    let users: Vec<i64> = st
        .db
        .call(move |c| {
            let mut stmt = c.prepare("SELECT user_id FROM device_accounts WHERE device_id = ?1")?;
            let v = stmt.query_map(params![raw], |r| r.get(0))?.collect::<Result<Vec<i64>, _>>()?;
            c.execute("DELETE FROM device_accounts WHERE device_id = ?1", params![raw])?;
            c.execute("DELETE FROM sessions WHERE device_id = ?1", params![raw])?;
            c.execute("DELETE FROM devices WHERE id = ?1", params![raw])?;
            Ok(v)
        })
        .await?;
    crate::ws::refresh_device_accounts(&st, did).await;
    for u in users {
        st.hub.notify_user(u as u64, WsMsg::DevicesChanged);
    }
    audit::log(&st, &s.actor(), "device.delete", Target::new("device", did.to_hex(), ""), json!({})).await;
    Ok(ok())
}

// ---------------- 连接 ----------------

async fn list_connections(State(st): State<St>, Admin(_s): Admin) -> AppResult<Json<Value>> {
    let conns = st.conns.read();
    let relay = st.relay.pairs.read();
    let items: Vec<Value> = conns
        .values()
        .map(|c| {
            let mut v = serde_json::to_value(c).unwrap_or(json!({}));
            v["relay_bytes"] = json!(relay.get(&c.id).map(|p| p.bytes.load(std::sync::atomic::Ordering::Relaxed)).unwrap_or(0));
            v
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

async fn terminate_connection(State(st): State<St>, Admin(s): Admin, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let cid = galaxy_proto::ConnId::from_hex(&id).ok_or_else(|| AppError::bad_request("id"))?;
    let info = st.conns.write().remove(&cid);
    st.relay.remove(&cid);
    if let Some(c) = info {
        st.hub.send(&c.from_device_id, WsMsg::ConnClose { conn_id: cid });
        st.hub.send(&c.to_device_id, WsMsg::ConnClose { conn_id: cid });
        audit::log(&st, &s.actor(), "connection.end", Target::new("connection", cid.to_hex(), ""), json!({ "terminated": true })).await;
    }
    Ok(ok())
}

// ---------------- 审计 ----------------

#[derive(Deserialize)]
struct AuditQ {
    page: Option<i64>,
    page_size: Option<i64>,
    q: Option<String>,
    from: Option<i64>,
    to: Option<i64>,
    actor_type: Option<String>,
    action: Option<String>,
}

async fn list_audit(State(st): State<St>, Admin(_s): Admin, Query(q): Query<AuditQ>) -> AppResult<Json<Value>> {
    let pq = PageQ { page: q.page, page_size: q.page_size, q: q.q.clone() };
    let (limit, offset) = pq.limits();
    let like = pq.like();
    let from = q.from.unwrap_or(0);
    let to = q.to.unwrap_or(i64::MAX);
    let at = q.actor_type.clone().filter(|s| !s.is_empty());
    let ac = q.action.clone().filter(|s| !s.is_empty());
    let (items, total) = st
        .db
        .call(move |c| {
            let wh = "at >= ?1 AND at <= ?2 AND (?3 IS NULL OR actor_type = ?3) AND (?4 IS NULL OR action = ?4)
                      AND (actor_name LIKE ?5 ESCAPE '\\' OR target_name LIKE ?5 ESCAPE '\\' OR detail LIKE ?5 ESCAPE '\\' OR ip LIKE ?5 ESCAPE '\\')";
            let mut stmt = c.prepare(&format!(
                "SELECT id, at, actor_type, actor_id, actor_name, action, target_type, target_id, target_name, ip, detail
                 FROM audit WHERE {wh} ORDER BY id DESC LIMIT ?6 OFFSET ?7"
            ))?;
            let items = stmt
                .query_map(params![from, to, at, ac, like, limit, offset], |r| {
                    let detail: Value = serde_json::from_str(&r.get::<_, String>(10)?).unwrap_or(json!({}));
                    Ok(json!({
                        "id": r.get::<_, i64>(0)?, "at": r.get::<_, i64>(1)?,
                        "actor_type": r.get::<_, String>(2)?, "actor_id": r.get::<_, String>(3)?, "actor_name": r.get::<_, String>(4)?,
                        "action": r.get::<_, String>(5)?,
                        "target_type": r.get::<_, String>(6)?, "target_id": r.get::<_, String>(7)?, "target_name": r.get::<_, String>(8)?,
                        "ip": r.get::<_, String>(9)?, "detail": detail,
                    }))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let total: i64 = c.query_row(&format!("SELECT COUNT(*) FROM audit WHERE {wh}"), params![from, to, at, ac, like], |r| r.get(0))?;
            Ok((items, total))
        })
        .await?;
    Ok(Json(json!({ "items": items, "total": total })))
}

// ---------------- 设置 ----------------

async fn get_settings(State(st): State<St>, Admin(_s): Admin) -> AppResult<Json<Value>> {
    Ok(Json(serde_json::to_value(&*st.settings.read())?))
}

async fn patch_settings(State(st): State<St>, Admin(s): Admin, Json(req): Json<Value>) -> AppResult<Json<Value>> {
    let mut cur = serde_json::to_value(&*st.settings.read())?;
    if let (Some(obj), Some(patch)) = (cur.as_object_mut(), req.as_object()) {
        for (k, v) in patch {
            if obj.contains_key(k) {
                obj.insert(k.clone(), v.clone());
            }
        }
    }
    let new: crate::state::Settings = serde_json::from_value(cur).map_err(|e| AppError::bad_request(e.to_string()).lang(s.lang))?;
    for n in &new.admin_ip_allowlist {
        if n.parse::<ipnet::IpNet>().is_err() && n.parse::<std::net::IpAddr>().is_err() {
            return Err(AppError::bad_request(format!("admin_ip_allowlist: {n}")).lang(s.lang));
        }
    }
    // 防止把自己锁在外面
    if !new.admin_ip_allowlist.is_empty() {
        let mine: std::net::IpAddr = s.ip.parse().unwrap_or(std::net::IpAddr::from([127, 0, 0, 1]));
        let ok = new.admin_ip_allowlist.iter().any(|n| {
            n.parse::<ipnet::IpNet>().map(|x| x.contains(&mine)).unwrap_or(false) || n.parse::<std::net::IpAddr>().map(|a| a == mine).unwrap_or(false)
        });
        if !ok {
            return Err(AppError::bad_request(format!("admin_ip_allowlist 不包含当前 IP {}", s.ip)).lang(s.lang));
        }
    }
    *st.settings.write() = new;
    st.save_settings().await?;
    audit::log(&st, &s.actor(), "settings.update", Target::none(), req).await;
    get_settings(State(st), Admin(s)).await
}

// ---------------- 更新包（第三期完善） ----------------

async fn list_updates(State(st): State<St>, Admin(_s): Admin) -> AppResult<Json<Value>> {
    let items: Vec<Value> = st
        .db
        .call(|c| {
            let mut stmt = c.prepare(
                "SELECT id, component, platform, arch, version, file_name, size, sha256, signature_ok, source, created_at FROM update_packages ORDER BY id DESC",
            )?;
            let v = stmt
                .query_map([], |r| {
                    Ok(json!({
                        "id": r.get::<_, i64>(0)?, "component": r.get::<_, String>(1)?, "platform": r.get::<_, String>(2)?,
                        "arch": r.get::<_, String>(3)?, "version": r.get::<_, String>(4)?, "file_name": r.get::<_, String>(5)?,
                        "size": r.get::<_, i64>(6)?, "sha256": r.get::<_, String>(7)?, "signature_ok": r.get::<_, i64>(8)? != 0,
                        "source": r.get::<_, String>(9)?, "created_at": r.get::<_, i64>(10)?,
                    }))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(v)
        })
        .await?;
    Ok(Json(json!({ "items": items, "last_check_at": 0, "last_error": "" })))
}

async fn check_updates(State(st): State<St>, Admin(s): Admin) -> AppResult<Json<Value>> {
    let _ = &st;
    Err(AppError::new(StatusCode::NOT_IMPLEMENTED, "not_implemented").lang(s.lang))
}

async fn upload_update(Admin(s): Admin, _h: HeaderMap) -> AppResult<Json<Value>> {
    Err(AppError::new(StatusCode::NOT_IMPLEMENTED, "not_implemented").lang(s.lang))
}

async fn delete_update(State(st): State<St>, Admin(_s): Admin, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    st.db.call(move |c| c.execute("DELETE FROM update_packages WHERE id = ?1", params![id])).await?;
    Ok(ok())
}
