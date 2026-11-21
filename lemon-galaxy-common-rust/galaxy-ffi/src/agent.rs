//! AI 对话。对话记录只存在本机（用设备密钥派生的密钥加密）。

use crate::callbacks::*;
use crate::types::*;
use crate::{rt, run, Result};
use galaxy_agent::agent as ga;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Serialize, Deserialize)]
pub(crate) struct SavedConversation {
    pub id: String,
    pub title: String,
    pub model_id: String,
    pub target_names: Vec<String>,
    pub updated_at: i64,
    pub messages: Vec<ga::AgentMessage>,
}

pub(crate) fn conv_dir() -> PathBuf {
    PathBuf::from(crate::config().data_dir).join("conversations")
}

pub(crate) fn save_conv(key: &[u8; 32], c: &SavedConversation) {
    let _ = std::fs::create_dir_all(conv_dir());
    if let Ok(plain) = serde_json::to_vec(c) {
        let (mut n, ct) = galaxy_crypto::seal(key, &plain, b"lemon-galaxy/conversation/v1");
        n.extend_from_slice(&ct);
        let safe: String = c.id.chars().filter(|x| x.is_ascii_alphanumeric() || *x == '-').collect();
        let _ = std::fs::write(conv_dir().join(format!("{safe}.bin")), n);
    }
}

pub(crate) fn load_conv(key: &[u8; 32], path: &std::path::Path) -> Option<SavedConversation> {
    let b = std::fs::read(path).ok()?;
    if b.len() < 24 {
        return None;
    }
    let plain = galaxy_crypto::open(key, &b[..24], &b[24..], b"lemon-galaxy/conversation/v1").ok()?;
    serde_json::from_slice(&plain).ok()
}

struct UiBridge {
    listener: Arc<dyn AgentListener>,
    on_change: Box<dyn Fn() + Send + Sync>,
}

impl ga::AgentUi for UiBridge {
    fn event(&self, e: ga::AgentEvent) {
        let persist = matches!(e, ga::AgentEvent::AssistantDone { .. } | ga::AgentEvent::ToolFinished { .. } | ga::AgentEvent::Status { running: false, .. });
        self.listener.on_event(e.into());
        if persist {
            (self.on_change)();
        }
    }
}

#[derive(uniffi::Object)]
pub struct AgentConversation {
    pub(crate) agent: Arc<ga::Agent>,
    pub(crate) meta: Arc<Mutex<SavedConversation>>,
    pub(crate) key: [u8; 32],
    pub(crate) foreground: Arc<Mutex<bool>>,
}

impl AgentConversation {
    pub(crate) fn build(cfg: ga::AgentConfig, llm: Arc<dyn ga::Llm>, exec: Arc<dyn ga::Executor>, listener: Arc<dyn AgentListener>, key: [u8; 32], title: String, target_names: Vec<String>) -> Arc<Self> {
        let meta = Arc::new(Mutex::new(SavedConversation { id: cfg.conversation_id.clone(), title, model_id: cfg.model_id.clone(), target_names, updated_at: galaxy_proto::now_ms(), messages: vec![] }));
        let holder: Arc<Mutex<Option<std::sync::Weak<ga::Agent>>>> = Arc::new(Mutex::new(None));
        let (m2, h2) = (meta.clone(), holder.clone());
        let on_change = Box::new(move || {
            if let Some(a) = h2.lock().as_ref().and_then(|w| w.upgrade()) {
                let mut m = m2.lock();
                m.messages = a.messages();
                m.updated_at = galaxy_proto::now_ms();
                save_conv(&key, &m);
            }
        });
        let agent = ga::Agent::new(cfg, llm, exec, Arc::new(UiBridge { listener, on_change }));
        *holder.lock() = Some(Arc::downgrade(&agent));
        Arc::new(Self { agent, meta, key, foreground: Arc::new(Mutex::new(true)) })
    }
}

#[uniffi::export]
impl AgentConversation {
    pub fn id(&self) -> String {
        self.agent.id()
    }

    pub fn title(&self) -> String {
        self.meta.lock().title.clone()
    }

    /// 发送一条消息；任务在内核后台运行，进度经监听器回调。
    pub fn send(&self, text: String) {
        {
            let mut m = self.meta.lock();
            if m.title.is_empty() {
                m.title = text.chars().take(40).collect();
            }
        }
        let a = self.agent.clone();
        rt().spawn(async move { a.send(text).await });
    }

    pub fn decide(&self, call_id: String, decision: ApprovalDecision) {
        self.agent.decide(&call_id, decision.into());
    }

    pub fn answer(&self, question_id: String, answer: String) {
        self.agent.answer(&question_id, answer);
    }

    /// 回应 request_secret：内容直接送到被控端，不进模型上下文。
    pub fn provide_secret(&self, request_id: String, secret: String) {
        self.agent.provide_secret(&request_id, secret);
    }

    pub fn set_mode(&self, mode: AiMode, trust_minutes: Option<u32>) {
        self.agent.set_mode(mode.into(), trust_minutes);
    }

    pub fn stop(&self) {
        self.agent.stop();
    }

    /// 移动端切到后台时调用 false：正在进行的任务停止，回到前台后可继续发送。
    pub fn set_foreground(&self, foreground: bool) {
        *self.foreground.lock() = foreground;
        if !foreground {
            self.agent.stop();
        }
    }

    pub fn messages(&self) -> Vec<AgentMessage> {
        let live = self.agent.messages();
        let v = if live.is_empty() { self.meta.lock().messages.clone() } else { live };
        v.into_iter().map(Into::into).collect()
    }
}

/// 列出本机保存的对话。
pub(crate) fn list_conversations(key: &[u8; 32]) -> Vec<ConversationSummary> {
    let mut v = vec![];
    if let Ok(rd) = std::fs::read_dir(conv_dir()) {
        for e in rd.flatten() {
            if let Some(c) = load_conv(key, &e.path()) {
                v.push(ConversationSummary { id: c.id, title: c.title, model_id: c.model_id, target_names: c.target_names, updated_at: c.updated_at });
            }
        }
    }
    v.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    v
}

pub(crate) async fn _unused() -> Result<()> {
    run(async { Ok(()) }).await
}
