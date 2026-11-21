#![allow(deprecated)]
//! 客户端接口 /api/v1/*（docs/api.md 第二部分）：登录、改密码、设备、加密库、吊销列表。

use crate::admin::{totp, totp_ok};
use crate::audit::{self, Target};
use crate::auth::{new_token, token_hash, Ctx, User, UserAny};
use crate::error::{AppError, AppResult};
use crate::state::{bump_stat, PendingLogin, St};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use galaxy_proto::api::*;
use galaxy_proto::cert::{DeviceCert, SignedBlob};
use galaxy_proto::ws::WsMsg;
use galaxy_proto::{DeviceId, DeviceKind};
use r2d2_sqlite::rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Instant;

pub fn routes() -> Router<St> {
    Router::new()
        .route("/info", get(info))
        .route("/auth/login/start", post(login_start))
        .route("/auth/login/finish", post(login_finish))
        .route("/auth/password/start", post(password_start))
        .route("/auth/password/finish", post(password_finish))
        .route("/auth/totp/setup", post(totp_setup))
        .route("/auth/totp/verify", post(totp_verify))
        .route("/auth/totp/disable", post(totp_disable))
        .route("/auth/logout", post(logout))
        .route("/account", get(account))
        .route("/account/revocations", get(get_revocations).put(put_revocations))
        .route("/devices", get(list_devices))
        .route("/devices/certificate", post(upload_cert))
        .route("/devices/{id}", delete(remove_device))
        .route("/vault", get(vault_list))
        .route("/vault/{id}", put(vault_put).delete(vault_delete))
        .route("/updates/latest", get(crate::update::latest))
        .route("/updates/files/{id}", get(crate::update::file))
}

async fn info(State(st): State<St>) -> Json<ServerInfoResp> {
    Json(ServerInfoResp {
        server_name: st.settings.read().server_name.clone(),
        version: galaxy_proto::APP_VERSION.into(),
        server_sign_pubkey: st.keys.sign.public().to_vec(),
        min_client_version: "0.1.0".into(),
        features: vec!["ai".into(), "relay".into(), "assist".into()],
        udp_port: st.port,
    })
}

// ---------------- 登录 ----------------

struct UserRow {
    id: i64,
    username: String,
    display_name: String,
    record: Vec<u8>,
    disabled: bool,
    must_change: bool,
    totp_secret: Option<Vec<u8>>,
    totp_enabled: bool,
    epoch: i64,
    pubkey: Option<Vec<u8>>,
    wrapped: Option<Vec<u8>>,
    nonce: Option<Vec<u8>>,
    ai_enabled: bool,
    #[allow(dead_code)]
    failed: i64,
    locked_until: i64,
}

async fn load_user(st: &St, username: String) -> anyhow::Result<Option<UserRow>> {
    st.db
        .call(move |c| {
            c.query_row(
                "SELECT id, username, display_name, opaque_record, disabled, must_change_password, totp_secret, totp_enabled,
                        account_epoch, account_pubkey, wrapped_secrets, wrapped_nonce, ai_enabled, failed_logins, locked_until
                 FROM users WHERE username = ?1",
                params![username],
                |r| {
                    Ok(UserRow {
                        id: r.get(0)?,
                        username: r.get(1)?,
                        display_name: r.get(2)?,
                        record: r.get(3)?,
                        disabled: r.get::<_, i64>(4)? != 0,
                        must_change: r.get::<_, i64>(5)? != 0,
                        totp_secret: r.get(6)?,
                        totp_enabled: r.get::<_, i64>(7)? != 0,
                        epoch: r.get(8)?,
                        pubkey: r.get(9)?,
                        wrapped: r.get(10)?,
                        nonce: r.get(11)?,
                        ai_enabled: r.get::<_, i64>(12)? != 0,
                        failed: r.get(13)?,
                        locked_until: r.get(14)?,
                    })
                },
            )
            .optional()
        })
        .await
}

