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
    #[serde(default)]
    pub config: Option<ConversationConfig>,
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
    #[allow(dead_code)]
    pub(crate) key: [u8; 32],
    pub(crate) foreground: Arc<Mutex<bool>>,
    pub(crate) ws: Arc<crate::client::ChatWorkspace>,
}

impl AgentConversation {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build(cfg: ga::AgentConfig, llm: Arc<dyn ga::Llm>, exec: Arc<dyn ga::Executor>, ws: Arc<crate::client::ChatWorkspace>, listener: Arc<dyn AgentListener>, key: [u8; 32], title: String, target_names: Vec<String>, ui_config: ConversationConfig) -> Arc<Self> {
        let meta = Arc::new(Mutex::new(SavedConversation { id: cfg.conversation_id.clone(), title, model_id: cfg.model_id.clone(), target_names, updated_at: galaxy_proto::now_ms(), messages: vec![], config: Some(ui_config) }));
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
        let agent = ga::Agent::new(cfg, llm, exec, Some(ws.clone() as Arc<dyn ga::Workspace>), Arc::new(UiBridge { listener, on_change }));
        *holder.lock() = Some(Arc::downgrade(&agent));
        Arc::new(Self { agent, meta, key, foreground: Arc::new(Mutex::new(true)), ws })
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
        self.send_with_refs(text, vec![]);
    }

    /// 发送一条带引用的消息：引用的设备与主机可直接使用；文件内容、目录列表、片段、端口映射、连接记录展开后随消息给模型。
    /// 展开在后台进行（可能要先连上设备），失败的引用以说明文字代替，不影响发送。
    pub fn send_with_refs(&self, text: String, refs: Vec<ContextRef>) {
        {
            let mut m = self.meta.lock();
            if m.title.is_empty() {
                let t: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
                m.title = if t.is_empty() { refs.first().map(|r| r.label.clone()).unwrap_or_default() } else { t.chars().take(40).collect() };
            }
        }
        let a = self.agent.clone();
        let ws = self.ws.clone();
        rt().spawn(async move {
            let (context, targets) = resolve_refs(&ws, &refs).await;
            a.send_input(ga::UserInput { text, context, refs: refs.into_iter().map(Into::into).collect(), targets }).await
        });
    }

    /// 对话中途换模型（上下文保留）。
    pub fn set_model(&self, provider_id: u64, model_id: String) -> Result<()> {
        let models = self.ws.cached_models().ok_or_else(|| crate::other("模型列表还没有加载"))?;
        let m = models.items.iter().find(|m| m.provider_id == provider_id && m.id == model_id).ok_or_else(|| crate::GalaxyError::Permission { detail: "未开通该模型".into() })?;
        self.agent.set_model(m.provider_id, m.id.clone(), m.vision, m.context_length);
        let mut meta = self.meta.lock();
        meta.model_id = m.id.clone();
        if let Some(c) = meta.config.as_mut() {
            c.provider_id = m.provider_id;
            c.model_id = m.id.clone();
        }
        Ok(())
    }

