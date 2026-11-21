//! AI：网关流式接口与工具执行器（把智能体的工具调用落到与被控设备的会话上）。

use crate::api::Api;
use crate::session::{PeerSession, TerminalChannel, TerminalSink};
use bytes::Bytes;
use futures::stream::BoxStream;
use futures::StreamExt;
use galaxy_agent::agent::{BoxFut, CmdResult, Executor, Llm, TargetInfo, TargetKind};
use galaxy_proto::peer::*;
use galaxy_proto::AiMode;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::watch;

/// 经服务器网关请求大模型。
pub struct GatewayLlm {
    pub api: Api,
}

impl Llm for GatewayLlm {
    fn stream(&self, body: serde_json::Value) -> BoxFut<'_, anyhow::Result<BoxStream<'static, anyhow::Result<Bytes>>>> {
        Box::pin(async move {
            let resp = self.api.post_stream("/ai/chat/completions", &body).await.map_err(|e| anyhow::anyhow!("{e}"))?;
            let s = resp.bytes_stream().map(|r| r.map_err(|e| anyhow::anyhow!("{e}")));
            Ok(s.boxed())
        })
    }
}

struct NullSink;
impl TerminalSink for NullSink {
    fn on_output(&self, _d: Vec<u8>) {}
    fn on_exit(&self, _c: Option<i32>) {}
}

/// 主控端的工具执行器：目标 ID → 会话。
pub struct SessionExecutor {
    pub sessions: Mutex<HashMap<String, (Arc<PeerSession>, String)>>,
    terms: Mutex<HashMap<String, Arc<TerminalChannel>>>,
}

impl SessionExecutor {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { sessions: Default::default(), terms: Default::default() })
    }

    /// 加入一个目标（设备会话 + 显示名称）。
    pub fn add(&self, id: String, s: Arc<PeerSession>, name: String) {
        self.sessions.lock().insert(id, (s, name));
    }

    fn sess(&self, target: &str) -> anyhow::Result<Arc<PeerSession>> {
        self.sessions.lock().get(target).map(|x| x.0.clone()).ok_or_else(|| anyhow::anyhow!("目标未连接：{target}"))
    }
}

fn e<E: std::fmt::Display>(x: E) -> anyhow::Error {
    anyhow::anyhow!("{x}")
}

impl Executor for SessionExecutor {
    fn targets(&self) -> Vec<TargetInfo> {
        self.sessions
            .lock()
            .iter()
            .map(|(id, (s, name))| {
                let h = &s.ack.host;
                TargetInfo {
                    id: id.clone(),
                    name: name.clone(),
                    kind: TargetKind::Device,
                    platform: h.platform,
                    os: h.os_version.clone(),
                    user: h.system_user.clone(),
                    shell: h.shell.clone(),
                    gui: s.ack.capabilities.iter().any(|c| c == "screenshot" || c == "display"),
                    ai_cap: if s.ack.permissions.terminal || s.ack.permissions.files { s.ack.permissions.ai_cap } else { AiMode::ReadOnly.min(s.ack.permissions.ai_cap) },
                }
            })
            .collect()
    }

