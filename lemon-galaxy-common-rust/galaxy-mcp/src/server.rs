//! 本机 MCP 服务（Streamable HTTP，只听 127.0.0.1）。
//!
//! 同一个端点同时服务两代协议（docs/mcp.md）：
//! - 2026-07-28（无状态）：每个请求的 `params._meta` 带协议版本、客户端信息与能力，HTTP 头带
//!   `MCP-Protocol-Version` / `Mcp-Method` / `Mcp-Name`；`server/discover` 取服务信息；结果都带 `resultType`。
//! - 2025-11-25 及更早（有会话）：先 `initialize` 协商版本，服务端发 `Mcp-Session-Id`，之后的请求带上它。
//!
//! 需要用户批准的工具调用一直挂着，直到用户在 LemonGalaxy 界面里作答（批准不交给外部客户端，不用 MRTR）；
//! 客户端要 SSE 时每隔一段时间发心跳（有 progressToken 时发进度通知），客户端断开即取消这次调用。

// 校验失败直接回一个完整的 HTTP 响应（axum 的 Response 比较大，属正常用法）
#![allow(clippy::result_large_err)]

use crate::tools;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;
use base64::Engine;
use galaxy_agent::agent::{Agent, ExternalResult};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;
use tokio::sync::oneshot;

/// 无状态新版
pub const MODERN_VERSION: &str = "2026-07-28";
/// 有会话的旧版，按新到旧
pub const LEGACY_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CLIENT: &str = "io.modelcontextprotocol/clientInfo";
const META_SERVER: &str = "io.modelcontextprotocol/serverInfo";

/// 一个外部客户端（按客户端名归到同一个智能体：确认过的设备、应用在它的后续调用里继续有效）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientIdent {
    /// 归类用的键（客户端名规整成小写字母数字与 -）
    pub key: String,
    /// 显示名
    pub name: String,
    pub version: String,
}

impl ClientIdent {
    pub fn from_info(info: &Value) -> Self {
        let name = info["title"].as_str().or(info["name"].as_str()).unwrap_or("").trim().to_string();
        let raw = info["name"].as_str().unwrap_or(&name).trim().to_lowercase();
        let mut key: String = raw.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        key = key.trim_matches('-').chars().take(48).collect();
        if key.is_empty() {
            key = "mcp".into();
        }
        Self { key, name: if name.is_empty() { "MCP 客户端".into() } else { name.chars().take(64).collect() }, version: info["version"].as_str().unwrap_or("").chars().take(32).collect() }
    }

    fn anonymous() -> Self {
        Self::from_info(&Value::Null)
    }
}

/// 由桌面客户端实现：给外部客户端提供智能体（已登录的账号、设备目录、按需连接、确认卡片）。
pub trait Host: Send + Sync + 'static {
    /// 同一个 `client.key` 要返回同一个智能体；还没登录等情况返回错误（作为工具错误回给客户端）。
    fn agent_for(&self, client: &ClientIdent) -> anyhow::Result<Arc<Agent>>;
    /// 有请求进来（界面显示「最近活动」）。
    fn touched(&self, _client: &ClientIdent) {}
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// 0 表示随机端口
    pub port: u16,
    pub token: String,
    pub server_version: String,
    /// SSE 心跳间隔
    pub keepalive: Duration,
}

impl ServerConfig {
    pub fn new(port: u16, token: String, server_version: String) -> Self {
        Self { port, token, server_version, keepalive: Duration::from_secs(15) }
    }
}

struct Session {
    ident: ClientIdent,
    last: Instant,
}

struct Shared {
    cfg: ServerConfig,
    host: Arc<dyn Host>,
    sessions: Mutex<HashMap<String, Session>>,
}

/// 正在运行的服务；丢弃即停止。
pub struct McpServer {
    addr: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
}

impl McpServer {
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.addr.port())
    }

    pub fn stop(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 在 127.0.0.1 上启动服务（要在 tokio 运行时里调用）。
pub async fn start(cfg: ServerConfig, host: Arc<dyn Host>) -> anyhow::Result<McpServer> {
    anyhow::ensure!(cfg.token.len() >= 16, "令牌太短");
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", cfg.port)).await.map_err(|e| anyhow::anyhow!("无法监听 127.0.0.1:{}：{e}", cfg.port))?;
    let addr = listener.local_addr()?;
    let shared = Arc::new(Shared { cfg, host, sessions: Mutex::new(HashMap::new()) });
    let app = Router::new().route("/mcp", post(post_mcp).get(get_mcp).delete(delete_mcp)).with_state(shared);
    let (tx, rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let r = axum::serve(listener, app).with_graceful_shutdown(async move {
            let _ = rx.await;
        });
        if let Err(e) = r.await {
            tracing::warn!("MCP 服务出错：{e}");
        }
    });
    tracing::info!("MCP 服务已启动：http://{addr}/mcp");
    Ok(McpServer { addr, stop: Some(tx) })
}

