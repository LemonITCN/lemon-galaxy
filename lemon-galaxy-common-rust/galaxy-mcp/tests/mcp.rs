//! 端到端：起真的本机 HTTP 服务（假执行器 + 假账号目录），按两代协议发请求，验证鉴权、批准、SSE、取消与 stdio 桥接。

use bytes::Bytes;
use futures::stream::BoxStream;
use galaxy_agent::agent::*;
use galaxy_mcp::{ClientIdent, Host, ServerConfig};
use galaxy_proto::peer::{DisplayInfo, FileEntry, ImageData, InputEvent, Origin};
use galaxy_proto::{AiMode, Platform};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, watch};

const TOKEN: &str = "test-token-0123456789abcdef";

struct NoLlm;
impl Llm for NoLlm {
    fn stream(&self, _body: Value) -> BoxFut<'_, anyhow::Result<BoxStream<'static, anyhow::Result<Bytes>>>> {
        Box::pin(async { anyhow::bail!("外部调用不走模型") })
    }
}

#[derive(Default)]
struct FakeExec {
    connected: Mutex<Vec<String>>,
    ran: Mutex<Vec<String>>,
}

impl Executor for FakeExec {
    fn targets(&self) -> Vec<TargetInfo> {
        self.connected
            .lock()
            .iter()
            .map(|id| TargetInfo { id: id.clone(), name: format!("名-{id}"), kind: TargetKind::Device, platform: Platform::Linux, os: "Linux".into(), user: "u".into(), shell: "sh".into(), gui: true, ai_cap: AiMode::FullTrust, computer_use: false })
            .collect()
    }
    fn run_command(&self, _t: &str, cmd: &str, _cwd: Option<String>, _to: u64, _sh: Option<String>, _o: Origin, _on: Box<dyn Fn(&str) + Send + Sync>, _c: watch::Receiver<bool>) -> BoxFut<'_, anyhow::Result<CmdResult>> {
        self.ran.lock().push(cmd.to_string());
        let out = format!("ran: {cmd}");
        Box::pin(async move { Ok(CmdResult { code: Some(0), stdout: out, ..Default::default() }) })
    }
    fn read_file(&self, _: &str, path: &str, _: u64) -> BoxFut<'_, anyhow::Result<Vec<u8>>> {
        let p = path.to_string();
        Box::pin(async move { Ok(format!("content of {p}").into_bytes()) })
    }
    fn exists(&self, _: &str, _: &str) -> BoxFut<'_, anyhow::Result<bool>> {
        Box::pin(async { Ok(false) })
    }
    fn write_file(&self, _: &str, _: &str, _: Vec<u8>, _: Option<Vec<u8>>, _: Origin) -> BoxFut<'_, anyhow::Result<Option<String>>> {
        Box::pin(async { Ok(None) })
    }
    fn list_dir(&self, _: &str, _: &str) -> BoxFut<'_, anyhow::Result<Vec<FileEntry>>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn search(&self, _: &str, _: &str, _: &str, _: bool) -> BoxFut<'_, anyhow::Result<Vec<FileEntry>>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn displays(&self, _: &str) -> BoxFut<'_, anyhow::Result<Vec<DisplayInfo>>> {
        Box::pin(async { Ok(vec![DisplayInfo { id: 0, name: "主屏".into(), width: 1920, height: 1080, scale: 1.0, primary: true, is_virtual: false }]) })
    }
    fn screenshot(&self, _: &str, _: u32, _: u32, _: u32) -> BoxFut<'_, anyhow::Result<ImageData>> {
        Box::pin(async { Ok(ImageData { mime: "image/png".into(), data: vec![1, 2, 3], width: 192, height: 108 }) })
    }
    fn ui_tree(&self, _: &str, _: u32, _: u32) -> BoxFut<'_, anyhow::Result<String>> {
        Box::pin(async { Ok(String::new()) })
    }
    fn inject(&self, _: &str, _: u32, _: Vec<InputEvent>, _: Origin) -> BoxFut<'_, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn terminal_open(&self, _: &str, _: Origin) -> BoxFut<'_, anyhow::Result<String>> {
        Box::pin(async { Ok("t1".to_string()) })
    }
    fn terminal_send(&self, _: &str, _: Vec<u8>) -> BoxFut<'_, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn terminal_read(&self, _: &str) -> BoxFut<'_, anyhow::Result<String>> {
        Box::pin(async { Ok(String::new()) })
    }
    fn terminal_close(&self, _: &str) -> BoxFut<'_, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
}