async fn login_start(State(st): State<St>, ctx: Ctx, Json(req): Json<LoginStartReq>) -> AppResult<Json<LoginStartResp>> {
    let uname = req.username.trim().to_lowercase();
    if !st.rate.check(&format!("login-ip:{}", ctx.ip), 30, 60) || !st.rate.check(&format!("login-user:{uname}"), 20, 60) {
        return Err(AppError::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited").lang(ctx.lang));
    }
    let user = load_user(&st, uname.clone()).await?;
    let (sstate, resp) = st
        .keys
        .opaque
        .login_start(user.as_ref().map(|u| u.record.as_slice()), &req.opaque_request, &uname)
        .map_err(|e| AppError::bad_request(e.to_string()).lang(ctx.lang))?;
    let login_id = new_token();
    {
        let mut l = st.logins.lock();
        l.retain(|_, p| p.created.elapsed().as_secs() < 120);
        l.insert(
            login_id.clone(),
            PendingLogin { user_id: user.map(|u| u.id as u64), username: uname, server_state: sstate, created: Instant::now() },
        );
    }
    Ok(Json(LoginStartResp { login_id, opaque_response: resp }))
}

async fn register_failure(st: &St, ctx: &Ctx, user_id: i64, username: &str) -> AppError {
    let (thr, mins) = {
        let s = st.settings.read();
        (s.login_lockout_threshold as i64, s.login_lockout_minutes as i64)
    };
    let now = galaxy_proto::now_ms();
    let locked = st
        .db
        .call(move |c| {
            c.execute("UPDATE users SET failed_logins = failed_logins + 1 WHERE id = ?1", params![user_id])?;
            let f: i64 = c.query_row("SELECT failed_logins FROM users WHERE id = ?1", params![user_id], |r| r.get(0))?;
            if thr > 0 && f >= thr {
                c.execute("UPDATE users SET locked_until = ?1, failed_logins = 0 WHERE id = ?2", params![now + mins * 60_000, user_id])?;
                return Ok(true);
            }
            Ok(false)
        })
        .await
        .unwrap_or(false);
    let actor = audit::Actor { kind: "user", id: user_id.to_string(), name: username.to_string(), ip: ctx.ip.clone() };
    audit::log(st, &actor, if locked { "user.locked" } else { "user.login_failed" }, Target::none(), json!({})).await;
    AppError::new(StatusCode::UNAUTHORIZED, "invalid_credentials").lang(ctx.lang)
}

async fn login_finish(State(st): State<St>, ctx: Ctx, Json(req): Json<LoginFinishReq>) -> AppResult<Json<LoginFinishResp>> {
    let pending = st.logins.lock().remove(&req.login_id);
    let pending = pending.ok_or_else(|| AppError::new(StatusCode::UNAUTHORIZED, "invalid_credentials").lang(ctx.lang))?;
    let Some(uid) = pending.user_id else {
        return Err(AppError::new(StatusCode::UNAUTHORIZED, "invalid_credentials").lang(ctx.lang));
    };
    let user = load_user(&st, pending.username.clone()).await?.ok_or_else(|| AppError::unauthorized().lang(ctx.lang))?;
    if user.id as u64 != uid {
        return Err(AppError::unauthorized().lang(ctx.lang));
    }
    let now = galaxy_proto::now_ms();
    if user.locked_until > now {
        return Err(AppError::new(StatusCode::LOCKED, "locked").lang(ctx.lang));
    }
    if st.keys.opaque.login_finish(&pending.server_state, &req.opaque_finalization).is_err() {
        return Err(register_failure(&st, &ctx, user.id, &user.username).await);
    }
    if user.disabled {
        return Err(AppError::new(StatusCode::FORBIDDEN, "disabled").lang(ctx.lang));
    }
    if user.totp_enabled {
        let secret = user.totp_secret.as_deref().and_then(|b| st.keys.open(b, "user-totp")).unwrap_or_default();
        match req.totp_code.as_deref() {
            None | Some("") => return Err(AppError::new(StatusCode::UNAUTHORIZED, "need_totp").lang(ctx.lang)),
            Some(code) if totp_ok(&secret, &user.username, code) => {}
            Some(_) => return Err(register_failure(&st, &ctx, user.id, &user.username).await),
        }
    }

    // 设备登记
    let dev = req.device.clone();
    if dev.device_pubkey.len() != 32 || dev.device_id.is_zero() {
        return Err(AppError::bad_request("device").lang(ctx.lang));
    }
    let token = new_token();
    let th = token_hash(&token);
    let (uid_i, ip, must) = (user.id, ctx.ip.clone(), user.must_change);
    let conflict = st
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            let raw = dev.device_id.0.to_vec();
            let existing: Option<Vec<u8>> = tx.query_row("SELECT pubkey FROM devices WHERE id = ?1", params![raw], |r| r.get(0)).optional()?;
            match existing {
                Some(pk) if pk != dev.device_pubkey => return Ok(true),
                Some(_) => {
                    tx.execute(
                        "UPDATE devices SET kind=?2, platform=?3, os_version=?4, arch=?5, hostname=?6, app_version=?7, last_ip=?8, last_seen_at=?9 WHERE id=?1",
                        params![raw, dev.kind.as_str(), dev.platform.as_str(), dev.os_version, dev.arch, dev.hostname, dev.app_version, ip, now],
                    )?;
                }
                None => {
                    tx.execute(
                        "INSERT INTO devices(id, pubkey, kind, platform, os_version, arch, hostname, app_version, last_ip, last_seen_at, created_at)
                         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10)",
                        params![raw, dev.device_pubkey, dev.kind.as_str(), dev.platform.as_str(), dev.os_version, dev.arch, dev.hostname, dev.app_version, ip, now],
                    )?;
                }
            }
            tx.execute(
                "INSERT INTO sessions(token_hash, user_id, device_id, restricted, ip, created_at, last_used_at) VALUES(?1,?2,?3,?4,?5,?6,?6)",
                params![th, uid_i, raw, must as i64, ip, now],
            )?;
            tx.execute("UPDATE users SET failed_logins = 0, last_login_at = ?1 WHERE id = ?2", params![now, uid_i])?;
            tx.commit()?;
            Ok(false)
        })
        .await?;
    if conflict {
        return Err(AppError::conflict("device_conflict").lang(ctx.lang));
    }
    bump_stat(&st, "logins", 1).await;
    let actor = audit::Actor { kind: "user", id: user.id.to_string(), name: user.username.clone(), ip: ctx.ip.clone() };
    audit::log(&st, &actor, "user.login", Target::new("device", req.device.device_id.to_hex(), &req.device.hostname), json!({ "kind": req.device.kind.as_str(), "platform": req.device.platform.as_str() })).await;

    Ok(Json(LoginFinishResp {
        session_token: token,
        user: UserSelf {
            id: user.id as u64,
            username: user.username.clone(),
            display_name: user.display_name.clone(),
            ai_enabled: user.ai_enabled,
            totp_enabled: user.totp_enabled,
        },
        must_change_password: user.must_change,
        account: AccountMaterial {
            epoch: user.epoch as u32,
            pubkey: if user.must_change { None } else { user.pubkey.clone() },
            wrapped_secrets: if user.must_change { None } else { user.wrapped.clone() },
            nonce: if user.must_change { None } else { user.nonce.clone() },
        },
    }))
}