// ---------------- 校验 ----------------

fn is_local_host(h: &str) -> bool {
    let h = h.trim().to_ascii_lowercase();
    // 去掉端口：[::1]:8080、127.0.0.1:8080、localhost:8080
    let host = if let Some(rest) = h.strip_prefix('[') { rest.split(']').next().unwrap_or("").to_string() } else { h.rsplit_once(':').map(|(a, _)| a.to_string()).unwrap_or(h.clone()) };
    matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1")
}

/// 防 DNS 重绑定：Host 必须是本机名；带 Origin 的（浏览器发起）也只能来自本机页面。
fn check_origin(headers: &HeaderMap) -> Result<(), Response> {
    let host_ok = headers.get(header::HOST).and_then(|v| v.to_str().ok()).map(is_local_host).unwrap_or(false);
    let origin_ok = match headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        None | Some("null") => true,
        Some(o) => o.split_once("://").map(|(_, rest)| is_local_host(rest.split('/').next().unwrap_or(""))).unwrap_or(false),
    };
    if host_ok && origin_ok {
        Ok(())
    } else {
        Err((StatusCode::FORBIDDEN, axum::Json(rpc_error(Value::Null, -32600, "只接受本机请求（Host / Origin 不是本机）", None))).into_response())
    }
}

fn check_auth(headers: &HeaderMap, token: &str) -> Result<(), Response> {
    let given = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            let (scheme, rest) = v.trim().split_once(' ')?;
            scheme.eq_ignore_ascii_case("bearer").then(|| rest.trim())
        })
        .unwrap_or("");
    let ok = given.len() == token.len() && bool::from(given.as_bytes().ct_eq(token.as_bytes()));
    if ok {
        Ok(())
    } else {
        Err((StatusCode::UNAUTHORIZED, axum::Json(rpc_error(Value::Null, -32600, "令牌不对：请在 LemonGalaxy 的 MCP 页复制连接配置", None))).into_response())
    }
}

fn rpc_error(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut e = json!({ "code": code, "message": message });
    if let Some(d) = data {
        e["data"] = d;
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": e })
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok()).map(|s| s.trim())
}

// ---------------- 处理 ----------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Era {
    Modern,
    Legacy,
}