struct FakeWs {
    exec: Arc<FakeExec>,
}

impl Workspace for FakeWs {
    fn catalog(&self) -> BoxFut<'_, anyhow::Result<Vec<CatalogItem>>> {
        Box::pin(async { Ok(vec![CatalogItem { id: "dev1".into(), name: "Ubuntu".into(), kind: TargetKind::Device, platform: Platform::Linux, online: true, detail: String::new() }]) })
    }
    fn query(&self, _kind: &str, _limit: u32) -> BoxFut<'_, anyhow::Result<String>> {
        Box::pin(async { Ok(String::new()) })
    }
    fn connect(&self, target: &str) -> BoxFut<'_, anyhow::Result<()>> {
        let t = target.to_string();
        Box::pin(async move {
            let mut c = self.exec.connected.lock();
            if !c.contains(&t) {
                c.push(t);
            }
            Ok(())
        })
    }
    fn wake(&self, target: &str) -> BoxFut<'_, anyhow::Result<String>> {
        let t = target.to_string();
        Box::pin(async move { Ok(format!("已经由 dev1 给 {t} 发了唤醒包")) })
    }
}

/// 需要批准的步骤送到测试里（客户端键, 调用号, 工具名）；所有事件也记下来。
struct Ui {
    key: String,
    approvals: mpsc::UnboundedSender<(String, String, String)>,
    events: Arc<Mutex<Vec<AgentEvent>>>,
}

impl AgentUi for Ui {
    fn event(&self, e: AgentEvent) {
        if let AgentEvent::ToolProposed { call, needs_approval: true } = &e {
            let _ = self.approvals.send((self.key.clone(), call.call_id.clone(), call.tool.clone()));
        }
        self.events.lock().push(e);
    }
}

struct TestHost {
    agents: Mutex<HashMap<String, Arc<Agent>>>,
    approvals: mpsc::UnboundedSender<(String, String, String)>,
    events: Arc<Mutex<Vec<AgentEvent>>>,
    execs: Mutex<HashMap<String, Arc<FakeExec>>>,
}

impl Host for TestHost {
    fn agent_for(&self, client: &ClientIdent) -> anyhow::Result<Arc<Agent>> {
        let mut a = self.agents.lock();
        if let Some(x) = a.get(&client.key) {
            return Ok(x.clone());
        }
        let exec = Arc::new(FakeExec::default());
        self.execs.lock().insert(client.key.clone(), exec.clone());
        let cfg = AgentConfig {
            conversation_id: format!("mcp-{}", client.key),
            provider_id: 0,
            model_id: String::new(),
            vision: true,
            context_length: 128_000,
            mode: AiMode::ConfirmDangerous,
            admin_cap: AiMode::FullTrust,
            plan_first: false,
            max_steps: 100,
            locale: "zh-CN".into(),
            trust_minutes: None,
        };
        let ui = Arc::new(Ui { key: client.key.clone(), approvals: self.approvals.clone(), events: self.events.clone() });
        let agent = Agent::new(cfg, Arc::new(NoLlm), exec.clone(), Some(Arc::new(FakeWs { exec }) as Arc<dyn Workspace>), ui);
        a.insert(client.key.clone(), agent.clone());
        Ok(agent)
    }
}

struct Env {
    url: String,
    host: Arc<TestHost>,
    approvals: mpsc::UnboundedReceiver<(String, String, String)>,
    http: reqwest::Client,
    _server: galaxy_mcp::McpServer,
}

async fn env() -> Env {
    let (tx, rx) = mpsc::unbounded_channel();
    let host = Arc::new(TestHost { agents: Mutex::default(), approvals: tx, events: Arc::default(), execs: Mutex::default() });
    let mut cfg = ServerConfig::new(0, TOKEN.into(), "0.0.0-test".into());
    cfg.keepalive = Duration::from_millis(200);
    let server = galaxy_mcp::start(cfg, host.clone()).await.unwrap();
    Env { url: server.url(), host, approvals: rx, http: reqwest::Client::builder().no_proxy().build().unwrap(), _server: server }
}

fn modern(id: u64, method: &str, mut params: Value) -> Value {
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": { "name": "claude-code", "version": "9.9" },
        "io.modelcontextprotocol/clientCapabilities": {}
    });
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

impl Env {
    fn post(&self, body: &Value) -> reqwest::RequestBuilder {
        let mut r = self.http.post(&self.url).bearer_auth(TOKEN).header("accept", "application/json, text/event-stream").json(body);
        if let Some(v) = body["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"].as_str() {
            r = r.header("mcp-protocol-version", v).header("mcp-method", body["method"].as_str().unwrap());
            if let Some(n) = body["params"]["name"].as_str() {
                r = r.header("mcp-name", n);
            }
        }
        r
    }

