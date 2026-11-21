//! 登录（OPAQUE）、首次登录改密码并生成账号密钥、改密码、签发本设备证书。见 docs/security.md 第 3–4 节。

use crate::api::Api;
use crate::error::{ClientError, Result};
use crate::identity::Identity;
use galaxy_crypto::{opaque, AccountSecrets};
use galaxy_proto::api::*;
use galaxy_proto::cert::{DeviceCert, SignedBlob};
use galaxy_proto::{DeviceKind, Platform};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceMeta {
    pub kind: DeviceKind,
    pub platform: Platform,
    pub os_version: String,
    pub arch: String,
    pub hostname: String,
    pub app_version: String,
}

impl DeviceMeta {
    fn registration(&self, id: &Identity) -> DeviceRegistration {
        DeviceRegistration {
            device_id: id.device_id,
            device_pubkey: id.key.public().to_vec(),
            kind: self.kind,
            platform: self.platform,
            os_version: self.os_version.clone(),
            arch: self.arch.clone(),
            hostname: self.hostname.clone(),
            app_version: self.app_version.clone(),
        }
    }
}

/// 登录完成后的账号状态。主控设备保留 `secrets`，被控设备签完证书即丢弃。
#[derive(Clone)]
pub struct LoggedIn {
    pub token: String,
    pub user: UserSelf,
    pub account_pubkey: [u8; 32],
    pub epoch: u32,
    pub secrets: Option<AccountSecrets>,
    pub cert: SignedBlob,
}

pub enum LoginOutcome {
    Done(LoggedIn),
    NeedTotp,
    MustChangePassword(FirstLogin),
}

/// 首次登录（或管理员重置后）必须先改密码。
pub struct FirstLogin {
    pub token: String,
    pub user: UserSelf,
    pub epoch: u32,
}

fn sign_cert(id: &Identity, meta: &DeviceMeta, secrets: &AccountSecrets, user_id: u64, epoch: u32) -> SignedBlob {
    let cert = DeviceCert {
        v: 1,
        account_id: user_id,
        account_epoch: epoch,
        device_id: id.device_id,
        device_pubkey: id.key.public(),
        kind: meta.kind,
        issued_at: galaxy_proto::now_secs(),
    };
    galaxy_crypto::sign_device_cert(secrets, &cert)
}

async fn upload_cert(api: &Api, cert: &SignedBlob) -> Result<()> {
    let _: OkBody = api.post("/devices/certificate", &CertUploadReq { cert: cert.data.clone(), signature: cert.sig.clone() }).await?;
    Ok(())
}

/// 完整登录。成功后 `api` 已带上会话令牌。
pub async fn login(api: &Api, id: &Identity, meta: &DeviceMeta, username: &str, password: &str, totp: Option<&str>) -> Result<LoginOutcome> {
    let (cst, req) = opaque::client_login_start(password)?;
    let start: LoginStartResp = api.post("/auth/login/start", &LoginStartReq { username: username.trim().to_string(), opaque_request: req }).await?;
    let out = match opaque::client_login_finish(&cst, password, &start.opaque_response) {
        Ok(o) => o,
        Err(_) => {
            // 密码错误在本地就能判断；仍向服务器报告一次失败，让服务器计入锁定
            let _ = api
                .post::<_, serde_json::Value>(
                    "/auth/login/finish",
                    &LoginFinishReq { login_id: start.login_id, opaque_finalization: vec![0u8; 64], totp_code: None, device: meta.registration(id) },
                )
                .await;
            return Err(ClientError::Auth { code: "invalid_credentials".into(), message: "用户名或密码不正确".into() });
        }
    };
    let fin = LoginFinishReq {
        login_id: start.login_id,
        opaque_finalization: out.finalization,
        totp_code: totp.map(|s| s.to_string()),
        device: meta.registration(id),
    };
    let resp: LoginFinishResp = match api.post("/auth/login/finish", &fin).await {
        Ok(r) => r,
        Err(ClientError::Auth { code, .. }) if code == "need_totp" => return Ok(LoginOutcome::NeedTotp),
        Err(e) => return Err(e),
    };
    api.set_token(Some(resp.session_token.clone()));
    if resp.must_change_password || resp.account.wrapped_secrets.is_none() {
        return Ok(LoginOutcome::MustChangePassword(FirstLogin { token: resp.session_token, user: resp.user, epoch: resp.account.epoch }));
    }
    let (Some(wrapped), Some(nonce), Some(pk)) = (resp.account.wrapped_secrets, resp.account.nonce, resp.account.pubkey) else {
        return Err(ClientError::Protocol("账号数据不完整".into()));
    };
    let secrets = AccountSecrets::unwrap(&out.export_key, resp.user.id, resp.account.epoch, &nonce, &wrapped)
        .map_err(|_| ClientError::Protocol("无法解开账号密钥".into()))?;
    let pubkey: [u8; 32] = pk.as_slice().try_into().map_err(|_| ClientError::Protocol("账号公钥格式错误".into()))?;
    if secrets.public() != pubkey {
        return Err(ClientError::Protocol("账号密钥与服务器记录不一致".into()));
    }
    let cert = sign_cert(id, meta, &secrets, resp.user.id, resp.account.epoch);
    upload_cert(api, &cert).await?;
    Ok(LoginOutcome::Done(LoggedIn { token: resp.session_token, user: resp.user, account_pubkey: pubkey, epoch: resp.account.epoch, secrets: Some(secrets), cert }))
}

