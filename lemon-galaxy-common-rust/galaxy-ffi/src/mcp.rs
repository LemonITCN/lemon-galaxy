//! 本机 MCP 服务（docs/mcp.md）：让这台电脑上的外部智能体（Claude Code、Codex、Cursor 等）经 MCP 使用
//! 账号里的设备、SSH 主机与本机。只有桌面端（macOS / Windows / Linux）可用；接口在所有平台都导出
//! （绑定各端一致），手机上调用返回「不支持」。
//!
//! - 每个外部客户端（按客户端名）一个智能体：确认过的设备、应用在它后续的调用里继续有效；空闲 2 小时回收。
//! - 档位默认「危险操作确认」；不受服务器管理员的 AI 开关与上限限制（用户拍板），被控端机主给账号设的
//!   AI 上限与绝对禁止清单照旧生效（被控端强制执行）。
//! - 「本机」在设备目录里，第一次用时和其他设备一样先请用户同意。
//! - 令牌与地址写在数据目录的 mcp-endpoint.json（只有本机用户可读），stdio 桥接从这里读。

use crate::types::*;
use crate::{GalaxyClient, Result};
use std::sync::Arc;

/// 默认端口（固定，外部客户端的配置不用跟着变）
pub const MCP_DEFAULT_PORT: u16 = 47821;

#[derive(uniffi::Record, Clone, Debug)]
pub struct McpConfig {
    /// 0 表示用默认端口
    pub port: u16,
    pub mode: AiMode,
    /// 完全信任档的时限（分钟），到时回到危险操作确认
    pub trust_minutes: Option<u32>,
}

#[derive(uniffi::Record, Clone, Debug, Default)]
pub struct McpClientInfo {
    /// 归类键（客户端名规整后），mcp_decide 等用它指定客户端
    pub key: String,
    pub name: String,
    pub version: String,
    pub last_active_ms: i64,
    /// 正在执行工具
    pub running: bool,
    /// 等用户批准的步骤数
    pub pending_approvals: u32,
}

#[derive(uniffi::Record, Clone, Debug, Default)]
pub struct McpStatus {
    /// 这个平台能不能开 MCP 服务（手机为 false）
    pub supported: bool,
    pub running: bool,
    /// 例如 http://127.0.0.1:47821/mcp
    pub url: String,
    pub port: u16,
    pub token: String,
    /// stdio 桥接读的端点文件
    pub endpoint_file: String,
    pub mode: Option<AiMode>,
    pub error: Option<String>,
    pub clients: Vec<McpClientInfo>,
}

/// 界面收 MCP 的状态与活动（可能在任意内核线程回调）。
#[uniffi::export(with_foreign)]
pub trait McpListener: Send + Sync {
    /// 服务状态、客户端列表、待批准数变化
    fn on_status(&self, status: McpStatus);
    /// 某个客户端的智能体事件：工具卡片、需要批准、输出、结果、请求输入密码等（与 AI 对话的事件相同）
    fn on_event(&self, client_key: String, event: AgentEvent);
}

#[uniffi::export]
pub fn mcp_default_port() -> u16 {
    MCP_DEFAULT_PORT
}

/// stdio 桥接：桌面应用以 `--mcp-stdio` 启动时调用，读标准输入、写标准输出，直到输入结束。返回进程退出码。
#[uniffi::export]
pub fn mcp_stdio_bridge(data_dir: String) -> i32 {
    imp::stdio_bridge(data_dir)
}

#[uniffi::export]
impl GalaxyClient {
    /// 开启（或按新设置重启）本机 MCP 服务。
    pub async fn mcp_start(&self, config: McpConfig, listener: Arc<dyn McpListener>) -> Result<McpStatus> {
        let inner = self.inner.clone();
        crate::run(async move { imp::start(inner, config, listener).await }).await
    }

    /// 关闭服务；正在等批准的步骤一律拒绝，正在跑的命令停止。
    pub fn mcp_stop(&self) {
        imp::stop();
    }