    /// 发请求并取 JSON-RPC 响应（SSE 时取最后一条带 id 的消息）
    async fn rpc(&self, body: &Value) -> (u16, Value) {
        let resp = self.post(body).send().await.unwrap();
        let status = resp.status().as_u16();
        let sse = resp.headers().get("content-type").map(|v| v.to_str().unwrap().starts_with("text/event-stream")).unwrap_or(false);
        let text = resp.text().await.unwrap();
        if sse {
            let last = text.lines().filter_map(|l| l.strip_prefix("data:")).map(|d| serde_json::from_str::<Value>(d.trim()).unwrap()).rfind(|v| v.get("id").is_some()).unwrap();
            (status, last)
        } else {
            (status, serde_json::from_str(&text).unwrap_or(Value::Null))
        }
    }

    async fn next_approval(&mut self) -> (String, String, String) {
        tokio::time::timeout(Duration::from_secs(5), self.approvals.recv()).await.expect("等批准请求超时").unwrap()
    }

    fn decide(&self, key: &str, call: &str, d: ApprovalDecision) {
        self.host.agents.lock().get(key).unwrap().decide(call, d);
    }
}

fn text_of(v: &Value) -> String {
    v["result"]["content"].as_array().unwrap().iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n")
}

#[tokio::test]
async fn legacy_session_flow() {
    let e = env().await;
    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "cursor", "version": "1" } } });
    let resp = e.post(&init).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let sid = resp.headers().get("mcp-session-id").unwrap().to_str().unwrap().to_string();
    let v: Value = resp.json().await.unwrap();
    assert_eq!(v["result"]["protocolVersion"], "2025-06-18");
    assert!(v["result"]["instructions"].as_str().unwrap().contains("list_devices"));
    assert!(v["result"].get("resultType").is_none());

    let r = e.http.post(&e.url).bearer_auth(TOKEN).header("mcp-session-id", &sid).json(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).send().await.unwrap();
    assert_eq!(r.status(), 202);

    let r = e.http.post(&e.url).bearer_auth(TOKEN).header("mcp-session-id", &sid).json(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })).send().await.unwrap();
    let v: Value = r.json().await.unwrap();
    let names: Vec<&str> = v["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"run_command") && names.contains(&"screenshot") && names.contains(&"upload_file"));

    let r = e.http.post(&e.url).bearer_auth(TOKEN).header("mcp-session-id", &sid).json(&json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "list_devices", "arguments": {} } })).send().await.unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["result"]["isError"], false);
    assert!(text_of(&v).contains("id=dev1"), "{v}");
    // 会话归到 initialize 时的客户端名
    assert!(e.host.agents.lock().contains_key("cursor"));

    let r = e.http.post(&e.url).bearer_auth(TOKEN).header("mcp-session-id", "nope").json(&json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/list" })).send().await.unwrap();
    assert_eq!(r.status(), 404);
    let r = e.http.delete(&e.url).bearer_auth(TOKEN).header("mcp-session-id", &sid).send().await.unwrap();
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn modern_flow_with_device_approval() {
    let mut e = env().await;
    let (s, v) = e.rpc(&modern(1, "server/discover", json!({}))).await;
    assert_eq!(s, 200);
    assert_eq!(v["result"]["resultType"], "complete");
    assert!(v["result"]["supportedVersions"].as_array().unwrap().contains(&json!("2026-07-28")));
    assert_eq!(v["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], "lemongalaxy");

    // 第一次用 dev1 要用户同意；echo 是只读命令，不用再批
    let body = modern(2, "tools/call", json!({ "name": "run_command", "arguments": { "target": "dev1", "command": "echo hi", "reason": "测试" } }));
    let req = e.post(&body);
    let call = tokio::spawn(async move { req.send().await.unwrap().text().await.unwrap() });
    let (key, cid, tool) = e.next_approval().await;
    assert_eq!((key.as_str(), tool.as_str()), ("claude-code", "use_target"));
    e.decide(&key, &cid, ApprovalDecision::AllowOnce);
    let text = call.await.unwrap();
    assert!(text.contains("ran: echo hi"), "{text}");
    assert!(text.contains("\"isError\":false"), "{text}");

    // 同一个客户端再用 dev1 不再问
    let (_, v) = e.rpc(&modern(3, "tools/call", json!({ "name": "screenshot", "arguments": { "target": "dev1" } }))).await;
    let content = v["result"]["content"].as_array().unwrap();
    assert!(content.iter().any(|c| c["type"] == "image" && c["data"] == "AQID" && c["mimeType"] == "image/png"), "{v}");
    assert!(e.approvals.try_recv().is_err());
}

#[tokio::test]
async fn dangerous_command_denied_by_user() {
    let mut e = env().await;
    let body = modern(1, "tools/call", json!({ "name": "run_command", "arguments": { "target": "dev1", "command": "rm -rf /tmp/build" } }));
    let req = e.post(&body);
    let call = tokio::spawn(async move { req.send().await.unwrap().text().await.unwrap() });
    let (key, cid, _) = e.next_approval().await; // use_target
    e.decide(&key, &cid, ApprovalDecision::AllowOnce);
    let (key, cid, tool) = e.next_approval().await; // 危险命令
    assert_eq!(tool, "run_command");
    e.decide(&key, &cid, ApprovalDecision::Deny { reason: Some("先别删".into()) });
    let text = call.await.unwrap();
    assert!(text.contains("\"isError\":true") && text.contains("先别删"), "{text}");
    assert!(e.host.execs.lock()["claude-code"].ran.lock().is_empty());
}

#[tokio::test]
async fn sse_keepalive_and_cancel_on_disconnect() {
    let mut e = env().await;
    // 先把 dev1 授权掉
    let warm = modern(1, "tools/call", json!({ "name": "list_displays", "arguments": { "target": "dev1" } }));
    let req = e.post(&warm);
    let call = tokio::spawn(async move { req.send().await.unwrap().text().await.unwrap() });
    let (key, cid, _) = e.next_approval().await;
    e.decide(&key, &cid, ApprovalDecision::AllowOnce);
    assert!(call.await.unwrap().contains("display=0"));

    // 危险命令挂着等批准：SSE 有进度通知；客户端断开后挂着的确认被撤销
    let mut body = modern(2, "tools/call", json!({ "name": "run_command", "arguments": { "target": "dev1", "command": "reboot" } }));
    body["params"]["_meta"]["progressToken"] = json!("p1");
    let resp = e.post(&body).send().await.unwrap();
    assert!(resp.headers()["content-type"].to_str().unwrap().starts_with("text/event-stream"));
    let (_, cid, tool) = e.next_approval().await;
    assert_eq!(tool, "run_command");
    let mut stream = resp.bytes_stream();
    use futures::StreamExt;
    let first = tokio::time::timeout(Duration::from_secs(3), stream.next()).await.unwrap().unwrap().unwrap();
    assert!(String::from_utf8_lossy(&first).contains("notifications/progress"));
    drop(stream);
    let events = e.host.events.clone();
    let cancelled = async {
        loop {
            let done = events.lock().iter().any(|ev| matches!(ev, AgentEvent::ToolFinished { call_id, result } if *call_id == cid && result.summary.contains("取消")));
            if done {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(5), cancelled).await.expect("断开后应撤销挂着的确认");
    assert!(e.host.execs.lock()["claude-code"].ran.lock().iter().all(|c| c != "reboot"));
}

#[tokio::test]
async fn rejects_bad_requests() {
    let e = env().await;
    let body = modern(1, "tools/list", json!({}));
    // 没令牌、令牌错
    assert_eq!(e.http.post(&e.url).json(&body).send().await.unwrap().status(), 401);
    assert_eq!(e.http.post(&e.url).bearer_auth("test-token-0123456789abcdeX").json(&body).send().await.unwrap().status(), 401);
    // 外站页面、DNS 重绑定
    assert_eq!(e.post(&body).header("origin", "https://evil.example").send().await.unwrap().status(), 403);
    assert_eq!(e.post(&body).header("host", "evil.example:80").send().await.unwrap().status(), 403);
    assert_eq!(e.post(&body).header("origin", "http://localhost:3000").send().await.unwrap().status(), 200);
    // 不支持的版本、头与正文不一致、未知方法、未知工具
    let mut old = body.clone();
    old["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"] = json!("1900-01-01");
    let (s, v) = e.rpc(&old).await;
    assert_eq!((s, v["error"]["code"].as_i64()), (400, Some(-32022)));
    assert!(v["error"]["data"]["supported"].as_array().unwrap().contains(&json!("2025-11-25")));
    let r = e.http.post(&e.url).bearer_auth(TOKEN).header("mcp-protocol-version", "2026-07-28").header("mcp-method", "tools/call").json(&body).send().await.unwrap();
    assert_eq!(r.status(), 400);
    assert_eq!(r.json::<Value>().await.unwrap()["error"]["code"], -32020);
    let (s, v) = e.rpc(&modern(2, "resources/list", json!({}))).await;
    assert_eq!((s, v["error"]["code"].as_i64()), (404, Some(-32601)));
    let (s, v) = e.rpc(&modern(3, "tools/call", json!({ "name": "format_disk", "arguments": {} }))).await;
    assert_eq!((s, v["error"]["code"].as_i64()), (200, Some(-32602)));
    assert_eq!(e.http.get(&e.url).bearer_auth(TOKEN).send().await.unwrap().status(), 405);
}

#[tokio::test]
async fn stdio_bridge_round_trip() {
    let e = env().await;
    let dir = std::env::temp_dir().join(format!("lg-mcp-bridge-{}", std::process::id()));
    let path = dir.join("mcp-endpoint.json");
    galaxy_mcp::write_endpoint(&path, &galaxy_mcp::Endpoint { url: e.url.clone(), token: TOKEN.into(), pid: 1 }).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let (mut client_in, bridge_in) = tokio::io::duplex(64 * 1024);
    let (bridge_out, client_out) = tokio::io::duplex(64 * 1024);
    let bridge = tokio::spawn(galaxy_mcp::run_stdio_bridge(path.clone(), bridge_in, bridge_out));
    let mut out = BufReader::new(client_out).lines();
    let init = format!("{}\n", json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": { "name": "claude-desktop", "version": "1" } } }));
    client_in.write_all(init.as_bytes()).await.unwrap();
    let v: Value = serde_json::from_str(&out.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(v["result"]["protocolVersion"], "2025-11-25");
    client_in.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n").await.unwrap();
    client_in.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"list_devices\",\"arguments\":{}}}\n").await.unwrap();
    let v: Value = serde_json::from_str(&out.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(v["id"], 2);
    assert!(text_of(&v).contains("id=dev1"), "{v}");
    // 旧版会话号由桥接记住：请求归到 initialize 时的客户端
    assert!(e.host.agents.lock().contains_key("claude-desktop"));
    // 新版请求经桥接补齐 HTTP 头
    let m = modern(3, "tools/list", json!({}));
    client_in.write_all(format!("{m}\n").as_bytes()).await.unwrap();
    let v: Value = serde_json::from_str(&out.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!((v["id"].as_i64(), v["result"]["resultType"].as_str()), (Some(3), Some("complete")));
    // 服务没开：回 JSON-RPC 错误而不是卡住
    galaxy_mcp::remove_endpoint(&path);
    client_in.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/list\"}\n").await.unwrap();
    let v: Value = serde_json::from_str(&out.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(v["id"], 4);
    assert!(v["error"]["message"].as_str().unwrap().contains("MCP"));
    drop(client_in);
    tokio::time::timeout(Duration::from_secs(5), bridge).await.unwrap().unwrap().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wake_and_file_transfer_tools() {
    let mut e = env().await;
    let (_, v) = e.rpc(&modern(1, "tools/call", json!({ "name": "wake_device", "arguments": { "target": "dev1" } }))).await;
    assert!(text_of(&v).contains("唤醒包"), "{v}");
    // 下载到本机：相对路径不收
    let body = modern(2, "tools/call", json!({ "name": "download_file", "arguments": { "target": "dev1", "path": "/etc/hostname", "local_path": "relative.txt" } }));
    let req = e.post(&body);
    let call = tokio::spawn(async move { req.send().await.unwrap().text().await.unwrap() });
    let (key, cid, _) = e.next_approval().await;
    e.decide(&key, &cid, ApprovalDecision::AllowOnce);
    let text = call.await.unwrap();
    assert!(text.contains("\"isError\":true") && text.contains("绝对路径"), "{text}");
    let local = std::env::temp_dir().join(format!("lg-mcp-dl-{}/sub/hostname", std::process::id()));
    let (_, v) = e.rpc(&modern(3, "tools/call", json!({ "name": "download_file", "arguments": { "target": "dev1", "path": "/etc/hostname", "local_path": local.to_string_lossy() } }))).await;
    assert_eq!(v["result"]["isError"], false, "{v}");
    assert_eq!(std::fs::read_to_string(&local).unwrap(), "content of /etc/hostname");
    let _ = std::fs::remove_dir_all(local.parent().unwrap().parent().unwrap());
}
