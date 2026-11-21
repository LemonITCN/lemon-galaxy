//! 大模型网关：供应商管理、按用户授权、额度、流式转发与用量记账。见 docs/ai-agent.md 第 2 节。

use crate::admin::PageQ;
use crate::audit::{self, Target};
use crate::auth::{Admin, User};
use crate::error::{AppError, AppResult};
use crate::state::{bump_stat, AppState, St};
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::StreamExt;
use galaxy_proto::api::{AiModelInfo, AiModelsResp};
use galaxy_proto::AiMode;
use r2d2_sqlite::rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

pub const OPENCODE_GO_MODELS: &str = "https://opencode.ai/zen/go/v1";
pub const OPENCODE_GO_INFERENCE: &str = "https://opencode.ai/inference/go/openai/v1";

fn user_agent() -> String {
    format!("LemonGalaxy/{}", galaxy_proto::APP_VERSION)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderModel {
    pub id: String,
    pub name: String,
    pub vision: bool,
    pub tools: bool,
    pub context_length: u32,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub chat_base_url: String,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug)]
pub struct Provider {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub enabled: bool,
    pub models: Vec<ProviderModel>,
    pub models_updated_at: i64,
    pub created_at: i64,
}

impl Provider {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "name": self.name, "kind": self.kind, "base_url": self.base_url,
            "has_api_key": self.api_key.as_deref().map(|k| !k.is_empty()).unwrap_or(false),
            "enabled": self.enabled,
            "models": self.models.iter().map(|m| json!({
                "id": m.id, "name": m.name, "vision": m.vision, "tools": m.tools,
                "context_length": m.context_length, "enabled": m.enabled
            })).collect::<Vec<_>>(),
            "models_updated_at": self.models_updated_at, "created_at": self.created_at,
        })
    }

    fn chat_url(&self, model: &ProviderModel) -> String {
        let base = if !model.chat_base_url.is_empty() {
            model.chat_base_url.clone()
        } else if self.kind == "opencode_go" {
            OPENCODE_GO_INFERENCE.to_string()
        } else {
            self.base_url.clone()
        };
        format!("{}/chat/completions", base.trim_end_matches('/'))
    }
}

async fn load_providers(st: &AppState) -> anyhow::Result<Vec<Provider>> {
    let rows: Vec<(i64, String, String, String, Option<Vec<u8>>, i64, String, i64, i64)> = st
        .db
        .call(|c| {
            let mut stmt = c.prepare(
                "SELECT id, name, kind, base_url, api_key_enc, enabled, models_json, models_updated_at, created_at FROM ai_providers ORDER BY id",
            )?;
            let v = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(v)
        })
        .await?;
    Ok(rows
        .into_iter()
        .map(|(id, name, kind, base_url, enc, enabled, models, mu, ca)| Provider {
            id,
            name,
            kind,
            base_url,
            api_key: enc.and_then(|b| st.keys.open(&b, "ai-key")).and_then(|v| String::from_utf8(v).ok()),
            enabled: enabled != 0,
            models: serde_json::from_str(&models).unwrap_or_default(),
            models_updated_at: mu,
            created_at: ca,
        })
        .collect())
}

async fn load_provider(st: &AppState, id: i64) -> AppResult<Provider> {
    load_providers(st).await?.into_iter().find(|p| p.id == id).ok_or_else(AppError::not_found)
}

async fn save_models(st: &AppState, id: i64, models: &[ProviderModel]) -> anyhow::Result<()> {
    let j = serde_json::to_string(models)?;
    let now = galaxy_proto::now_ms();
    st.db
        .call(move |c| c.execute("UPDATE ai_providers SET models_json = ?1, models_updated_at = ?2 WHERE id = ?3", params![j, now, id]))
        .await?;
    Ok(())
}