async fn password_start(State(st): State<St>, UserAny(s): UserAny, Json(req): Json<PasswordStartReq>) -> AppResult<Json<PasswordStartResp>> {
    if !st.rate.check(&format!("pw:{}", s.user_id), 10, 60) {
        return Err(AppError::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited").lang(s.lang));
    }
    // 改密码必须用 10 分钟内刚登录（验证过旧密码）的会话，被盗的长期令牌改不了密码
    if !s.restricted && galaxy_proto::now_ms() - s.created_at > 10 * 60_000 {
        return Err(AppError::new(StatusCode::UNAUTHORIZED, "unauthorized").detail("请重新输入旧密码").lang(s.lang));
    }
    let resp = st
        .keys
        .opaque
        .register_start(&req.registration_request, &s.username)
        .map_err(|e| AppError::bad_request(e.to_string()).lang(s.lang))?;
    Ok(Json(PasswordStartResp { registration_response: resp }))
}

async fn password_finish(State(st): State<St>, UserAny(s): UserAny, Json(req): Json<PasswordFinishReq>) -> AppResult<Json<Value>> {
    let record = st
        .keys
        .opaque
        .register_finish(&req.registration_upload)
        .map_err(|e| AppError::bad_request(e.to_string()).lang(s.lang))?;
    let acct = req.account;
    let (Some(pk), Some(wrapped), Some(nonce)) = (acct.pubkey.clone(), acct.wrapped_secrets.clone(), acct.nonce.clone()) else {
        return Err(AppError::bad_request("account").lang(s.lang));
    };
    if pk.len() != 32 {
        return Err(AppError::bad_request("pubkey").lang(s.lang));
    }
    let uid = s.user_id as i64;
    let th = s.token_hash.clone();
    let result = st
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            let (epoch, cur_pk): (i64, Option<Vec<u8>>) =
                tx.query_row("SELECT account_epoch, account_pubkey FROM users WHERE id = ?1", params![uid], |r| Ok((r.get(0)?, r.get(1)?)))?;
            if epoch as u32 != acct.epoch {
                return Ok(Err("account_mismatch"));
            }
            if let Some(cur) = cur_pk {
                if cur != pk {
                    return Ok(Err("account_mismatch"));
                }
            }
            tx.execute(
                "UPDATE users SET opaque_record = ?1, account_pubkey = ?2, wrapped_secrets = ?3, wrapped_nonce = ?4, must_change_password = 0 WHERE id = ?5",
                params![record, pk, wrapped, nonce, uid],
            )?;
            tx.execute("UPDATE sessions SET restricted = 0 WHERE token_hash = ?1", params![th])?;
            tx.commit()?;
            Ok(Ok(()))
        })
        .await?;
    if let Err(code) = result {
        return Err(AppError::conflict(code).lang(s.lang));
    }
    audit::log(&st, &s.actor(), "user.password_change", Target::new("user", s.user_id, &s.username), json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

async fn totp_setup(State(st): State<St>, User(s): User) -> AppResult<Json<TotpSetupResp>> {
    let secret = totp_rs::Secret::generate_secret().to_bytes().map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let t = totp(&secret, &s.username)?;
    let enc = st.keys.seal(&secret, "user-totp-pending");
    let uid = s.user_id as i64;
    st.db
        .call(move |c| c.execute("UPDATE users SET totp_secret = ?1 WHERE id = ?2 AND totp_enabled = 0", params![enc, uid]))
        .await?;
    Ok(Json(TotpSetupResp { secret: t.get_secret_base32(), otpauth_uri: t.get_url() }))
}

async fn totp_verify(State(st): State<St>, User(s): User, Json(req): Json<TotpCodeReq>) -> AppResult<Json<Value>> {
    let uid = s.user_id as i64;
    let enc: Option<Vec<u8>> = st.db.call(move |c| c.query_row("SELECT totp_secret FROM users WHERE id=?1", params![uid], |r| r.get(0))).await?;
    let secret = enc.as_deref().and_then(|b| st.keys.open(b, "user-totp-pending")).ok_or_else(|| AppError::bad_request("").lang(s.lang))?;
    if !totp_ok(&secret, &s.username, &req.code) {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "totp_invalid").lang(s.lang));
    }
    let fin = st.keys.seal(&secret, "user-totp");
    st.db.call(move |c| c.execute("UPDATE users SET totp_secret = ?1, totp_enabled = 1 WHERE id = ?2", params![fin, uid])).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn totp_disable(State(st): State<St>, User(s): User, Json(req): Json<TotpCodeReq>) -> AppResult<Json<Value>> {
    let uid = s.user_id as i64;
    let (enc, required): (Option<Vec<u8>>, i64) = st
        .db
        .call(move |c| c.query_row("SELECT totp_secret, require_totp FROM users WHERE id=?1", params![uid], |r| Ok((r.get(0)?, r.get(1)?))))
        .await?;
    if required != 0 {
        return Err(AppError::forbidden().lang(s.lang));
    }
    let secret = enc.as_deref().and_then(|b| st.keys.open(b, "user-totp")).unwrap_or_default();
    if !totp_ok(&secret, &s.username, &req.code) {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "totp_invalid").lang(s.lang));
    }
    st.db.call(move |c| c.execute("UPDATE users SET totp_secret = NULL, totp_enabled = 0 WHERE id = ?1", params![uid])).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn logout(State(st): State<St>, UserAny(s): UserAny) -> AppResult<Json<Value>> {
    let (th, uid, raw) = (s.token_hash.clone(), s.user_id as i64, s.device_id.0.to_vec());
    st.db
        .call(move |c| {
            c.execute("DELETE FROM sessions WHERE token_hash = ?1", params![th])?;
            c.execute("DELETE FROM device_accounts WHERE device_id = ?1 AND user_id = ?2", params![raw, uid])
        })
        .await?;
    crate::ws::refresh_device_accounts(&st, s.device_id).await;
    st.hub.notify_user(s.user_id, WsMsg::DevicesChanged);
    Ok(Json(json!({ "ok": true })))
}