    /// 当前使用的模型（provider_id, model_id）。
    pub fn model(&self) -> ConversationModel {
        let (provider_id, model_id) = self.agent.model_id();
        ConversationModel { provider_id, model_id }
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

#[derive(uniffi::Record, Clone, Debug)]
pub struct ConversationModel {
    pub provider_id: u64,
    pub model_id: String,
}

const MAX_REF_FILE: u64 = 256 * 1024;

/// 把引用展开成给模型看的文字，并找出用户点名的目标。
async fn resolve_refs(ws: &Arc<crate::client::ChatWorkspace>, refs: &[ContextRef]) -> (String, Vec<String>) {
    use galaxy_agent::agent::{Executor, Workspace};
    let mut out = String::new();
    let mut targets = vec![];
    for r in refs {
        let where_ = |t: &str| ws.name_of(t);
        match r.kind {
            ContextRefKind::Device | ContextRefKind::SshHost => {
                targets.push(r.id.clone());
                let detail = ws.catalog().await.ok().and_then(|c| c.into_iter().find(|x| x.id == r.id)).map(|x| format!("{}，{}，{}", x.platform.as_str(), if x.online { "在线" } else { "离线" }, x.detail)).unwrap_or_default();
                out.push_str(&format!("<引用 类型=\"{}\" target=\"{}\" 名称=\"{}\">{detail}</引用>\n", if r.kind == ContextRefKind::Device { "设备" } else { "SSH 主机" }, r.id, where_(&r.id)));
            }
            ContextRefKind::File | ContextRefKind::Folder => {
                targets.push(r.target_id.clone());
                let name = where_(&r.target_id);
                let body = match ws.connect(&r.target_id).await {
                    Err(e) => format!("（无法连接 {name}：{e}）"),
                    Ok(()) if r.kind == ContextRefKind::File => match ws.exec.read_file(&r.target_id, &r.id, MAX_REF_FILE + 1).await {
                        Err(e) => format!("（读取失败：{e}）"),
                        Ok(d) if d.iter().take(8000).any(|b| *b == 0) => format!("（二进制文件，{} 字节，不显示内容）", d.len()),
                        Ok(d) => {
                            let cut = d.len() as u64 > MAX_REF_FILE;
                            let t = String::from_utf8_lossy(&d[..d.len().min(MAX_REF_FILE as usize)]).to_string();
                            if cut { format!("{t}\n（文件超过 256 KB，后面的内容未附上，可用 read_file 按行读取）") } else { t }
                        }
                    },
                    Ok(()) => match ws.exec.list_dir(&r.target_id, &r.id).await {
                        Err(e) => format!("（列目录失败：{e}）"),
                        Ok(v) => {
                            let mut t = String::new();
                            for e in v.iter().take(300) {
                                t.push_str(&format!("{}\t{}\t{}\n", match e.kind { galaxy_proto::peer::FileKind::Dir => "目录", galaxy_proto::peer::FileKind::Symlink => "链接", _ => "文件" }, e.size, e.name));
                            }
                            if v.len() > 300 {
                                t.push_str(&format!("……共 {} 项，只列出前 300 项\n", v.len()));
                            }
                            t
                        }
                    },
                };
                out.push_str(&format!("<引用 类型=\"{}\" target=\"{}\" 设备=\"{name}\" 路径=\"{}\">\n{body}\n</引用>\n", if r.kind == ContextRefKind::File { "文件" } else { "目录" }, r.target_id, r.id));
            }
            ContextRefKind::Snippet => out.push_str(&format!("<引用 类型=\"命令片段\">\n{}</引用>\n", ws.snippets_text(Some(&r.id)))),
            ContextRefKind::PortForward => out.push_str(&format!("<引用 类型=\"端口映射\">\n{}</引用>\n", ws.forwards_text(Some(&r.id)))),
            ContextRefKind::Activity => {
                let t = ws.query("activity", 50).await.unwrap_or_else(|e| format!("（读取失败：{e}）"));
                out.push_str(&format!("<引用 类型=\"最近连接与登录记录\">\n{t}</引用>\n"));
            }
        }
    }
    (out, targets)
}

/// 列出本机保存的对话。
pub(crate) fn list_conversations(key: &[u8; 32]) -> Vec<ConversationSummary> {
    let mut v = vec![];
    if let Ok(rd) = std::fs::read_dir(conv_dir()) {
        for e in rd.flatten() {
            if let Some(c) = load_conv(key, &e.path()) {
                v.push(ConversationSummary { id: c.id, title: c.title, model_id: c.model_id, target_names: c.target_names, updated_at: c.updated_at, config: c.config });
            }
        }
    }
    v.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    v
}

pub(crate) async fn _unused() -> Result<()> {
    run(async { Ok(()) }).await
}
