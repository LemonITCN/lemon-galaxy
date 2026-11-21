//! 统一错误：HTTP 状态码 + `{ "error": { "code", "message" } }`，message 按 Accept-Language 本地化。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    pub fn from_header(h: Option<&str>) -> Self {
        match h {
            Some(v) if v.to_ascii_lowercase().starts_with("en") => Lang::En,
            _ => Lang::Zh,
        }
    }
}

#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    pub code: &'static str,
    pub detail: Option<String>,
    pub lang: Lang,
}

impl AppError {
    pub fn new(status: StatusCode, code: &'static str) -> Self {
        Self { status, code, detail: None, lang: Lang::Zh }
    }
    pub fn detail(mut self, d: impl Into<String>) -> Self {
        self.detail = Some(d.into());
        self
    }
    pub fn lang(mut self, l: Lang) -> Self {
        self.lang = l;
        self
    }
    pub fn bad_request(d: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request").detail(d)
    }
    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized")
    }
    pub fn forbidden() -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden")
    }
    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found")
    }
    pub fn conflict(code: &'static str) -> Self {
        Self::new(StatusCode::CONFLICT, code)
    }
    pub fn internal(e: impl std::fmt::Display) -> Self {
        tracing::error!("内部错误: {e}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal")
    }
}

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(e: E) -> Self {
        AppError::internal(e.into())
    }
}

pub type AppResult<T> = Result<T, AppError>;

pub fn message(code: &str, lang: Lang) -> &'static str {
    let (zh, en) = match code {
        "bad_request" => ("请求参数不正确", "Invalid request"),
        "unauthorized" => ("未登录或登录已过期", "Not signed in or session expired"),
        "forbidden" => ("没有权限", "Forbidden"),
        "not_found" => ("不存在", "Not found"),
        "conflict" => ("数据已被修改，请刷新后重试", "Conflict, please refresh and retry"),
        "rate_limited" => ("操作太频繁，请稍后再试", "Too many requests, try again later"),
        "locked" => ("连续失败次数过多，账号已临时锁定", "Too many failed attempts, account temporarily locked"),
        "need_totp" => ("请输入两步验证码", "Two-factor code required"),
        "totp_invalid" => ("两步验证码不正确", "Invalid two-factor code"),
        "must_change_password" => ("首次登录请先修改密码", "Please change your password first"),
        "setup_required" => ("服务器尚未初始化", "Server setup required"),
        "setup_done" => ("服务器已初始化", "Server already set up"),
        "invalid_token" => ("初始化令牌不正确", "Invalid setup token"),
        "invalid_credentials" => ("用户名或密码不正确", "Invalid username or password"),
        "disabled" => ("账号已被禁用", "Account disabled"),
        "username_taken" => ("用户名已存在", "Username already exists"),
        "invalid_username" => ("用户名需为 3 到 32 位字母、数字或 ._-", "Username must be 3-32 letters, digits or ._-"),
        "weak_password" => ("密码至少 8 位", "Password must be at least 8 characters"),
        "last_admin" => ("不能删除最后一个管理员", "Cannot delete the last administrator"),
        "self_delete" => ("不能删除自己", "Cannot delete yourself"),
        "totp_required" => ("请先绑定两步验证", "Two-factor setup required"),
        "ip_not_allowed" => ("当前 IP 不允许访问管理后台", "Your IP is not allowed to access the console"),
        "csrf" => ("页面已过期，请刷新", "Page expired, please refresh"),
        "device_conflict" => ("设备标识冲突", "Device identity conflict"),
        "account_mismatch" => ("账号密钥不匹配", "Account key mismatch"),
        "ai_disabled" => ("未开通 AI 功能", "AI is not enabled for this account"),
        "model_not_granted" => ("未开通该模型", "Model not granted"),
        "quota_exceeded" => ("AI 用量已达上限", "AI quota exceeded"),
        "provider_error" => ("大模型服务出错", "AI provider error"),
        "offline" => ("设备不在线", "Device offline"),
        "not_implemented" => ("暂不支持", "Not implemented"),
        "internal" => ("服务器内部错误", "Internal server error"),
        _ => ("请求失败", "Request failed"),
    };
    match lang {
        Lang::Zh => zh,
        Lang::En => en,
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let mut msg = message(self.code, self.lang).to_string();
        if let Some(d) = &self.detail {
            if self.status != StatusCode::INTERNAL_SERVER_ERROR && !d.is_empty() {
                msg = format!("{msg}：{d}");
            }
        }
        (self.status, Json(json!({ "error": { "code": self.code, "message": msg } }))).into_response()
    }
}
