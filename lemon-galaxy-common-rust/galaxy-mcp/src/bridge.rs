//! stdio 桥接：给只会用 stdio 启动 MCP 服务的客户端（如 Claude Desktop）用。
//!
//! 客户端启动 `LemonGalaxy --mcp-stdio`，本进程逐行读标准输入的 JSON-RPC 消息，转发给正在运行的
//! LemonGalaxy 的本机 HTTP 端点（地址和令牌从数据目录里的端点文件读，每次现读，令牌换了也跟得上），
//! 把响应（JSON 或 SSE 里的每条消息）逐行写回标准输出。旧版协议记住会话号与协商版本，新版补齐要求的 HTTP 头。
//! 客户端发 notifications/cancelled 时中止对应的 HTTP 请求（服务端视为取消）。

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

/// 端点文件：桌面客户端开启 MCP 服务时写进数据目录（只有本机用户可读）。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Endpoint {
    pub url: String,
    pub token: String,
    #[serde(default)]
    pub pid: u32,
}

pub fn write_endpoint(path: &Path, ep: &Endpoint) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let data = serde_json::to_vec_pretty(ep).map_err(std::io::Error::other)?;
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(&data)?;
    }
    #[cfg(not(unix))]
    std::fs::write(&tmp, &data)?;
    std::fs::rename(&tmp, path)
}

pub fn read_endpoint(path: &Path) -> anyhow::Result<Endpoint> {
    let b = std::fs::read(path).map_err(|_| anyhow::anyhow!("LemonGalaxy 的 MCP 服务没有开启：请打开 LemonGalaxy，在「MCP」页打开服务"))?;
    Ok(serde_json::from_slice(&b)?)
}

pub fn remove_endpoint(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[derive(Default)]
struct LegacyState {
    session: Option<String>,
    version: Option<String>,
}

/// 运行桥接直到输入结束（`input` / `output` 一般是标准输入输出）。
pub async fn run_stdio_bridge<R, W>(endpoint_path: PathBuf, input: R, output: W) -> anyhow::Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    // 本机地址绝不走代理（系统或环境变量里的代理会把 127.0.0.1 也转走）
    let http = reqwest::Client::builder().no_proxy().connect_timeout(std::time::Duration::from_secs(5)).build()?;
    let out = Arc::new(Mutex::new(output));
    let legacy = Arc::new(parking_lot::Mutex::new(LegacyState::default()));
    let running: Arc<parking_lot::Mutex<HashMap<String, tokio::task::AbortHandle>>> = Arc::default();
    let mut lines = BufReader::new(input).lines();
    let mut tasks: Vec<JoinHandle<()>> = vec![];
    while let Some(line) = lines.next_line().await? {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                write_line(&out, &json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("不是合法的 JSON：{e}") } })).await;
                continue;
            }
        };
        let method = msg["method"].as_str().unwrap_or("").to_string();
        if method.is_empty() {
            // 客户端回给服务端的响应：本服务不发请求，忽略
            continue;
        }
        if method == "notifications/cancelled" {
            let rid = id_key(&msg["params"]["requestId"]);
            if let Some(h) = running.lock().remove(&rid) {
                h.abort();
            }
            continue;
        }
        let id = msg.get("id").cloned().filter(|v| !v.is_null());
        let (http, out2, legacy2, path, running2) = (http.clone(), out.clone(), legacy.clone(), endpoint_path.clone(), running.clone());
        let key = id.as_ref().map(id_key);
        let h = tokio::spawn(async move {
            if let Err(e) = forward(&http, &path, &msg, &legacy2, &out2).await {
                if let Some(id) = msg.get("id").cloned().filter(|v| !v.is_null()) {
                    write_line(&out2, &json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": format!("连不上 LemonGalaxy：{e}") } })).await;
                }
            }
            if let Some(k) = msg.get("id").filter(|v| !v.is_null()).map(id_key) {
                running2.lock().remove(&k);
            }
        });
        if let Some(k) = key {
            running.lock().insert(k.clone(), h.abort_handle());
            // 已经结束的任务（来不及登记就跑完了）不留在表里
            if h.is_finished() {
                running.lock().remove(&k);
            }
        }
        tasks.retain(|t| !t.is_finished());
        tasks.push(h);
    }
    // 输入结束：等正在转发的请求写完
    for t in tasks {
        let _ = t.await;
    }
    running.lock().clear();
    Ok(())
}

fn id_key(v: &Value) -> String {
    v.to_string()
}

async fn write_line<W: AsyncWrite + Unpin>(out: &Mutex<W>, v: &Value) {
    let mut s = v.to_string();
    s.push('\n');
    let mut w = out.lock().await;
    let _ = w.write_all(s.as_bytes()).await;
    let _ = w.flush().await;
}

async fn forward<W: AsyncWrite + Unpin>(http: &reqwest::Client, path: &Path, msg: &Value, legacy: &parking_lot::Mutex<LegacyState>, out: &Mutex<W>) -> anyhow::Result<()> {
    let ep = read_endpoint(path)?;
    let method = msg["method"].as_str().unwrap_or("");
    let mut req = http
        .post(&ep.url)
        .bearer_auth(&ep.token)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(msg.to_string());
    if let Some(v) = msg["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"].as_str() {
        req = req.header("mcp-protocol-version", v).header("mcp-method", method);
        if method == "tools/call" {
            if let Some(n) = msg["params"]["name"].as_str() {
                req = req.header("mcp-name", n);
            }
        }
    } else if method != "initialize" {
        let st = legacy.lock();
        if let Some(s) = &st.session {
            req = req.header("mcp-session-id", s.as_str());
        }
        if let Some(v) = &st.version {
            req = req.header("mcp-protocol-version", v.as_str());
        }
    }
    let resp = req.send().await?;
    if method == "initialize" {
        if let Some(s) = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            legacy.lock().session = Some(s.to_string());
        }
    }
    if resp.status() == reqwest::StatusCode::ACCEPTED {
        return Ok(());
    }
    let status = resp.status();
    let is_sse = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).map(|c| c.starts_with("text/event-stream")).unwrap_or(false);
    if is_sse {
        let mut body = resp.bytes_stream();
        let mut buf: Vec<u8> = vec![];
        let mut data = String::new();
        while let Some(chunk) = body.next().await {
            buf.extend_from_slice(&chunk?);
            while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                let line = String::from_utf8_lossy(&line).trim_end_matches(['\r', '\n']).to_string();
                if line.is_empty() {
                    if !data.is_empty() {
                        if let Ok(v) = serde_json::from_str::<Value>(&data) {
                            write_line(out, &v).await;
                        }
                        data.clear();
                    }
                } else if let Some(d) = line.strip_prefix("data:") {
                    if !data.is_empty() {
                        data.push('\n');
                    }
                    data.push_str(d.strip_prefix(' ').unwrap_or(d));
                }
            }
        }
        if !data.is_empty() {
            if let Ok(v) = serde_json::from_str::<Value>(&data) {
                write_line(out, &v).await;
            }
        }
        return Ok(());
    }
    let bytes = resp.bytes().await?;
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(v) if v.get("jsonrpc").is_some() => {
            if method == "initialize" {
                if let Some(ver) = v["result"]["protocolVersion"].as_str() {
                    legacy.lock().version = Some(ver.to_string());
                }
            }
            if msg.get("id").map(|i| !i.is_null()).unwrap_or(false) {
                write_line(out, &v).await;
            }
            Ok(())
        }
        _ => anyhow::bail!("HTTP {status}：{}", String::from_utf8_lossy(&bytes).chars().take(200).collect::<String>()),
    }
}