/// 首次登录：设新密码并生成账号密钥。
pub async fn complete_first_login(api: &Api, id: &Identity, meta: &DeviceMeta, fl: FirstLogin, new_password: &str) -> Result<LoggedIn> {
    if new_password.chars().count() < 8 {
        return Err(ClientError::Auth { code: "weak_password".into(), message: "密码至少 8 位".into() });
    }
    api.set_token(Some(fl.token.clone()));
    let (st, req) = opaque::client_register_start(new_password)?;
    let start: PasswordStartResp = api.post("/auth/password/start", &PasswordStartReq { registration_request: req }).await?;
    let (upload, export_key) = opaque::client_register_finish(&st, new_password, &start.registration_response)?;
    let secrets = AccountSecrets::generate();
    let (nonce, wrapped) = secrets.wrap(&export_key, fl.user.id, fl.epoch);
    let pubkey = secrets.public();
    let _: OkBody = api
        .post(
            "/auth/password/finish",
            &PasswordFinishReq {
                registration_upload: upload,
                account: AccountMaterial { epoch: fl.epoch, pubkey: Some(pubkey.to_vec()), wrapped_secrets: Some(wrapped), nonce: Some(nonce) },
            },
        )
        .await?;
    let cert = sign_cert(id, meta, &secrets, fl.user.id, fl.epoch);
    upload_cert(api, &cert).await?;
    Ok(LoggedIn { token: fl.token, user: fl.user, account_pubkey: pubkey, epoch: fl.epoch, secrets: Some(secrets), cert })
}

/// 修改密码：先用旧密码重新登录验证，再用新密码注册并重新包裹账号密钥（密钥不变，各设备无感）。
pub async fn change_password(api: &Api, id: &Identity, meta: &DeviceMeta, username: &str, old: &str, new: &str, totp: Option<&str>) -> Result<LoggedIn> {
    if new.chars().count() < 8 {
        return Err(ClientError::Auth { code: "weak_password".into(), message: "密码至少 8 位".into() });
    }
    let logged = match login(api, id, meta, username, old, totp).await? {
        LoginOutcome::Done(l) => l,
        LoginOutcome::NeedTotp => return Err(ClientError::Auth { code: "need_totp".into(), message: "请输入两步验证码".into() }),
        LoginOutcome::MustChangePassword(fl) => return complete_first_login(api, id, meta, fl, new).await,
    };
    let secrets = logged.secrets.clone().ok_or_else(|| ClientError::Protocol("缺少账号密钥".into()))?;
    let (st, req) = opaque::client_register_start(new)?;
    let start: PasswordStartResp = api.post("/auth/password/start", &PasswordStartReq { registration_request: req }).await?;
    let (upload, export_key) = opaque::client_register_finish(&st, new, &start.registration_response)?;
    let (nonce, wrapped) = secrets.wrap(&export_key, logged.user.id, logged.epoch);
    let _: OkBody = api
        .post(
            "/auth/password/finish",
            &PasswordFinishReq {
                registration_upload: upload,
                account: AccountMaterial { epoch: logged.epoch, pubkey: Some(logged.account_pubkey.to_vec()), wrapped_secrets: Some(wrapped), nonce: Some(nonce) },
            },
        )
        .await?;
    Ok(logged)
}