/// 从供应商拉取模型列表，保留已有的停用设置。
pub async fn refresh_models(st: &AppState, p: &Provider) -> anyhow::Result<Vec<ProviderModel>> {
    let key = p.api_key.clone().unwrap_or_default();
    let base = if p.base_url.is_empty() && p.kind == "opencode_go" { OPENCODE_GO_MODELS.to_string() } else { p.base_url.clone() };
    let url = format!("{}/models", base.trim_end_matches('/'));
    let resp = st
        .http
        .get(&url)
        .bearer_auth(&key)
        .header(header::USER_AGENT, user_agent())
        .header("x-opencode-session", "lemon-galaxy-admin")
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    if !status.is_success() {
        anyhow::bail!("HTTP {status}: {}", body.to_string().chars().take(300).collect::<String>());
    }
    let list = body.get("data").and_then(|d| d.as_array()).cloned().or_else(|| body.as_array().cloned()).unwrap_or_default();
    let mut out = vec![];
    for m in list {
        let Some(id) = m.get("id").and_then(|v| v.as_str()) else { continue };
        let name = m.get("name").and_then(|v| v.as_str()).unwrap_or(id).to_string();
        let chat_base = m
            .pointer("/api/chat-completions/base_url")
            .or_else(|| m.pointer("/api/chat_completions/base_url"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        // OpenCode Go：只有 responses/messages 接口的模型暂不接入
        if p.kind == "opencode_go" && m.get("api").is_some() && chat_base.is_empty() {
            continue;
        }
        let inputs: Vec<String> = m
            .pointer("/modalities/input")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default();
        let vision = inputs.iter().any(|s| s.starts_with("image"))
            || m.get("vision").and_then(|v| v.as_bool()).unwrap_or(false)
            || m.pointer("/capabilities/vision").and_then(|v| v.as_bool()).unwrap_or(false);
        let tools = m
            .get("tool_call")
            .or_else(|| m.pointer("/capabilities/tools"))
            .or_else(|| m.get("tools"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let ctx = m
            .pointer("/limit/context")
            .or_else(|| m.get("context_length"))
            .or_else(|| m.get("context_window"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let enabled = p.models.iter().find(|o| o.id == id).map(|o| o.enabled).unwrap_or(true);
        out.push(ProviderModel { id: id.to_string(), name, vision, tools, context_length: ctx, enabled, chat_base_url: chat_base });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    save_models(st, p.id, &out).await?;
    Ok(out)
}

// ---------------- 后台接口 ----------------

pub async fn admin_list_providers(State(st): State<St>, Admin(_s): Admin) -> AppResult<Json<Value>> {
    let items: Vec<Value> = load_providers(&st).await?.iter().map(|p| p.to_json()).collect();
    Ok(Json(json!({ "items": items })))
}

#[derive(Deserialize)]
pub struct ProviderReq {
    name: Option<String>,
    kind: Option<String>,
    base_url: Option<String>,
    api_key: Option<String>,
    enabled: Option<bool>,
}

pub async fn admin_create_provider(State(st): State<St>, Admin(s): Admin, Json(req): Json<ProviderReq>) -> AppResult<Json<Value>> {
    let kind = req.kind.clone().unwrap_or_else(|| "opencode_go".into());
    if kind != "opencode_go" && kind != "openai_compatible" {
        return Err(AppError::bad_request("kind").lang(s.lang));
    }
    let base_url = match req.base_url.clone().filter(|s| !s.trim().is_empty()) {
        Some(u) => u.trim().trim_end_matches('/').to_string(),
        None if kind == "opencode_go" => OPENCODE_GO_MODELS.to_string(),
        None => return Err(AppError::bad_request("base_url").lang(s.lang)),
    };
    if !base_url.starts_with("https://") && !base_url.starts_with("http://") {
        return Err(AppError::bad_request("base_url").lang(s.lang));
    }
    let name = req.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| if kind == "opencode_go" { "OpenCode Go".into() } else { "OpenAI Compatible".into() });
    let key_enc = req.api_key.as_deref().filter(|k| !k.is_empty()).map(|k| st.keys.seal(k.trim().as_bytes(), "ai-key"));
    let now = galaxy_proto::now_ms();
    let enabled = req.enabled.unwrap_or(true) as i64;
    let (n2, k2, b2) = (name.clone(), kind.clone(), base_url.clone());
    let id = st
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO ai_providers(name, kind, base_url, api_key_enc, enabled, created_at) VALUES(?1,?2,?3,?4,?5,?6)",
                params![n2, k2, b2, key_enc, enabled, now],
            )?;
            Ok(c.last_insert_rowid())
        })
        .await?;
    audit::log(&st, &s.actor(), "ai.provider_create", Target::new("ai_provider", id, &name), json!({ "kind": kind })).await;
    let p = load_provider(&st, id).await?;
    if let Err(e) = refresh_models(&st, &p).await {
        tracing::warn!("拉取模型列表失败: {e}");
    }
    Ok(Json(load_provider(&st, id).await?.to_json()))
}

pub async fn admin_update_provider(State(st): State<St>, Admin(s): Admin, Path(id): Path<i64>, Json(req): Json<ProviderReq>) -> AppResult<Json<Value>> {
    let p = load_provider(&st, id).await.map_err(|e| e.lang(s.lang))?;
    let name = req.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or(p.name.clone());
    let base = req.base_url.clone().filter(|u| !u.trim().is_empty()).map(|u| u.trim().trim_end_matches('/').to_string()).unwrap_or(p.base_url.clone());
    let enabled = req.enabled.unwrap_or(p.enabled) as i64;
    let key_enc = req.api_key.as_deref().filter(|k| !k.is_empty()).map(|k| st.keys.seal(k.trim().as_bytes(), "ai-key"));
    let key_changed = key_enc.is_some();
    st.db
        .call(move |c| {
            c.execute("UPDATE ai_providers SET name = ?1, base_url = ?2, enabled = ?3 WHERE id = ?4", params![name, base, enabled, id])?;
            if let Some(k) = key_enc {
                c.execute("UPDATE ai_providers SET api_key_enc = ?1 WHERE id = ?2", params![k, id])?;
            }
            Ok(())
        })
        .await?;
    audit::log(&st, &s.actor(), "ai.provider_update", Target::new("ai_provider", id, &p.name), json!({ "api_key_changed": key_changed })).await;
    Ok(Json(load_provider(&st, id).await?.to_json()))
}

pub async fn admin_delete_provider(State(st): State<St>, Admin(s): Admin, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let p = load_provider(&st, id).await.map_err(|e| e.lang(s.lang))?;
    st.db
        .call(move |c| {
            c.execute("DELETE FROM ai_providers WHERE id = ?1", params![id])?;
            c.execute("DELETE FROM ai_grants WHERE provider_id = ?1", params![id])
        })
        .await?;
    audit::log(&st, &s.actor(), "ai.provider_delete", Target::new("ai_provider", id, &p.name), json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct ModelToggle {
    id: String,
    enabled: bool,
}

#[derive(Deserialize)]
pub struct ModelsReq {
    items: Vec<ModelToggle>,
}

pub async fn admin_set_models(State(st): State<St>, Admin(s): Admin, Path(id): Path<i64>, Json(req): Json<ModelsReq>) -> AppResult<Json<Value>> {
    let mut p = load_provider(&st, id).await.map_err(|e| e.lang(s.lang))?;
    for t in &req.items {
        if let Some(m) = p.models.iter_mut().find(|m| m.id == t.id) {
            m.enabled = t.enabled;
        }
    }
    save_models(&st, id, &p.models).await?;
    audit::log(&st, &s.actor(), "ai.provider_update", Target::new("ai_provider", id, &p.name), json!({ "models": req.items.iter().map(|t| json!({"id": t.id, "enabled": t.enabled})).collect::<Vec<_>>() })).await;
    Ok(Json(load_provider(&st, id).await?.to_json()))
}

pub async fn admin_refresh_models(State(st): State<St>, Admin(s): Admin, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let p = load_provider(&st, id).await.map_err(|e| e.lang(s.lang))?;
    refresh_models(&st, &p)
        .await
        .map_err(|e| AppError::new(StatusCode::BAD_GATEWAY, "provider_error").detail(e.to_string()).lang(s.lang))?;
    Ok(Json(load_provider(&st, id).await?.to_json()))
}

#[derive(Deserialize)]
pub struct TestReq {
    model: String,
}

pub async fn admin_test_provider(State(st): State<St>, Admin(s): Admin, Path(id): Path<i64>, Json(req): Json<TestReq>) -> AppResult<Json<Value>> {
    let p = load_provider(&st, id).await.map_err(|e| e.lang(s.lang))?;
    let m = p.models.iter().find(|m| m.id == req.model).cloned().unwrap_or(ProviderModel {
        id: req.model.clone(),
        name: req.model.clone(),
        vision: false,
        tools: true,
        context_length: 0,
        enabled: true,
        chat_base_url: String::new(),
    });
    let started = std::time::Instant::now();
    let body = json!({
        "model": m.id,
        "messages": [{ "role": "user", "content": "Reply with the single word OK." }],
        "max_tokens": 64,
        "stream": false,
    });
    let resp = upstream(&st, &p, &m, "lemon-galaxy-admin-test", &body)
        .await
        .map_err(|e| AppError::new(StatusCode::BAD_GATEWAY, "provider_error").detail(e.to_string()).lang(s.lang))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(json!({}));
    if !status.is_success() {
        return Err(AppError::new(StatusCode::BAD_GATEWAY, "provider_error").detail(format!("HTTP {status}: {}", v.to_string().chars().take(300).collect::<String>())).lang(s.lang));
    }
    let reply = v.pointer("/choices/0/message/content").and_then(|x| x.as_str()).unwrap_or("").to_string();
    Ok(Json(json!({ "ok": true, "latency_ms": started.elapsed().as_millis() as u64, "reply": reply })))
}

#[derive(Deserialize)]
pub struct UsageQ {
    from: Option<i64>,
    to: Option<i64>,
    user_id: Option<i64>,
    group_by: Option<String>,
}

pub async fn admin_usage(State(st): State<St>, Admin(_s): Admin, Query(q): Query<UsageQ>) -> AppResult<Json<Value>> {
    let from = q.from.unwrap_or(0);
    let to = q.to.unwrap_or(i64::MAX);
    let uid = q.user_id;
    let group = q.group_by.clone().unwrap_or_else(|| "user".into());
    let key_expr = match group.as_str() {
        "model" => "u.model",
        "day" => "strftime('%Y-%m-%d', u.at / 1000, 'unixepoch')",
        _ => "COALESCE(us.username, CAST(u.user_id AS TEXT))",
    };
    let sql = format!(
        "SELECT {key_expr} AS k, COUNT(*), SUM(u.prompt_tokens), SUM(u.completion_tokens) FROM ai_usage u
         LEFT JOIN users us ON us.id = u.user_id
         WHERE u.at >= ?1 AND u.at <= ?2 AND (?3 IS NULL OR u.user_id = ?3) GROUP BY k ORDER BY k"
    );
    let items: Vec<Value> = st
        .db
        .call(move |c| {
            let mut stmt = c.prepare(&sql)?;
            let v = stmt
                .query_map(params![from, to, uid], |r| {
                    let p: i64 = r.get::<_, Option<i64>>(2)?.unwrap_or(0);
                    let cpl: i64 = r.get::<_, Option<i64>>(3)?.unwrap_or(0);
                    Ok(json!({ "key": r.get::<_, String>(0)?, "requests": r.get::<_, i64>(1)?, "prompt_tokens": p, "completion_tokens": cpl, "total_tokens": p + cpl }))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(v)
        })
        .await?;
    Ok(Json(json!({ "items": items })))
}

// ---------------- 客户端接口 ----------------

struct UserAi {
    enabled: bool,
    max_mode: AiMode,
    daily: i64,
    monthly: i64,
    grants: Vec<(i64, Vec<String>)>,
}

async fn user_ai(st: &AppState, uid: i64) -> anyhow::Result<UserAi> {
    st.db
        .call(move |c| {
            let (enabled, mode, daily, monthly): (i64, String, i64, i64) = c.query_row(
                "SELECT ai_enabled, ai_max_mode, ai_daily_token_limit, ai_monthly_token_limit FROM users WHERE id = ?1",
                params![uid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
            let mut stmt = c.prepare("SELECT provider_id, models_json FROM ai_grants WHERE user_id = ?1")?;
            let grants = stmt
                .query_map(params![uid], |r| Ok((r.get::<_, i64>(0)?, serde_json::from_str::<Vec<String>>(&r.get::<_, String>(1)?).unwrap_or_default())))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(UserAi { enabled: enabled != 0, max_mode: AiMode::parse(&mode).unwrap_or(AiMode::Off), daily, monthly, grants })
        })
        .await
}

fn granted(ua: &UserAi, provider: i64, model: &str) -> bool {
    ua.grants.iter().any(|(p, ms)| *p == provider && ms.iter().any(|m| m == "*" || m == model))
}

pub async fn client_models(State(st): State<St>, User(s): User) -> AppResult<Json<AiModelsResp>> {
    let ua = user_ai(&st, s.user_id as i64).await?;
    let mut items = vec![];
    if ua.enabled {
        for mut p in load_providers(&st).await? {
            if p.models_updated_at == 0 && p.enabled {
                if let Ok(m) = refresh_models(&st, &p).await {
                    p.models = m;
                }
            }
            if !p.enabled {
                continue;
            }
            for m in &p.models {
                if m.enabled && granted(&ua, p.id, &m.id) {
                    items.push(AiModelInfo {
                        provider_id: p.id as u64,
                        provider_name: p.name.clone(),
                        id: m.id.clone(),
                        name: m.name.clone(),
                        vision: m.vision,
                        tools: m.tools,
                        context_length: m.context_length,
                    });
                }
            }
        }
    }
    Ok(Json(AiModelsResp { enabled: ua.enabled, max_mode: if ua.enabled { ua.max_mode } else { AiMode::Off }, items }))
}

fn sanitize_session(s: &str) -> String {
    let t: String = s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(64).collect();
    if t.is_empty() {
        "lemon-galaxy".into()
    } else {
        t
    }
}

async fn upstream(st: &AppState, p: &Provider, m: &ProviderModel, session: &str, body: &Value) -> reqwest::Result<reqwest::Response> {
    let mut req = st
        .http
        .post(p.chat_url(m))
        .bearer_auth(p.api_key.clone().unwrap_or_default())
        .header(header::USER_AGENT, user_agent())
        .header(header::CONTENT_TYPE, "application/json")
        .timeout(std::time::Duration::from_secs(600))
        .json(body);
    if p.kind == "opencode_go" {
        req = req.header("x-opencode-session", sanitize_session(session));
    }
    req.send().await
}

async fn tokens_since(st: &AppState, uid: i64, since: i64) -> i64 {
    st.db
        .call(move |c| {
            c.query_row(
                "SELECT COALESCE(SUM(prompt_tokens + completion_tokens), 0) FROM ai_usage WHERE user_id = ?1 AND at >= ?2",
                params![uid, since],
                |r| r.get(0),
            )
        })
        .await
        .unwrap_or(0)
}

fn day_start() -> i64 {
    let now = time::OffsetDateTime::now_utc();
    now.replace_time(time::Time::MIDNIGHT).unix_timestamp() * 1000
}

fn month_start() -> i64 {
    let now = time::OffsetDateTime::now_utc();
    now.replace_day(1).unwrap_or(now).replace_time(time::Time::MIDNIGHT).unix_timestamp() * 1000
}

struct UsageRecorder {
    st: St,
    user_id: i64,
    device: String,
    provider_id: i64,
    model: String,
    prompt: i64,
    completion: i64,
}

impl UsageRecorder {
    fn absorb(&mut self, v: &Value) {
        if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
            if let Some(p) = u.get("prompt_tokens").and_then(|x| x.as_i64()) {
                self.prompt = p;
            }
            if let Some(c) = u.get("completion_tokens").and_then(|x| x.as_i64()) {
                self.completion = c;
            }
        }
    }
}

impl Drop for UsageRecorder {
    fn drop(&mut self) {
        let (st, uid, dev, pid, model, p, c) = (self.st.clone(), self.user_id, self.device.clone(), self.provider_id, self.model.clone(), self.prompt, self.completion);
        tokio::spawn(async move {
            let now = galaxy_proto::now_ms();
            let _ = st
                .db
                .call(move |conn| {
                    conn.execute(
                        "INSERT INTO ai_usage(at, user_id, device_id, provider_id, model, prompt_tokens, completion_tokens) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                        params![now, uid, dev, pid, model, p, c],
                    )
                })
                .await;
            bump_stat(&st, "ai_tokens", p + c).await;
        });
    }
}

pub async fn client_chat(State(st): State<St>, User(s): User, body: Bytes) -> AppResult<Response> {
    let mut v: Value = serde_json::from_slice(&body).map_err(|e| AppError::bad_request(e.to_string()).lang(s.lang))?;
    let obj = v.as_object_mut().ok_or_else(|| AppError::bad_request("body").lang(s.lang))?;
    let provider_id = obj.remove("lg_provider_id").and_then(|x| x.as_i64()).ok_or_else(|| AppError::bad_request("lg_provider_id").lang(s.lang))?;
    let conversation = obj.remove("lg_conversation_id").and_then(|x| x.as_str().map(|s| s.to_string())).unwrap_or_default();
    let devices = obj.remove("lg_device_ids").map(|x| x.to_string()).unwrap_or_default();
    let model_id = obj.get("model").and_then(|x| x.as_str()).ok_or_else(|| AppError::bad_request("model").lang(s.lang))?.to_string();
    let stream = obj.get("stream").and_then(|x| x.as_bool()).unwrap_or(false);
    if stream {
        obj.insert("stream_options".into(), json!({ "include_usage": true }));
    }

    let uid = s.user_id as i64;
    let ua = user_ai(&st, uid).await?;
    if !ua.enabled {
        return Err(AppError::new(StatusCode::FORBIDDEN, "ai_disabled").lang(s.lang));
    }
    if !granted(&ua, provider_id, &model_id) {
        return Err(AppError::new(StatusCode::FORBIDDEN, "model_not_granted").lang(s.lang));
    }
    let p = load_provider(&st, provider_id).await.map_err(|e| e.lang(s.lang))?;
    let m = p.models.iter().find(|m| m.id == model_id && m.enabled).cloned().ok_or_else(|| AppError::new(StatusCode::FORBIDDEN, "model_not_granted").lang(s.lang))?;
    if !p.enabled {
        return Err(AppError::new(StatusCode::FORBIDDEN, "model_not_granted").lang(s.lang));
    }
    if ua.daily > 0 && tokens_since(&st, uid, day_start()).await >= ua.daily {
        return Err(AppError::new(StatusCode::TOO_MANY_REQUESTS, "quota_exceeded").lang(s.lang));
    }
    if ua.monthly > 0 && tokens_since(&st, uid, month_start()).await >= ua.monthly {
        return Err(AppError::new(StatusCode::TOO_MANY_REQUESTS, "quota_exceeded").lang(s.lang));
    }
    if !st.rate.check(&format!("ai:{uid}"), 120, 60) {
        return Err(AppError::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited").lang(s.lang));
    }

    let session = if conversation.is_empty() { format!("lg-{}", s.user_id) } else { conversation };
    let resp = upstream(&st, &p, &m, &session, &v)
        .await
        .map_err(|e| AppError::new(StatusCode::BAD_GATEWAY, "provider_error").detail(e.to_string()).lang(s.lang))?;
    let status = resp.status();
    let mut rec = UsageRecorder { st: st.clone(), user_id: uid, device: devices, provider_id, model: model_id, prompt: 0, completion: 0 };
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        rec.prompt = 0;
        return Err(AppError::new(StatusCode::BAD_GATEWAY, "provider_error").detail(format!("HTTP {status}: {}", text.chars().take(500).collect::<String>())).lang(s.lang));
    }
    if !stream {
        let bytes = resp.bytes().await.map_err(|e| AppError::new(StatusCode::BAD_GATEWAY, "provider_error").detail(e.to_string()))?;
        if let Ok(j) = serde_json::from_slice::<Value>(&bytes) {
            rec.absorb(&j);
        }
        return Ok(([(header::CONTENT_TYPE, "application/json")], bytes).into_response());
    }

    let rec = Arc::new(parking_lot::Mutex::new(rec));
    let rec2 = rec.clone();
    let mut line_buf = String::new();
    let upstream_stream = resp.bytes_stream().map(move |chunk| {
        if let Ok(b) = &chunk {
            line_buf.push_str(&String::from_utf8_lossy(b));
            while let Some(pos) = line_buf.find('\n') {
                let line: String = line_buf.drain(..=pos).collect();
                if let Some(data) = line.trim().strip_prefix("data:") {
                    let data = data.trim();
                    if data != "[DONE]" && data.contains("\"usage\"") {
                        if let Ok(j) = serde_json::from_str::<Value>(data) {
                            rec2.lock().absorb(&j);
                        }
                    }
                }
            }
            if line_buf.len() > 1 << 20 {
                line_buf.clear();
            }
        }
        chunk.map_err(std::io::Error::other)
    });
    // 流结束（或客户端断开）时 rec 被释放，记一次用量
    let guarded = GuardedStream { inner: Box::pin(upstream_stream), _rec: rec };
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("x-accel-buffering", "no")
        .body(Body::from_stream(guarded))
        .unwrap())
}

struct GuardedStream<S> {
    inner: std::pin::Pin<Box<S>>,
    _rec: Arc<parking_lot::Mutex<UsageRecorder>>,
}

impl<S: futures::Stream> futures::Stream for GuardedStream<S> {
    type Item = S::Item;
    fn poll_next(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

#[allow(dead_code)]
pub fn unused(_q: PageQ) {}