    pub fn mcp_status(&self) -> McpStatus {
        imp::status()
    }

    /// 换一个新令牌（已配置的外部客户端要更新配置）；服务开着时随即用新令牌重启。
    pub async fn mcp_regenerate_token(&self) -> Result<McpStatus> {
        let inner = self.inner.clone();
        crate::run(async move { imp::regenerate(inner).await }).await
    }

    /// 调整档位：立即作用于所有客户端。
    pub fn mcp_set_mode(&self, mode: AiMode, trust_minutes: Option<u32>) {
        imp::set_mode(mode, trust_minutes);
    }

    pub fn mcp_decide(&self, client_key: String, call_id: String, decision: ApprovalDecision) {
        imp::with_agent(&client_key, |a| a.decide(&call_id, decision.into()));
    }

    /// 回应 request_secret：内容直接送到设备，不经过外部智能体。
    pub fn mcp_provide_secret(&self, client_key: String, request_id: String, secret: String) {
        imp::with_agent(&client_key, |a| a.provide_secret(&request_id, secret));
    }

    /// 停下某个客户端：拒绝它等批准的步骤、停止正在跑的命令（之后的新调用照常）。
    pub fn mcp_stop_client(&self, client_key: String) {
        imp::with_agent(&client_key, |a| a.stop());
    }

    /// 某个客户端最近的操作记录（界面打开时补齐）。
    pub fn mcp_history(&self, client_key: String) -> Vec<AgentMessage> {
        imp::history(&client_key)
    }
}

/// 退出登录时调用：外部客户端的智能体（含已连的设备会话）全部作废，服务继续开着（新调用会提示未登录）。
pub(crate) fn reset_clients() {
    imp::reset_clients();
}

#[cfg(any(target_os = "ios", target_os = "android"))]
mod imp {
    use super::*;

    fn unsupported() -> crate::GalaxyError {
        crate::other("这个平台不支持 MCP 服务")
    }
    pub fn stdio_bridge(_data_dir: String) -> i32 {
        1
    }
    pub async fn start(_inner: Arc<crate::client::Inner>, _c: McpConfig, _l: Arc<dyn McpListener>) -> Result<McpStatus> {
        Err(unsupported())
    }
    pub fn stop() {}
    pub fn status() -> McpStatus {
        McpStatus::default()
    }
    pub async fn regenerate(_inner: Arc<crate::client::Inner>) -> Result<McpStatus> {
        Err(unsupported())
    }
    pub fn set_mode(_m: AiMode, _t: Option<u32>) {}
    pub fn with_agent(_key: &str, _f: impl FnOnce(&galaxy_agent::agent::Agent)) {}
    pub fn history(_key: &str) -> Vec<AgentMessage> {
        vec![]
    }
    pub fn reset_clients() {}
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
mod imp {
    use super::*;
    use crate::client::{ChatWorkspace, Inner};
    use base64::Engine;
    use galaxy_agent::agent as ga;
    use galaxy_mcp::{ClientIdent, Endpoint};
    use parking_lot::Mutex;
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;
    use std::sync::Weak;

    /// 空闲多久回收一个客户端的智能体（断开它连着的设备会话）
    const IDLE_MS: i64 = 2 * 3600 * 1000;

    struct ClientEntry {
        ident: ClientIdent,
        agent: Arc<ga::Agent>,
        last: i64,
        running: bool,
        pending: HashSet<String>,
    }

    struct Shared {
        inner: Weak<Inner>,
        listener: Arc<dyn McpListener>,
        mode: Mutex<(galaxy_proto::AiMode, Option<u32>)>,
        clients: Mutex<HashMap<String, ClientEntry>>,
        base: Mutex<McpStatus>,
    }

    struct Runtime {
        server: galaxy_mcp::McpServer,
        shared: Arc<Shared>,
        config: McpConfig,
    }

    static RUNTIME: Mutex<Option<Runtime>> = Mutex::new(None);

