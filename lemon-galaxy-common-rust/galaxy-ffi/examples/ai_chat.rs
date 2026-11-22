//! AI 对话的命令行试跑：走和桌面客户端完全相同的内核接口（登录、本机被控、对话、本机目标、引用），
//! 用真实模型跑场景，打印模型输出与工具调用。开发自测用，不随产品发布。
//!
//! ```text
//! cargo run -p galaxy-ffi --example ai_chat -- <数据目录> <服务器地址> <用户名> <密码> [--model 模型] [--mode 档位] [--yes] -- <消息1> [<消息2> …]
//! ```
//! `--yes` 自动同意所有确认（默认一律拒绝）；多条消息按顺序在同一个对话里发。

use galaxy_ffi::*;
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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

struct Printer {
    yes: bool,
    running: AtomicBool,
    conv: Mutex<Option<Arc<AgentConversation>>>,
    thinking: AtomicBool,
}
impl AgentListener for Printer {
    fn on_event(&self, e: AgentEvent) {
        let mut o = std::io::stdout();
        match e {
            AgentEvent::Thinking { text, .. } => {
                if !self.thinking.swap(true, Ordering::Relaxed) {
                    print!("\n\x1b[2m[思考] ");
                }
                print!("{text}");
            }
            AgentEvent::AssistantDelta { text, .. } => {
                if self.thinking.swap(false, Ordering::Relaxed) {
                    print!("\x1b[0m\n");
                }
                print!("{text}");
            }
            AgentEvent::AssistantDone { .. } => {
                if self.thinking.swap(false, Ordering::Relaxed) {
                    print!("\x1b[0m");
                }
                println!();
            }
            AgentEvent::ToolProposed { call, needs_approval } => {
                println!("\n[工具] {} @{} 风险={:?} {}{}", call.tool, if call.target_name.is_empty() { "-" } else { &call.target_name }, call.risk, call.summary, if needs_approval { if self.yes { "（需确认：自动同意）" } else { "（需确认：自动拒绝）" } } else { "" });
                if needs_approval {
                    if let Some(c) = self.conv.lock().unwrap().clone() {
                        c.decide(call.call_id, if self.yes { ApprovalDecision::AllowOnce } else { ApprovalDecision::Deny { reason: Some("自动化测试中拒绝".into()) } });
                    }
                }
            }
            AgentEvent::ToolFinished { result, .. } => {
                let out: String = result.output.lines().take(12).collect::<Vec<_>>().join("\n    ");
                println!("[结果] {} {}{}", if result.ok { "成功" } else { "失败" }, result.summary, if out.is_empty() { String::new() } else { format!("\n    {out}") });
            }
            AgentEvent::AskUser { question_id, question, options } => {
                println!("[提问] {question} {options:?}（自动回答第一个选项）");
                if let Some(c) = self.conv.lock().unwrap().clone() {
                    c.answer(question_id, options.first().cloned().unwrap_or_else(|| "你决定".into()));
                }
            }
            AgentEvent::Usage { prompt_tokens, completion_tokens } => println!("[用量] 输入 {prompt_tokens} 输出 {completion_tokens}"),
            AgentEvent::Status { running, .. } => self.running.store(running, Ordering::Relaxed),
            AgentEvent::Error { message } => println!("[错误] {message}"),
            _ => {}
        }
        let _ = o.flush();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let split = args.iter().position(|a| a == "--").expect("用 -- 分隔消息");
    let (opts, msgs) = (&args[..split], &args[split + 1..]);
    let (dir, server, user, pass) = (&opts[0], &opts[1], &opts[2], &opts[3]);
    let flag = |k: &str| opts.iter().position(|a| a == k).and_then(|i| opts.get(i + 1)).cloned();
    let yes = opts.iter().any(|a| a == "--yes");
    let mode = match flag("--mode").as_deref() {
        Some("read_only") => AiMode::ReadOnly,
        Some("confirm_all") => AiMode::ConfirmAll,
        Some("full_trust") => AiMode::FullTrust,
        _ => AiMode::ConfirmDangerous,
    };
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
    futures::executor::block_on(async {
        if !client.restore() {
            match client.login(server.clone(), user.clone(), pass.clone(), None).await.expect("登录失败") {
                LoginResult::Success => {}
                other => panic!("登录需要额外步骤：{other:?}"),
            }
        }
        let t0 = Instant::now();
        while client.host().is_none() && t0.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(200));
        }
        println!("本机被控：{}", if client.host().is_some() { "已启动" } else { "没启动（本机目标不可用）" });
        let models = client.ai_models().await.expect("取模型列表失败");
        let want = flag("--model").unwrap_or_else(|| "deepseek-v4.1-flash".into());
        let m = models.items.iter().find(|m| m.id == want).expect("没有这个模型").clone();
        println!("模型 {}（{}）档位 {mode:?}", m.name, m.id);
        let printer = Arc::new(Printer { yes, running: AtomicBool::new(false), conv: Mutex::new(None), thinking: AtomicBool::new(false) });
        let conv = client
            .new_conversation(ConversationConfig { provider_id: m.provider_id, model_id: m.id.clone(), targets: vec![], mode, plan_first: false, trust_minutes: None, title: None }, printer.clone())
            .await
            .expect("建对话失败");
        *printer.conv.lock().unwrap() = Some(conv.clone());
        for msg in msgs {
            println!("\n==================== 用户：{msg}");
            printer.running.store(true, Ordering::Relaxed);
            conv.send(msg.clone());
            std::thread::sleep(Duration::from_millis(500));
            let t = Instant::now();
            while printer.running.load(Ordering::Relaxed) && t.elapsed() < Duration::from_secs(600) {
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    });
}