async fn get_mcp(State(sh): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if let Err(r) = check_origin(&headers).and_then(|_| check_auth(&headers, &sh.cfg.token)) {
        return r;
    }
    // 不提供服务端主动推送的流
    (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST, DELETE")]).into_response()
}

async fn delete_mcp(State(sh): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if let Err(r) = check_origin(&headers).and_then(|_| check_auth(&headers, &sh.cfg.token)) {
        return r;
    }
    match header_str(&headers, "mcp-session-id") {
        Some(sid) if sh.sessions.lock().remove(sid).is_some() => StatusCode::OK.into_response(),
        Some(_) => StatusCode::NOT_FOUND.into_response(),
        None => (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST")]).into_response(),
    }
}

async fn post_mcp(State(sh): State<Arc<Shared>>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(r) = check_origin(&headers).and_then(|_| check_auth(&headers, &sh.cfg.token)) {
        return r;
    }
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, axum::Json(rpc_error(Value::Null, -32700, &format!("不是合法的 JSON：{e}"), None))).into_response(),
    };
    if !msg.is_object() {
        return (StatusCode::BAD_REQUEST, axum::Json(rpc_error(Value::Null, -32600, "一次只接受一条 JSON-RPC 消息", None))).into_response();
    }
    let method = msg["method"].as_str().unwrap_or("").to_string();
    let id = msg.get("id").cloned().filter(|v| !v.is_null());
    let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
    if method.is_empty() {
        // 客户端发来的响应：本服务不向客户端发请求，忽略
        return StatusCode::ACCEPTED.into_response();
    }

    // 判定哪一代协议、哪个客户端
    let modern_version = params["_meta"][META_VERSION].as_str().map(String::from);
    let (era, ident) = if method == "initialize" {
        (Era::Legacy, ClientIdent::from_info(&params["clientInfo"]))
    } else if let Some(v) = &modern_version {
        let rid = id.clone().unwrap_or(Value::Null);
        if v != MODERN_VERSION {
            let mut supported = vec![MODERN_VERSION.to_string()];
            supported.extend(LEGACY_VERSIONS.iter().map(|s| s.to_string()));
            return (StatusCode::BAD_REQUEST, axum::Json(rpc_error(rid, -32022, "Unsupported protocol version", Some(json!({ "supported": supported, "requested": v }))))).into_response();
        }
        let mismatch = |h: &str, want: &str| header_str(&headers, h).map(|x| x != want).unwrap_or(false);
        let name = params["name"].as_str().unwrap_or("");
        if mismatch("mcp-protocol-version", v) || mismatch("mcp-method", &method) || (method == "tools/call" && mismatch("mcp-name", name)) {
            return (StatusCode::BAD_REQUEST, axum::Json(rpc_error(rid, -32020, "Header mismatch: MCP-Protocol-Version / Mcp-Method / Mcp-Name must match the request body", None))).into_response();
        }
        (Era::Modern, ClientIdent::from_info(&params["_meta"][META_CLIENT]))
    } else {
        match header_str(&headers, "mcp-session-id") {
            Some(sid) => {
                let mut s = sh.sessions.lock();
                match s.get_mut(sid) {
                    Some(sess) => {
                        sess.last = Instant::now();
                        (Era::Legacy, sess.ident.clone())
                    }
                    None => {
                        drop(s);
                        let rid = id.clone().unwrap_or(Value::Null);
                        return (StatusCode::NOT_FOUND, axum::Json(rpc_error(rid, -32001, "Session not found: initialize again", None))).into_response();
                    }
                }
            }
            // 没带会话号的旧客户端：宽松处理，归到匿名客户端
            None => (Era::Legacy, ClientIdent::anonymous()),
        }
    };

    // 通知：一律 202
    let Some(id) = id else {
        return StatusCode::ACCEPTED.into_response();
    };

    let server_info = json!({ "name": "lemongalaxy", "title": "LemonGalaxy", "version": sh.cfg.server_version });
    let finish = move |mut result: Value| -> Value {
        if era == Era::Modern {
            result["resultType"] = json!("complete");
            result["_meta"] = json!({ META_SERVER: server_info.clone() });
        }
        json!({ "jsonrpc": "2.0", "id": id.clone(), "result": result })
    };
    let rid = msg.get("id").cloned().unwrap_or(Value::Null);

    match (era, method.as_str()) {
        (Era::Legacy, "initialize") => {
            let requested = params["protocolVersion"].as_str().unwrap_or("");
            let version = LEGACY_VERSIONS.iter().find(|v| **v == requested).copied().unwrap_or(LEGACY_VERSIONS[0]).to_string();
            let sid = uuid::Uuid::new_v4().simple().to_string();
            {
                let mut s = sh.sessions.lock();
                // 只留最近的 512 个会话
                if s.len() >= 512 {
                    if let Some(old) = s.iter().min_by_key(|(_, v)| v.last).map(|(k, _)| k.clone()) {
                        s.remove(&old);
                    }
                }
                s.insert(sid.clone(), Session { ident: ident.clone(), last: Instant::now() });
            }
            sh.host.touched(&ident);
            let result = json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "lemongalaxy", "title": "LemonGalaxy", "version": sh.cfg.server_version },
                "instructions": tools::INSTRUCTIONS,
            });
            let mut r = axum::Json(finish(result)).into_response();
            if let Ok(v) = HeaderValue::from_str(&sid) {
                r.headers_mut().insert("mcp-session-id", v);
            }
            r
        }
        (Era::Legacy, "ping") => axum::Json(finish(json!({}))).into_response(),
        (Era::Modern, "server/discover") => {
            let mut versions = vec![MODERN_VERSION.to_string()];
            versions.extend(LEGACY_VERSIONS.iter().map(|s| s.to_string()));
            let result = json!({
                "supportedVersions": versions,
                "capabilities": { "tools": {} },
                "instructions": tools::INSTRUCTIONS,
                "ttlMs": 3_600_000,
                "cacheScope": "private",
            });
            axum::Json(finish(result)).into_response()
        }
        (_, "tools/list") => {
            let mut result = json!({ "tools": tools::definitions() });
            if era == Era::Modern {
                result["ttlMs"] = json!(3_600_000);
                result["cacheScope"] = json!("private");
            }
            axum::Json(finish(result)).into_response()
        }
        (_, "tools/call") => {
            let name = params["name"].as_str().unwrap_or("").to_string();
            if !tools::exists(&name) {
                return axum::Json(rpc_error(rid, -32602, &format!("Unknown tool: {name}"), None)).into_response();
            }
            let args = params.get("arguments").cloned().filter(|a| a.is_object()).unwrap_or_else(|| json!({}));
            sh.host.touched(&ident);
            let agent = sh.host.agent_for(&ident);
            tracing::info!("MCP 调用：{} {name}", ident.key);
            let call: Pin<Box<dyn Future<Output = Value> + Send>> = Box::pin(async move {
                let r = match agent {
                    Ok(a) => a.call_external(&name, args).await,
                    Err(e) => ExternalResult { text: e.to_string(), image: None, is_error: true },
                };
                finish(tool_result(r))
            });
            let wants_sse = header_str(&headers, "accept").map(|a| a.contains("text/event-stream")).unwrap_or(false);
            if wants_sse {
                let token = params["_meta"]["progressToken"].clone();
                sse_response(call, sh.cfg.keepalive, token).into_response()
            } else {
                axum::Json(call.await).into_response()
            }
        }
        (Era::Modern, _) => (StatusCode::NOT_FOUND, axum::Json(rpc_error(rid, -32601, &format!("Method not found: {method}"), None))).into_response(),
        (Era::Legacy, _) => axum::Json(rpc_error(rid, -32601, &format!("Method not found: {method}"), None)).into_response(),
    }
}

