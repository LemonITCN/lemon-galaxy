//! 请求上下文提取：客户端 IP、语言、管理员会话（Cookie + CSRF）、用户会话（Bearer）。

use crate::error::{AppError, Lang};
use crate::state::St;
use axum::extract::{ConnectInfo, FromRequestParts};
use crate::tls::PeerAddr;
use axum::http::request::Parts;
use axum::http::{header, Method, StatusCode};
use r2d2_sqlite::rusqlite::{params, OptionalExtension};

pub const ADMIN_COOKIE: &str = "lg_admin";

pub fn token_hash(token: &str) -> Vec<u8> {
    galaxy_crypto::sha256(token.as_bytes()).to_vec()
}

pub fn new_token() -> String {
    galaxy_proto::b64::encode(&galaxy_crypto::random_bytes::<32>())
}

pub struct Ctx {
    pub ip: String,
    pub lang: Lang,
}

impl<S: Send + Sync> FromRequestParts<S> for Ctx {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, _s: &S) -> Result<Self, Self::Rejection> {
        let ip = parts
            .extensions
            .get::<ConnectInfo<PeerAddr>>()
            .map(|c| canonical_ip(c.0 .0.ip()).to_string())
            .unwrap_or_default();
        let lang = Lang::from_header(parts.headers.get(header::ACCEPT_LANGUAGE).and_then(|v| v.to_str().ok()));
        Ok(Ctx { ip, lang })
    }
}

pub fn canonical_ip(ip: std::net::IpAddr) -> std::net::IpAddr {
    match ip {
        std::net::IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => std::net::IpAddr::V4(v4),
            None => std::net::IpAddr::V6(v6),
        },
        v => v,
    }
}

fn cookie_value(parts: &Parts, name: &str) -> Option<String> {
    for v in parts.headers.get_all(header::COOKIE) {
        let s = v.to_str().ok()?;
        for kv in s.split(';') {
            let kv = kv.trim();
            if let Some(rest) = kv.strip_prefix(name) {
                if let Some(val) = rest.strip_prefix('=') {
                    return Some(val.to_string());
                }
            }
        }
    }
    None
}

#[derive(Clone)]
pub struct AdminSession {
    pub admin_id: i64,
    pub username: String,
    pub csrf: String,
    pub stage: String,
    pub token_hash: Vec<u8>,
    pub ip: String,
    pub lang: Lang,
}

impl AdminSession {
    pub fn actor(&self) -> crate::audit::Actor {
        crate::audit::Actor { kind: "admin", id: self.admin_id.to_string(), name: self.username.clone(), ip: self.ip.clone() }
    }
}

fn ip_allowed(st: &St, ip: &str) -> bool {
    let list = st.settings.read().admin_ip_allowlist.clone();
    if list.is_empty() {
        return true;
    }
    let Ok(addr) = ip.parse::<std::net::IpAddr>() else { return false };
    list.iter().any(|n| {
        if let Ok(net) = n.parse::<ipnet::IpNet>() {
            net.contains(&addr)
        } else {
            n.parse::<std::net::IpAddr>().map(|a| a == addr).unwrap_or(false)
        }
    })
}

async fn load_admin_session(parts: &mut Parts, st: &St) -> Result<AdminSession, AppError> {
    let ctx = Ctx::from_request_parts(parts, st).await?;
    if !ip_allowed(st, &ctx.ip) {
        return Err(AppError::new(StatusCode::FORBIDDEN, "ip_not_allowed").lang(ctx.lang));
    }
    let token = cookie_value(parts, ADMIN_COOKIE).ok_or_else(|| AppError::unauthorized().lang(ctx.lang))?;
    let th = token_hash(&token);
    let now = galaxy_proto::now_ms();
    let th2 = th.clone();
    let row = st
        .db
        .call(move |c| {
            c.query_row(
                "SELECT s.admin_id, a.username, s.csrf, s.stage FROM admin_sessions s JOIN admins a ON a.id = s.admin_id
                 WHERE s.token_hash = ?1 AND s.expires_at > ?2",
                params![th2, now],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)),
            )
            .optional()
        })
        .await?;
    let (admin_id, username, csrf, stage) = row.ok_or_else(|| AppError::unauthorized().lang(ctx.lang))?;
    if parts.method != Method::GET && parts.method != Method::HEAD {
        let h = parts.headers.get("x-csrf-token").and_then(|v| v.to_str().ok()).unwrap_or("");
        if !galaxy_crypto::ct_eq(h.as_bytes(), csrf.as_bytes()) {
            return Err(AppError::new(StatusCode::FORBIDDEN, "csrf").lang(ctx.lang));
        }
    }
    Ok(AdminSession { admin_id, username, csrf, stage, token_hash: th, ip: ctx.ip, lang: ctx.lang })
}