    fn run_command(&self, target: &str, cmd: &str, cwd: Option<String>, timeout_s: u64, shell: Option<String>, origin: Origin, on_out: Box<dyn Fn(&str) + Send + Sync>, cancel: watch::Receiver<bool>) -> BoxFut<'_, anyhow::Result<CmdResult>> {
        let (target, cmd) = (target.to_string(), cmd.to_string());
        Box::pin(async move {
            let s = self.sess(&target)?;
            let req = ExecRequest { command: cmd, shell, cwd, env: vec![], timeout_ms: timeout_s * 1000, origin, max_output: 4 * 1024 * 1024 };
            let (done, so, se) = s.exec(req, |_, d| on_out(&String::from_utf8_lossy(d)), Some(cancel)).await.map_err(e)?;
            Ok(CmdResult {
                code: done.code,
                stdout: String::from_utf8_lossy(&so).to_string(),
                stderr: String::from_utf8_lossy(&se).to_string(),
                timed_out: done.timed_out,
                truncated: done.truncated,
                duration_ms: done.duration_ms,
            })
        })
    }

    fn read_file(&self, target: &str, path: &str, max_bytes: u64) -> BoxFut<'_, anyhow::Result<Vec<u8>>> {
        let (target, path) = (target.to_string(), path.to_string());
        Box::pin(async move { self.sess(&target)?.fs_read_all(&path, max_bytes).await.map_err(e) })
    }

    fn exists(&self, target: &str, path: &str) -> BoxFut<'_, anyhow::Result<bool>> {
        let (target, path) = (target.to_string(), path.to_string());
        Box::pin(async move { Ok(self.sess(&target)?.fs_stat(&path).await.is_ok()) })
    }

    fn write_file(&self, target: &str, path: &str, data: Vec<u8>, expect: Option<Vec<u8>>, origin: Origin) -> BoxFut<'_, anyhow::Result<Option<String>>> {
        let (target, path) = (target.to_string(), path.to_string());
        Box::pin(async move { self.sess(&target)?.fs_write(&path, 0, true, None, expect, origin, &data).await.map(|x| x.1).map_err(e) })
    }

    fn list_dir(&self, target: &str, path: &str) -> BoxFut<'_, anyhow::Result<Vec<FileEntry>>> {
        let (target, path) = (target.to_string(), path.to_string());
        Box::pin(async move { self.sess(&target)?.fs_list(&path).await.map_err(e) })
    }

    fn search(&self, target: &str, root: &str, pattern: &str, content: bool) -> BoxFut<'_, anyhow::Result<Vec<FileEntry>>> {
        let (target, root, pattern) = (target.to_string(), root.to_string(), pattern.to_string());
        Box::pin(async move { self.sess(&target)?.fs_search(&root, &pattern, content, 200).await.map_err(e) })
    }

    fn displays(&self, target: &str) -> BoxFut<'_, anyhow::Result<Vec<DisplayInfo>>> {
        let target = target.to_string();
        Box::pin(async move { self.sess(&target)?.list_displays().await.map_err(e) })
    }

    fn screenshot(&self, target: &str, display: u32, max_w: u32, max_h: u32) -> BoxFut<'_, anyhow::Result<ImageData>> {
        let target = target.to_string();
        Box::pin(async move { self.sess(&target)?.screenshot(display, max_w, max_h).await.map_err(e) })
    }

    fn ui_tree(&self, target: &str, display: u32, max_nodes: u32) -> BoxFut<'_, anyhow::Result<String>> {
        let target = target.to_string();
        Box::pin(async move {
            match self.sess(&target)?.mgmt(MgmtReq::UiTree { display, max_nodes }).await.map_err(e)? {
                MgmtResp::Text(t) => Ok(t),
                _ => anyhow::bail!("应答错误"),
            }
        })
    }

    fn inject(&self, target: &str, events: Vec<InputEvent>, origin: Origin) -> BoxFut<'_, anyhow::Result<()>> {
        let target = target.to_string();
        Box::pin(async move { self.sess(&target)?.mgmt(MgmtReq::Inject { events, origin }).await.map(|_| ()).map_err(e) })
    }

    fn terminal_open(&self, target: &str, origin: Origin) -> BoxFut<'_, anyhow::Result<String>> {
        let target = target.to_string();
        Box::pin(async move {
            let s = self.sess(&target)?;
            let ch = s.open_terminal(TerminalOpen { attach: None, shell: None, cols: 120, rows: 36, env: vec![], origin }, Arc::new(NullSink)).await.map_err(e)?;
            let id = ch.session_id.clone();
            self.terms.lock().insert(id.clone(), ch);
            Ok(id)
        })
    }

    fn terminal_send(&self, term: &str, data: Vec<u8>) -> BoxFut<'_, anyhow::Result<()>> {
        let term = term.to_string();
        Box::pin(async move {
            let ch = self.terms.lock().get(&term).cloned().ok_or_else(|| anyhow::anyhow!("终端不存在"))?;
            ch.write(data).await.map_err(e)
        })
    }

    fn terminal_read(&self, term: &str) -> BoxFut<'_, anyhow::Result<String>> {
        let term = term.to_string();
        Box::pin(async move {
            let ch = self.terms.lock().get(&term).cloned().ok_or_else(|| anyhow::anyhow!("终端不存在"))?;
            ch.snapshot().await.map_err(e)
        })
    }

    fn terminal_close(&self, term: &str) -> BoxFut<'_, anyhow::Result<()>> {
        let term = term.to_string();
        Box::pin(async move {
            let ch = self.terms.lock().remove(&term);
            if let Some(ch) = ch {
                ch.kill().await;
            }
            Ok(())
        })
    }
}