/// 工具结果 → MCP CallToolResult
fn tool_result(r: ExternalResult) -> Value {
    let mut content = vec![json!({ "type": "text", "text": r.text })];
    if let Some(img) = r.image {
        content.push(json!({ "type": "image", "data": base64::engine::general_purpose::STANDARD.encode(&img.data), "mimeType": img.mime }));
    }
    json!({ "content": content, "isError": r.is_error })
}

/// 用 SSE 回 tools/call：工具的 future 就在流里，客户端断开（流被丢弃）即取消；
/// 等待期间按间隔发心跳注释，客户端给了 progressToken 时改发进度通知。
fn sse_response(call: Pin<Box<dyn Future<Output = Value> + Send>>, every: Duration, token: Value) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    struct St {
        call: Pin<Box<dyn Future<Output = Value> + Send>>,
        tick: tokio::time::Interval,
        n: u64,
        token: Value,
    }
    let tick = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
    let stream = futures::stream::unfold(Some(St { call, tick, n: 0, token }), |st| async move {
        let mut st = st?;
        tokio::select! {
            out = &mut st.call => Some((Ok(Event::default().event("message").data(out.to_string())), None)),
            _ = st.tick.tick() => {
                st.n += 1;
                let ev = if st.token.is_null() {
                    Event::default().comment("keepalive")
                } else {
                    let note = json!({ "jsonrpc": "2.0", "method": "notifications/progress", "params": {
                        "progressToken": st.token, "progress": st.n,
                        "message": "still running (it may be waiting for the user's approval in LemonGalaxy)"
                    } });
                    Event::default().event("message").data(note.to_string())
                };
                Some((Ok(ev), Some(st)))
            }
        }
    });
    Sse::new(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_hosts() {
        for h in ["127.0.0.1:4000", "localhost:1", "[::1]:9", "LOCALHOST", "127.0.0.1"] {
            assert!(is_local_host(h), "{h}");
        }
        for h in ["evil.com:4000", "127.0.0.1.evil.com", "192.168.1.2:80", ""] {
            assert!(!is_local_host(h), "{h}");
        }
    }

    #[test]
    fn client_keys() {
        let c = ClientIdent::from_info(&json!({ "name": "claude-code", "version": "2.1.0" }));
        assert_eq!((c.key.as_str(), c.name.as_str()), ("claude-code", "claude-code"));
        let c = ClientIdent::from_info(&json!({ "name": "Codex CLI", "title": "Codex" }));
        assert_eq!((c.key.as_str(), c.name.as_str()), ("codex-cli", "Codex"));
        assert_eq!(ClientIdent::from_info(&Value::Null).key, "mcp");
    }
}