/// 已完成两步验证绑定的管理员。
pub struct Admin(pub AdminSession);

impl FromRequestParts<St> for Admin {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, st: &St) -> Result<Self, Self::Rejection> {
        let s = load_admin_session(parts, st).await?;
        if s.stage != "done" {
            return Err(AppError::new(StatusCode::FORBIDDEN, "totp_required").lang(s.lang));
        }
        Ok(Admin(s))
    }
}

/// 任意阶段的管理员会话（用于两步验证绑定、me、登出）。
pub struct AdminAny(pub AdminSession);

impl FromRequestParts<St> for AdminAny {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, st: &St) -> Result<Self, Self::Rejection> {
        Ok(AdminAny(load_admin_session(parts, st).await?))
    }
}

#[derive(Clone)]
pub struct UserSession {
    pub user_id: u64,
    pub username: String,
    pub device_id: galaxy_proto::DeviceId,
    pub restricted: bool,
    pub token_hash: Vec<u8>,
    pub ip: String,
    pub lang: Lang,
}

impl UserSession {
    pub fn actor(&self) -> crate::audit::Actor {
        crate::audit::Actor { kind: "user", id: self.user_id.to_string(), name: self.username.clone(), ip: self.ip.clone() }
    }
}

async fn load_user_session(parts: &mut Parts, st: &St) -> Result<UserSession, AppError> {
    let ctx = Ctx::from_request_parts(parts, st).await?;
    let auth = parts
        .headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| AppError::unauthorized().lang(ctx.lang))?;
    let th = token_hash(auth.trim());
    let th2 = th.clone();
    let now = galaxy_proto::now_ms();
    let idle_ms = st.settings.read().session_idle_days as i64 * 86_400_000;
    let row = st
        .db
        .call(move |c| {
            let r = c
                .query_row(
                    "SELECT s.user_id, u.username, s.device_id, s.restricted, s.last_used_at, u.disabled
                     FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.token_hash = ?1",
                    params![th2],
                    |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, Vec<u8>>(2)?,
                            r.get::<_, i64>(3)?,
                            r.get::<_, i64>(4)?,
                            r.get::<_, i64>(5)?,
                        ))
                    },
                )
                .optional()?;
            if let Some((_, _, _, _, last, _)) = &r {
                if now - *last > 60_000 {
                    c.execute("UPDATE sessions SET last_used_at = ?1 WHERE token_hash = ?2", params![now, th2])?;
                }
            }
            Ok(r)
        })
        .await?;
    let (uid, username, dev, restricted, last, disabled) = row.ok_or_else(|| AppError::unauthorized().lang(ctx.lang))?;
    if disabled != 0 {
        return Err(AppError::new(StatusCode::FORBIDDEN, "disabled").lang(ctx.lang));
    }
    if idle_ms > 0 && now - last > idle_ms {
        return Err(AppError::unauthorized().lang(ctx.lang));
    }
    let device_id = galaxy_proto::DeviceId::from_slice(&dev).ok_or_else(|| AppError::unauthorized())?;
    Ok(UserSession { user_id: uid as u64, username, device_id, restricted: restricted != 0, token_hash: th, ip: ctx.ip, lang: ctx.lang })
}

/// 已完成首次改密码的用户会话。
pub struct User(pub UserSession);

impl FromRequestParts<St> for User {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, st: &St) -> Result<Self, Self::Rejection> {
        let s = load_user_session(parts, st).await?;
        if s.restricted {
            return Err(AppError::new(StatusCode::FORBIDDEN, "must_change_password").lang(s.lang));
        }
        Ok(User(s))
    }
}

/// 任意用户会话（包括必须改密码的受限会话）。
pub struct UserAny(pub UserSession);

impl FromRequestParts<St> for UserAny {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, st: &St) -> Result<Self, Self::Rejection> {
        Ok(UserAny(load_user_session(parts, st).await?))
    }
}