    fn endpoint_path() -> PathBuf {
        PathBuf::from(crate::config().data_dir).join("mcp-endpoint.json")
    }

    fn load_or_new_token() -> String {
        if let Ok(ep) = galaxy_mcp::read_endpoint(&endpoint_path()) {
            if ep.token.len() >= 32 {
                return ep.token;
            }
        }
        new_token()
    }

    fn new_token() -> String {
        format!("lgm_{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(galaxy_crypto::random_bytes::<32>()))
    }

    struct NoLlm;
    impl ga::Llm for NoLlm {
        fn stream(&self, _body: serde_json::Value) -> ga::BoxFut<'_, anyhow::Result<futures::stream::BoxStream<'static, anyhow::Result<bytes::Bytes>>>> {
            Box::pin(async { anyhow::bail!("外部智能体调用不经过大模型") })
        }
    }

    /// 智能体事件 → 界面；顺手维护「正在执行 / 待批准」。
    struct Ui {
        key: String,
        shared: Weak<Shared>,
    }

    impl ga::AgentUi for Ui {
        fn event(&self, e: ga::AgentEvent) {
            let Some(sh) = self.shared.upgrade() else { return };
            let changed = {
                let mut c = sh.clients.lock();
                match c.get_mut(&self.key) {
                    Some(entry) => match &e {
                        ga::AgentEvent::ToolProposed { call, needs_approval: true } => entry.pending.insert(call.call_id.clone()),
                        ga::AgentEvent::ToolStarted { call_id } | ga::AgentEvent::ToolFinished { call_id, .. } => entry.pending.remove(call_id),
                        ga::AgentEvent::Status { running, .. } => {
                            let ch = entry.running != *running;
                            entry.running = *running;
                            if !running {
                                entry.pending.clear();
                            }
                            ch
                        }
                        _ => false,
                    },
                    None => false,
                }
            };
            sh.listener.on_event(self.key.clone(), e.into());
            if changed {
                sh.emit();
            }
        }
    }

    impl Shared {
        fn status(&self) -> McpStatus {
            let mut s = self.base.lock().clone();
            let mut clients: Vec<McpClientInfo> = self
                .clients
                .lock()
                .values()
                .map(|e| McpClientInfo { key: e.ident.key.clone(), name: e.ident.name.clone(), version: e.ident.version.clone(), last_active_ms: e.last, running: e.running, pending_approvals: e.pending.len() as u32 })
                .collect();
            clients.sort_by_key(|c| std::cmp::Reverse(c.last_active_ms));
            s.clients = clients;
            s.mode = Some(self.mode.lock().0.into());
            s
        }

        fn emit(&self) {
            self.listener.on_status(self.status());
        }

        fn build_agent(self: &Arc<Self>, ident: &ClientIdent) -> anyhow::Result<Arc<ga::Agent>> {
            let inner = self.inner.upgrade().ok_or_else(|| anyhow::anyhow!("LemonGalaxy 已退出"))?;
            inner.api().map_err(|_| anyhow::anyhow!("LemonGalaxy 还没有登录账号：请先在 LemonGalaxy 里登录"))?;
            let exec = galaxy_client::ai::SessionExecutor::new();
            let ws = ChatWorkspace::new(&inner, exec.clone());
            let (mode, trust) = *self.mode.lock();
            let cfg = ga::AgentConfig {
                conversation_id: format!("mcp-{}-{}", ident.key, &uuid::Uuid::new_v4().simple().to_string()[..8]),
                provider_id: 0,
                model_id: String::new(),
                vision: true,
                context_length: 0,
                mode,
                // 用户拍板：MCP 不受服务器管理员的 AI 开关与上限限制；被控端机主的上限照旧（被控端强制）
                admin_cap: galaxy_proto::AiMode::FullTrust,
                plan_first: false,
                max_steps: 0,
                locale: crate::locale(),
                trust_minutes: trust,
            };
            let ui = Arc::new(Ui { key: ident.key.clone(), shared: Arc::downgrade(self) });
            // 「本机」不预先加入、也不预先授权：它在设备目录里，第一次用时和其他设备一样先请用户同意
            Ok(ga::Agent::new(cfg, Arc::new(NoLlm), exec, Some(ws as Arc<dyn ga::Workspace>), ui))
        }
    }

    struct HostImpl(Arc<Shared>);

    impl galaxy_mcp::Host for HostImpl {
        fn agent_for(&self, client: &ClientIdent) -> anyhow::Result<Arc<ga::Agent>> {
            let sh = &self.0;
            if let Some(e) = sh.clients.lock().get(&client.key) {
                return Ok(e.agent.clone());
            }
            let agent = sh.build_agent(client)?;
            let agent = {
                let mut c = sh.clients.lock();
                // 并发的第一次调用：以先登记的为准
                c.entry(client.key.clone())
                    .or_insert_with(|| ClientEntry { ident: client.clone(), agent, last: galaxy_proto::now_ms(), running: false, pending: HashSet::new() })
                    .agent
                    .clone()
            };
            sh.emit();
            Ok(agent)
        }

        fn touched(&self, client: &ClientIdent) {
            let mut c = self.0.clients.lock();
            if let Some(e) = c.get_mut(&client.key) {
                e.last = galaxy_proto::now_ms();
                if e.ident.version != client.version && !client.version.is_empty() {
                    e.ident.version = client.version.clone();
                }
            }
        }
    }

    pub fn stdio_bridge(data_dir: String) -> i32 {
        let path = PathBuf::from(data_dir).join("mcp-endpoint.json");
        let rt = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("LemonGalaxy MCP 桥接启动失败：{e}");
                return 1;
            }
        };
        match rt.block_on(galaxy_mcp::run_stdio_bridge(path, tokio::io::stdin(), tokio::io::stdout())) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("LemonGalaxy MCP 桥接出错：{e}");
                1
            }
        }
    }

    fn stop_agents(shared: &Shared) {
        let agents: Vec<Arc<ga::Agent>> = shared.clients.lock().drain().map(|(_, e)| e.agent).collect();
        for a in agents {
            a.stop();
        }
    }

    pub async fn start(inner: Arc<Inner>, config: McpConfig, listener: Arc<dyn McpListener>) -> Result<McpStatus> {
        // 重启：先关掉旧的（端口要先释放）
        let old = RUNTIME.lock().take();
        if let Some(mut r) = old {
            r.server.stop();
            stop_agents(&r.shared);
        }
        let port = if config.port == 0 { MCP_DEFAULT_PORT } else { config.port };
        let token = load_or_new_token();
        let path = endpoint_path();
        let base = McpStatus { supported: true, running: false, url: String::new(), port, token: token.clone(), endpoint_file: path.to_string_lossy().to_string(), mode: Some(config.mode), error: None, clients: vec![] };
        let shared = Arc::new(Shared { inner: Arc::downgrade(&inner), listener, mode: Mutex::new((config.mode.into(), config.trust_minutes)), clients: Mutex::new(HashMap::new()), base: Mutex::new(base) });
        let cfg = galaxy_mcp::ServerConfig::new(port, token.clone(), galaxy_proto::APP_VERSION.to_string());
        let server = match galaxy_mcp::start(cfg, Arc::new(HostImpl(shared.clone()))).await {
            Ok(s) => s,
            Err(e) => {
                let msg = format!("{e}（端口 {port} 可能被别的程序占用，可以换一个端口）");
                shared.base.lock().error = Some(msg.clone());
                shared.emit();
                return Err(crate::other(msg));
            }
        };
        let url = server.url();
        if let Err(e) = galaxy_mcp::write_endpoint(&path, &Endpoint { url: url.clone(), token, pid: std::process::id() }) {
            tracing::warn!("写 MCP 端点文件失败：{e}");
        }
        {
            let mut b = shared.base.lock();
            b.running = true;
            b.url = url;
        }
        spawn_reaper(Arc::downgrade(&shared));
        let st = shared.status();
        *RUNTIME.lock() = Some(Runtime { server, shared: shared.clone(), config });
        shared.emit();
        Ok(st)
    }

    /// 定时回收空闲的客户端（没在执行、没有待批准、超过 2 小时没调用）
    fn spawn_reaper(shared: Weak<Shared>) {
        crate::rt().spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(600)).await;
                let Some(sh) = shared.upgrade() else { break };
                let now = galaxy_proto::now_ms();
                let idle: Vec<Arc<ga::Agent>> = {
                    let mut c = sh.clients.lock();
                    let keys: Vec<String> = c.iter().filter(|(_, e)| !e.running && e.pending.is_empty() && now - e.last > IDLE_MS).map(|(k, _)| k.clone()).collect();
                    keys.iter().filter_map(|k| c.remove(k)).map(|e| e.agent).collect()
                };
                if !idle.is_empty() {
                    for a in idle {
                        a.stop();
                    }
                    sh.emit();
                }
            }
        });
    }

    pub fn stop() {
        let old = RUNTIME.lock().take();
        if let Some(mut r) = old {
            r.server.stop();
            stop_agents(&r.shared);
            {
                let mut b = r.shared.base.lock();
                b.running = false;
                b.url.clear();
            }
            r.shared.emit();
        }
    }

    pub fn status() -> McpStatus {
        if let Some(r) = RUNTIME.lock().as_ref() {
            return r.shared.status();
        }
        let token = galaxy_mcp::read_endpoint(&endpoint_path()).map(|e| e.token).unwrap_or_default();
        McpStatus { supported: true, port: MCP_DEFAULT_PORT, token, endpoint_file: endpoint_path().to_string_lossy().to_string(), ..Default::default() }
    }

    pub async fn regenerate(inner: Arc<Inner>) -> Result<McpStatus> {
        let token = new_token();
        let path = endpoint_path();
        let running = RUNTIME.lock().as_ref().map(|r| (r.config.clone(), r.shared.listener.clone()));
        let url = galaxy_mcp::read_endpoint(&path).map(|e| e.url).unwrap_or_default();
        galaxy_mcp::write_endpoint(&path, &Endpoint { url, token, pid: std::process::id() }).map_err(|e| crate::other(format!("写端点文件失败：{e}")))?;
        match running {
            Some((cfg, listener)) => start(inner, cfg, listener).await,
            None => Ok(status()),
        }
    }

    pub fn set_mode(mode: AiMode, trust_minutes: Option<u32>) {
        let guard = RUNTIME.lock();
        let Some(r) = guard.as_ref() else { return };
        *r.shared.mode.lock() = (mode.into(), trust_minutes);
        let agents: Vec<Arc<ga::Agent>> = r.shared.clients.lock().values().map(|e| e.agent.clone()).collect();
        let shared = r.shared.clone();
        drop(guard);
        for a in agents {
            a.set_mode(mode.into(), trust_minutes);
        }
        shared.emit();
    }

    pub fn with_agent(key: &str, f: impl FnOnce(&ga::Agent)) {
        let agent = RUNTIME.lock().as_ref().and_then(|r| r.shared.clients.lock().get(key).map(|e| e.agent.clone()));
        if let Some(a) = agent {
            f(&a);
        }
    }

    pub fn history(key: &str) -> Vec<AgentMessage> {
        let agent = RUNTIME.lock().as_ref().and_then(|r| r.shared.clients.lock().get(key).map(|e| e.agent.clone()));
        agent.map(|a| a.messages().into_iter().map(Into::into).collect()).unwrap_or_default()
    }

    pub fn reset_clients() {
        let shared = RUNTIME.lock().as_ref().map(|r| r.shared.clone());
        if let Some(sh) = shared {
            stop_agents(&sh);
            sh.emit();
        }
    }
}