async fn account(State(st): State<St>, User(s): User) -> AppResult<Json<AccountMaterial>> {
    let uid = s.user_id as i64;
    let (epoch, pk, w, n): (i64, Option<Vec<u8>>, Option<Vec<u8>>, Option<Vec<u8>>) = st
        .db
        .call(move |c| {
            c.query_row(
                "SELECT account_epoch, account_pubkey, wrapped_secrets, wrapped_nonce FROM users WHERE id = ?1",
                params![uid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
        })
        .await?;
    Ok(Json(AccountMaterial { epoch: epoch as u32, pubkey: pk, wrapped_secrets: w, nonce: n }))
}

// ---------------- 设备 ----------------

async fn upload_cert(State(st): State<St>, User(s): User, Json(req): Json<CertUploadReq>) -> AppResult<Json<Value>> {
    let uid = s.user_id as i64;
    let raw = s.device_id.0.to_vec();
    let raw2 = raw.clone();
    let (acct_pk, epoch, dev_pk, dev_kind): (Option<Vec<u8>>, i64, Vec<u8>, String) = st
        .db
        .call(move |c| {
            let (pk, ep): (Option<Vec<u8>>, i64) =
                c.query_row("SELECT account_pubkey, account_epoch FROM users WHERE id = ?1", params![uid], |r| Ok((r.get(0)?, r.get(1)?)))?;
            let (dpk, kind): (Vec<u8>, String) = c.query_row("SELECT pubkey, kind FROM devices WHERE id = ?1", params![raw2], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok((pk, ep, dpk, kind))
        })
        .await?;
    let acct_pk = acct_pk.ok_or_else(|| AppError::new(StatusCode::FORBIDDEN, "must_change_password").lang(s.lang))?;
    let blob = SignedBlob { data: req.cert.clone(), sig: req.signature.clone() };
    let cert: DeviceCert = galaxy_crypto::verify_device_cert(&acct_pk, &blob).map_err(|_| AppError::bad_request("cert").lang(s.lang))?;
    if cert.account_id != s.user_id
        || cert.account_epoch as i64 != epoch
        || cert.device_id != s.device_id
        || cert.device_pubkey.as_slice() != dev_pk.as_slice()
        || cert.kind.as_str() != dev_kind
    {
        return Err(AppError::bad_request("cert").lang(s.lang));
    }
    let now = galaxy_proto::now_ms();
    let single = matches!(cert.kind, DeviceKind::Control | DeviceKind::Mobile);
    let removed: Vec<i64> = st
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            let mut removed = vec![];
            if single {
                // 主控设备同一时间只能登录一个账号
                let mut stmt = tx.prepare("SELECT user_id FROM device_accounts WHERE device_id = ?1 AND user_id != ?2")?;
                removed = stmt.query_map(params![raw, uid], |r| r.get(0))?.collect::<Result<Vec<i64>, _>>()?;
                drop(stmt);
                tx.execute("DELETE FROM device_accounts WHERE device_id = ?1 AND user_id != ?2", params![raw, uid])?;
                tx.execute("DELETE FROM sessions WHERE device_id = ?1 AND user_id != ?2", params![raw, uid])?;
            }
            tx.execute(
                "INSERT INTO device_accounts(device_id, user_id, status, cert, cert_sig, added_at) VALUES(?1,?2,'active',?3,?4,?5)
                 ON CONFLICT(device_id, user_id) DO UPDATE SET status = 'active', cert = excluded.cert, cert_sig = excluded.cert_sig",
                params![raw, uid, req.cert, req.signature, now],
            )?;
            tx.commit()?;
            Ok(removed)
        })
        .await?;
    crate::ws::refresh_device_accounts(&st, s.device_id).await;
    st.hub.notify_user(s.user_id, WsMsg::DevicesChanged);
    for u in removed {
        st.hub.notify_user(u as u64, WsMsg::DevicesChanged);
    }
    audit::log(&st, &s.actor(), "device.register", Target::new("device", s.device_id.to_hex(), ""), json!({ "kind": cert.kind.as_str() })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn list_devices(State(st): State<St>, User(s): User) -> AppResult<Json<ListResp<ClientDevice>>> {
    let uid = s.user_id as i64;
    let rows = st
        .db
        .call(move |c| {
            let mut stmt = c.prepare(
                "SELECT d.id, d.kind, d.platform, d.os_version, d.arch, d.hostname, d.app_version, d.last_seen_at, da.cert, da.cert_sig, d.pubkey
                 FROM device_accounts da JOIN devices d ON d.id = da.device_id WHERE da.user_id = ?1 AND da.status = 'active'
                 ORDER BY d.hostname",
            )?;
            let v = stmt
                .query_map(params![uid], |r| {
                    Ok((
                        r.get::<_, Vec<u8>>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, i64>(7)?,
                        r.get::<_, Option<Vec<u8>>>(8)?,
                        r.get::<_, Option<Vec<u8>>>(9)?,
                        r.get::<_, Vec<u8>>(10)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(v)
        })
        .await?;
    let hub = st.hub.devices.read();
    let items = rows
        .into_iter()
        .filter_map(|(id, kind, platform, os, arch, host, ver, seen, cert, sig, pk)| {
            let did = DeviceId::from_slice(&id)?;
            let online = hub.get(&did);
            Some(ClientDevice {
                id: did,
                kind: DeviceKind::parse(&kind)?,
                platform: galaxy_proto::Platform::parse(&platform)?,
                os_version: os,
                arch,
                hostname: host,
                app_version: ver,
                online: online.is_some(),
                last_seen_at: seen,
                cert,
                cert_signature: sig,
                public_ip: online.map(|o| o.public_ip.to_string()).unwrap_or_default(),
                device_pubkey: pk,
            })
        })
        .collect();
    Ok(Json(ListResp { items }))
}

async fn remove_device(State(st): State<St>, User(s): User, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let did = DeviceId::from_hex(&id).ok_or_else(|| AppError::bad_request("id").lang(s.lang))?;
    let (raw, uid) = (did.0.to_vec(), s.user_id as i64);
    st.db
        .call(move |c| {
            c.execute("DELETE FROM device_accounts WHERE device_id = ?1 AND user_id = ?2", params![raw, uid])?;
            c.execute("DELETE FROM sessions WHERE device_id = ?1 AND user_id = ?2", params![raw, uid])
        })
        .await?;
    st.hub.send(&did, WsMsg::AccountReset { user_id: s.user_id });
    crate::ws::refresh_device_accounts(&st, did).await;
    st.hub.notify_user(s.user_id, WsMsg::DevicesChanged);
    audit::log(&st, &s.actor(), "device.revoke", Target::new("device", did.to_hex(), ""), json!({ "by_user": true })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn get_revocations(State(st): State<St>, User(s): User) -> AppResult<Json<RevocationsWire>> {
    let uid = s.user_id as i64;
    let (l, sig): (Option<Vec<u8>>, Option<Vec<u8>>) = st
        .db
        .call(move |c| c.query_row("SELECT revocations, revocations_sig FROM users WHERE id = ?1", params![uid], |r| Ok((r.get(0)?, r.get(1)?))))
        .await?;
    Ok(Json(RevocationsWire { list: l, signature: sig }))
}

async fn put_revocations(State(st): State<St>, User(s): User, Json(req): Json<RevocationsWire>) -> AppResult<Json<Value>> {
    let (Some(list), Some(sig)) = (req.list, req.signature) else { return Err(AppError::bad_request("").lang(s.lang)) };
    let uid = s.user_id as i64;
    let pk: Option<Vec<u8>> = st.db.call(move |c| c.query_row("SELECT account_pubkey FROM users WHERE id=?1", params![uid], |r| r.get(0))).await?;
    let pk = pk.ok_or_else(|| AppError::forbidden().lang(s.lang))?;
    let blob = SignedBlob { data: list.clone(), sig: sig.clone() };
    let rl = galaxy_crypto::verify_revocations(&pk, &blob).map_err(|_| AppError::bad_request("signature").lang(s.lang))?;
    if rl.account_id != s.user_id {
        return Err(AppError::bad_request("account").lang(s.lang));
    }
    st.db.call(move |c| c.execute("UPDATE users SET revocations = ?1, revocations_sig = ?2 WHERE id = ?3", params![list, sig, uid])).await?;
    st.hub.notify_user(s.user_id, WsMsg::RevocationsChanged);
    Ok(Json(json!({ "ok": true })))
}

// ---------------- 加密库 ----------------

#[derive(Deserialize)]
struct SinceQ {
    since: Option<i64>,
}

async fn vault_list(State(st): State<St>, User(s): User, Query(q): Query<SinceQ>) -> AppResult<Json<VaultListResp>> {
    let uid = s.user_id as i64;
    let since = q.since.unwrap_or(0);
    let (items, version) = st
        .db
        .call(move |c| {
            let mut stmt = c.prepare(
                "SELECT id, kind, ciphertext, nonce, version, deleted, updated_at FROM vault_items WHERE user_id = ?1 AND version > ?2 ORDER BY version",
            )?;
            let items = stmt
                .query_map(params![uid, since], |r| {
                    Ok(VaultItemWire {
                        id: r.get(0)?,
                        kind: r.get(1)?,
                        ciphertext: r.get(2)?,
                        nonce: r.get(3)?,
                        version: r.get(4)?,
                        deleted: r.get::<_, i64>(5)? != 0,
                        updated_at: r.get(6)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let v: i64 = c.query_row("SELECT vault_version FROM users WHERE id = ?1", params![uid], |r| r.get(0))?;
            Ok((items, v))
        })
        .await?;
    Ok(Json(VaultListResp { items, version }))
}

fn valid_item_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

async fn vault_put(State(st): State<St>, User(s): User, Path(id): Path<String>, Json(req): Json<VaultPutReq>) -> AppResult<Json<VersionResp>> {
    if !valid_item_id(&id) || req.kind.len() > 32 || req.ciphertext.len() > 1024 * 1024 {
        return Err(AppError::bad_request("item").lang(s.lang));
    }
    let uid = s.user_id as i64;
    let now = galaxy_proto::now_ms();
    let r = st
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            let cur: Option<i64> = tx.query_row("SELECT version FROM vault_items WHERE user_id = ?1 AND id = ?2", params![uid, id], |r| r.get(0)).optional()?;
            if cur.unwrap_or(0) != req.base_version {
                return Ok(None);
            }
            tx.execute("UPDATE users SET vault_version = vault_version + 1 WHERE id = ?1", params![uid])?;
            let v: i64 = tx.query_row("SELECT vault_version FROM users WHERE id = ?1", params![uid], |r| r.get(0))?;
            tx.execute(
                "INSERT INTO vault_items(user_id, id, kind, ciphertext, nonce, version, deleted, updated_at) VALUES(?1,?2,?3,?4,?5,?6,0,?7)
                 ON CONFLICT(user_id, id) DO UPDATE SET kind=excluded.kind, ciphertext=excluded.ciphertext, nonce=excluded.nonce, version=excluded.version, deleted=0, updated_at=excluded.updated_at",
                params![uid, id, req.kind, req.ciphertext, req.nonce, v, now],
            )?;
            tx.commit()?;
            Ok(Some(v))
        })
        .await?;
    let v = r.ok_or_else(|| AppError::conflict("conflict").lang(s.lang))?;
    st.hub.notify_user(s.user_id, WsMsg::VaultChanged { version: v });
    Ok(Json(VersionResp { version: v }))
}

#[derive(Deserialize)]
struct BaseQ {
    base_version: Option<i64>,
}

async fn vault_delete(State(st): State<St>, User(s): User, Path(id): Path<String>, Query(q): Query<BaseQ>) -> AppResult<Json<VersionResp>> {
    if !valid_item_id(&id) {
        return Err(AppError::bad_request("item").lang(s.lang));
    }
    let uid = s.user_id as i64;
    let now = galaxy_proto::now_ms();
    let base = q.base_version;
    let r = st
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            let cur: Option<i64> = tx.query_row("SELECT version FROM vault_items WHERE user_id = ?1 AND id = ?2", params![uid, id], |r| r.get(0)).optional()?;
            let Some(cur) = cur else { return Ok(Some(0)) };
            if let Some(b) = base {
                if b != cur {
                    return Ok(None);
                }
            }
            tx.execute("UPDATE users SET vault_version = vault_version + 1 WHERE id = ?1", params![uid])?;
            let v: i64 = tx.query_row("SELECT vault_version FROM users WHERE id = ?1", params![uid], |r| r.get(0))?;
            tx.execute(
                "UPDATE vault_items SET deleted = 1, ciphertext = x'', nonce = x'', version = ?1, updated_at = ?2 WHERE user_id = ?3 AND id = ?4",
                params![v, now, uid, id],
            )?;
            tx.commit()?;
            Ok(Some(v))
        })
        .await?;
    let v = r.ok_or_else(|| AppError::conflict("conflict").lang(s.lang))?;
    st.hub.notify_user(s.user_id, WsMsg::VaultChanged { version: v });
    Ok(Json(VersionResp { version: v }))
}

