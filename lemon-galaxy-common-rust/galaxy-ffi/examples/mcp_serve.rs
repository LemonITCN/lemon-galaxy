//! 本机 MCP 服务的命令行试跑：走和桌面客户端完全相同的内核接口（登录、本机被控、mcp_start），
//! 打印外部智能体的每一步，按参数自动同意或拒绝确认。开发自测用，不随产品发布。
//!
//! ```text
//! cargo run -p galaxy-ffi --example mcp_serve -- <数据目录> <服务器地址> <用户名> <密码> [--new-pass 新密码]
//!     [--port 端口] [--mode read_only|confirm_all|confirm_dangerous|full_trust] [--yes] [--seconds 秒]
//! cargo run -p galaxy-ffi --example mcp_serve -- --stdio <数据目录>      stdio 桥接（同 LemonGalaxy --mcp-stdio）
//! ```
//! `--yes` 自动同意所有确认（默认一律拒绝）。服务起来后打印地址与令牌，直到 `--seconds` 到时或被结束。

use galaxy_ffi::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

struct FileStore(std::path::PathBuf, Mutex<HashMap<String, Vec<u8>>>);
impl FileStore {
    fn new(dir: &str) -> Self {
        let p = std::path::Path::new(dir).join("secure-store.json");
        let m: HashMap<String, Vec<u8>> = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self(p, Mutex::new(m))
    }
    fn save(&self) {
        let _ = std::fs::write(&self.0, serde_json::to_vec(&*self.1.lock().unwrap()).unwrap());
    }
}
impl SecureStore for FileStore {
    fn get(&self, key: String) -> Option<Vec<u8>> {
        self.1.lock().unwrap().get(&key).cloned()
    }
    fn set(&self, key: String, value: Vec<u8>) {
        self.1.lock().unwrap().insert(key, value);
        self.save();
    }
    fn remove(&self, key: String) {
        self.1.lock().unwrap().remove(&key);
        self.save();
    }
}

struct Quiet;
impl ClientListener for Quiet {
    fn on_state(&self, _s: ClientState) {}
    fn on_devices_changed(&self) {}
    fn on_vault_changed(&self) {}
    fn on_session_revoked(&self) {}
}

static CLIENT: OnceLock<Arc<GalaxyClient>> = OnceLock::new();

struct Printer {
    yes: bool,
}
impl McpListener for Printer {
    fn on_status(&self, s: McpStatus) {
        let clients: Vec<String> = s.clients.iter().map(|c| format!("{}({}){}{}", c.name, c.key, if c.running { " 执行中" } else { "" }, if c.pending_approvals > 0 { format!(" 待批准 {}", c.pending_approvals) } else { String::new() })).collect();
        println!("[状态] 运行={} {} 客户端：{}", s.running, s.url, if clients.is_empty() { "无".into() } else { clients.join("，") });
    }
    fn on_event(&self, key: String, e: AgentEvent) {
        match e {
            AgentEvent::ToolProposed { call, needs_approval } => {
                println!("[{key}] 工具 {} @{} 风险={:?} {}{}", call.tool, if call.target_name.is_empty() { "-" } else { &call.target_name }, call.risk, call.summary, if needs_approval { if self.yes { "（需确认：自动同意）" } else { "（需确认：自动拒绝）" } } else { "" });
                if needs_approval {
                    let d = if self.yes { ApprovalDecision::AllowOnce } else { ApprovalDecision::Deny { reason: Some("试跑脚本默认拒绝".into()) } };
                    let id = call.call_id.clone();
                    // 回调里不要同步作答（内核还拿着锁的路径），换线程
                    std::thread::spawn(move || {
                        if let Some(c) = CLIENT.get() {
                            c.mcp_decide(key, id, d);
                        }
                    });
                }
            }
            AgentEvent::ToolFinished { result, .. } => {
                let out: String = result.output.chars().take(300).collect();
                println!("    → {}{}{}", if result.ok { "成功" } else if result.denied { "被拒绝" } else { "失败" }, if result.summary.is_empty() { String::new() } else { format!("：{}", result.summary) }, if out.is_empty() { String::new() } else { format!("\n      {}", out.replace('\n', "\n      ")) });
            }
            AgentEvent::SecretRequested { prompt, .. } => println!("[{key}] 请求输入密码：{prompt}（试跑脚本不回应）"),
            _ => {}
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|a| a == "--stdio").unwrap_or(false) {
        std::process::exit(mcp_stdio_bridge(args.get(1).cloned().expect("--stdio <数据目录>")));
    }
    let (dir, server, user, pass) = (&args[0], &args[1], &args[2], &args[3]);
    let flag = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let yes = args.iter().any(|a| a == "--yes");
    let mode = match flag("--mode").as_deref() {
        Some("read_only") => AiMode::ReadOnly,
        Some("confirm_all") => AiMode::ConfirmAll,
        Some("full_trust") => AiMode::FullTrust,
        _ => AiMode::ConfirmDangerous,
    };
    let port: u16 = flag("--port").and_then(|p| p.parse().ok()).unwrap_or(0);
    let seconds: u64 = flag("--seconds").and_then(|p| p.parse().ok()).unwrap_or(3600);
    let host = String::from_utf8(std::process::Command::new("hostname").output().map(|o| o.stdout).unwrap_or_default()).unwrap_or_default().trim().to_string();
    core_init(CoreConfig {
        data_dir: dir.clone(),
        cache_dir: format!("{dir}/cache"),
        log_level: "warn".into(),
        platform: galaxy_proto::Platform::current().into(),
        device_kind: DeviceKind::Control,
        app_version: "0.1.0-dev".into(),
        os_version: String::from_utf8(std::process::Command::new("uname").arg("-sr").output().map(|o| o.stdout).unwrap_or_default()).unwrap_or_default().trim().to_string(),
        hostname: host,
        locale: "zh-CN".into(),
        host_enabled: true,
    });
    let client = GalaxyClient::new(Arc::new(FileStore::new(dir)), Arc::new(Quiet));
    let _ = CLIENT.set(client.clone());
    futures::executor::block_on(async {
        if !client.restore() {
            match client.login(server.clone(), user.clone(), pass.clone(), None).await.expect("登录失败") {
                LoginResult::Success => {}
                LoginResult::MustChangePassword => {
                    let np = flag("--new-pass").expect("首次登录要改密码：加 --new-pass");
                    client.complete_first_login(np).await.expect("改密码失败");
                }
                other => panic!("登录需要额外步骤：{other:?}"),
            }
        }
        let t0 = Instant::now();
        while client.host().is_none() && t0.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(200));
        }
        println!("本机被控：{}", if client.host().is_some() { "已启动" } else { "没启动（本机目标不可用）" });
        let st = client.mcp_start(McpConfig { port, mode, trust_minutes: None }, Arc::new(Printer { yes })).await.expect("MCP 服务启动失败");
        println!("MCP 地址 {}\nMCP 令牌 {}\n端点文件 {}", st.url, st.token, st.endpoint_file);
    });
    std::thread::sleep(Duration::from_secs(seconds));
    client.mcp_stop();
}
