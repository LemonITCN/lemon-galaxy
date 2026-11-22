//! AI：网关流式接口与工具执行器（把智能体的工具调用落到与被控设备的会话上）。

use crate::api::Api;
use crate::session::{PeerSession, TerminalChannel, TerminalSink};
use crate::ssh::{SshSession, SshTerminal};
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

/// SSH 终端的输出喂进本地屏幕模型，供 AI 读屏。
struct ScreenSink(Arc<Mutex<vt100::Parser>>);
impl TerminalSink for ScreenSink {
    fn on_output(&self, d: Vec<u8>) {
        self.0.lock().process(&d);
    }
    fn on_exit(&self, _c: Option<i32>) {}
}

/// SSH 主机的基本信息（连接后探测一次）。
#[derive(Clone, Debug, Default)]
pub struct SshMeta {
    pub os: String,
    pub user: String,
    pub shell: String,
}

#[derive(Clone)]
pub enum AiTarget {
    Device(Arc<PeerSession>),
    Ssh(Arc<SshSession>, SshMeta),
    /// 本机（桌面客户端进程内执行，不走网络）
    Local(Arc<dyn LocalHost>),
}

/// 本机的基本信息
#[derive(Clone, Debug)]
pub struct LocalMeta {
    pub platform: galaxy_proto::Platform,
    pub os: String,
    pub user: String,
    pub shell: String,
    pub gui: bool,
    pub computer_use: bool,
}

/// 本机执行后端：桌面客户端进程内直接执行（由 galaxy-ffi 用 galaxy-host 的同一套代码实现）。
pub trait LocalHost: Send + Sync {
    fn meta(&self) -> LocalMeta;
    fn exec(&self, req: ExecRequest, on_out: Box<dyn Fn(&[u8]) + Send + Sync>, cancel: watch::Receiver<bool>) -> BoxFut<'_, anyhow::Result<CmdResult>>;
    /// 文件操作：`upload` 是写入时的内容。返回服务端回的全部消息（读取时是 Size + Chunk… + End）。
    fn file(&self, op: FileOp, upload: Option<Vec<u8>>) -> BoxFut<'_, anyhow::Result<Vec<FileMsg>>>;
    fn displays(&self) -> Vec<DisplayInfo>;
    fn screenshot(&self, display: u32, max_w: u32, max_h: u32) -> BoxFut<'_, anyhow::Result<ImageData>>;
    fn ui_tree(&self, display: u32, max_nodes: u32) -> BoxFut<'_, anyhow::Result<String>>;
    fn inject(&self, display: u32, events: Vec<InputEvent>, origin: Origin) -> BoxFut<'_, anyhow::Result<()>>;
    fn term_open(&self, origin: Origin) -> BoxFut<'_, anyhow::Result<String>>;
    fn term_write(&self, id: &str, data: Vec<u8>) -> BoxFut<'_, anyhow::Result<()>>;
    fn term_snapshot(&self, id: &str) -> BoxFut<'_, anyhow::Result<String>>;
    fn term_close(&self, id: &str) -> BoxFut<'_, anyhow::Result<()>>;
    fn computer_use(&self, request: String, origin: Origin) -> BoxFut<'_, anyhow::Result<(String, Option<ImageData>)>>;
}

fn file_err(msgs: &[FileMsg]) -> Option<anyhow::Error> {
    msgs.iter().find_map(|m| match m {
        FileMsg::Error(e) => Some(anyhow::anyhow!("{}", e.message)),
        _ => None,
    })
}

enum AiTerm {
    Device(Arc<TerminalChannel>),
    Ssh(SshTerminal, Arc<Mutex<vt100::Parser>>),
    Local(Arc<dyn LocalHost>, String),
}

/// 主控端的工具执行器：目标 ID → 会话（设备或 SSH 主机）。
pub struct SessionExecutor {
    pub sessions: Mutex<HashMap<String, (AiTarget, String)>>,
    terms: Mutex<HashMap<String, Arc<AiTerm>>>,
}

impl SessionExecutor {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { sessions: Default::default(), terms: Default::default() })
    }

    /// 加入一个设备目标（设备会话 + 显示名称）。
    pub fn add(&self, id: String, s: Arc<PeerSession>, name: String) {
        self.sessions.lock().insert(id, (AiTarget::Device(s), name));
    }

    /// 加入一个 SSH 主机目标。先探测系统与 shell，给模型做参考。
    pub async fn add_ssh(&self, id: String, s: Arc<SshSession>, name: String) {
        let mut out = Vec::new();
        let _ = s.exec("uname -srm 2>/dev/null || ver; echo \"__lg__${USER:-$(whoami)}__${SHELL:-sh}\"", std::time::Duration::from_secs(10), |_, d| out.extend_from_slice(d), None).await;
        let text = String::from_utf8_lossy(&out).to_string();
        let mut meta = SshMeta { user: s.username.clone(), shell: "/bin/sh".into(), os: String::new() };
        for l in text.lines() {
            if let Some(r) = l.strip_prefix("__lg__") {
                let mut it = r.splitn(2, "__");
                meta.user = it.next().unwrap_or("").to_string();
                meta.shell = it.next().unwrap_or("/bin/sh").to_string();
            } else if meta.os.is_empty() && !l.trim().is_empty() {
                meta.os = l.trim().to_string();
            }
        }
        self.sessions.lock().insert(id, (AiTarget::Ssh(s, meta), name));
    }

    fn target(&self, target: &str) -> anyhow::Result<AiTarget> {
        self.sessions.lock().get(target).map(|x| x.0.clone()).ok_or_else(|| anyhow::anyhow!("目标未连接：{target}"))
    }

    /// 加入本机目标
    pub fn add_local(&self, id: String, l: Arc<dyn LocalHost>, name: String) {
        self.sessions.lock().insert(id, (AiTarget::Local(l), name));
    }

    pub fn has(&self, id: &str) -> bool {
        self.sessions.lock().contains_key(id)
    }

    fn sess(&self, target: &str) -> anyhow::Result<Arc<PeerSession>> {
        match self.target(target)? {
            AiTarget::Device(s) => Ok(s),
            AiTarget::Ssh(..) => anyhow::bail!("SSH 主机没有图形界面，不能截图或操作鼠标键盘"),
            AiTarget::Local(_) => anyhow::bail!("内部错误：本机目标不走会话"),
        }
    }
}

fn local_entries(msgs: Vec<FileMsg>) -> anyhow::Result<Vec<FileEntry>> {
    if let Some(err) = file_err(&msgs) {
        return Err(err);
    }
    Ok(msgs.into_iter().flat_map(|m| if let FileMsg::Entries(v) = m { v } else { vec![] }).collect())
}

fn e<E: std::fmt::Display>(x: E) -> anyhow::Error {
    anyhow::anyhow!("{x}")
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

impl Executor for SessionExecutor {
    fn targets(&self) -> Vec<TargetInfo> {
        self.sessions
            .lock()
            .iter()
            .map(|(id, (t, name))| match t {
                AiTarget::Device(s) => {
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
                        computer_use: s.ack.permissions.control && s.ack.capabilities.iter().any(|c| c == CAP_COMPUTER_USE),
                    }
                }
                AiTarget::Local(l) => {
                    let m = l.meta();
                    TargetInfo { id: id.clone(), name: name.clone(), kind: TargetKind::Device, platform: m.platform, os: m.os, user: m.user, shell: m.shell, gui: m.gui, ai_cap: AiMode::FullTrust, computer_use: m.computer_use }
                }
                AiTarget::Ssh(s, m) => TargetInfo {
                    id: id.clone(),
                    name: name.clone(),
                    kind: TargetKind::SshHost,
                    platform: galaxy_proto::Platform::Linux,
                    os: if m.os.is_empty() { format!("SSH {}:{}", s.host, s.port) } else { m.os.clone() },
                    user: m.user.clone(),
                    shell: m.shell.clone(),
                    gui: false,
                    // SSH 主机没有被控端设上限，按用户自己的档位走
                    ai_cap: AiMode::FullTrust,
                    computer_use: false,
                },
            })
            .collect()
    }

    fn run_command(&self, target: &str, cmd: &str, cwd: Option<String>, timeout_s: u64, shell: Option<String>, origin: Origin, on_out: Box<dyn Fn(&str) + Send + Sync>, cancel: watch::Receiver<bool>) -> BoxFut<'_, anyhow::Result<CmdResult>> {
        let (target, cmd) = (target.to_string(), cmd.to_string());
        Box::pin(async move {
            match self.target(&target)? {
                AiTarget::Device(s) => {
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
                }
                AiTarget::Local(l) => {
                    let req = ExecRequest { command: cmd, shell, cwd, env: vec![], timeout_ms: timeout_s * 1000, origin, max_output: 4 * 1024 * 1024 };
                    l.exec(req, Box::new(move |d| on_out(&String::from_utf8_lossy(d))), cancel).await
                }
                AiTarget::Ssh(s, _) => {
                    let full = match cwd.filter(|c| !c.is_empty()) {
                        Some(c) => format!("cd {} && {}", shell_quote(&c), cmd),
                        None => cmd,
                    };
                    let (mut so, mut se, mut truncated) = (Vec::new(), Vec::new(), false);
                    const MAX: usize = 4 * 1024 * 1024;
                    let (code, timed_out, ms) = s
                        .exec(
                            &full,
                            std::time::Duration::from_secs(timeout_s.max(1)),
                            |is_err, d| {
                                on_out(&String::from_utf8_lossy(d));
                                let b = if is_err { &mut se } else { &mut so };
                                if b.len() + d.len() > MAX {
                                    truncated = true;
                                } else {
                                    b.extend_from_slice(d);
                                }
                            },
                            Some(cancel),
                        )
                        .await
                        .map_err(e)?;
                    Ok(CmdResult { code, stdout: String::from_utf8_lossy(&so).to_string(), stderr: String::from_utf8_lossy(&se).to_string(), timed_out, truncated, duration_ms: ms })
                }
            }
        })
    }

    fn read_file(&self, target: &str, path: &str, max_bytes: u64) -> BoxFut<'_, anyhow::Result<Vec<u8>>> {
        let (target, path) = (target.to_string(), path.to_string());
        Box::pin(async move {
            match self.target(&target)? {
                AiTarget::Device(s) => s.fs_read_all(&path, max_bytes).await.map_err(e),
                AiTarget::Ssh(s, _) => s.fs_read_all(&path, max_bytes).await.map_err(e),
                AiTarget::Local(l) => {
                    let msgs = l.file(FileOp::Read { path, offset: 0, len: Some(max_bytes) }, None).await?;
                    if let Some(err) = file_err(&msgs) {
                        return Err(err);
                    }
                    Ok(msgs.into_iter().flat_map(|m| if let FileMsg::Chunk(d) = m { d } else { vec![] }).collect())
                }
            }
        })
    }

    fn exists(&self, target: &str, path: &str) -> BoxFut<'_, anyhow::Result<bool>> {
        let (target, path) = (target.to_string(), path.to_string());
        Box::pin(async move {
            match self.target(&target)? {
                AiTarget::Device(s) => Ok(s.fs_stat(&path).await.is_ok()),
                AiTarget::Ssh(s, _) => s.fs_exists(&path).await.map_err(e),
                AiTarget::Local(l) => Ok(file_err(&l.file(FileOp::Stat { path }, None).await?).is_none()),
            }
        })
    }

    fn write_file(&self, target: &str, path: &str, data: Vec<u8>, expect: Option<Vec<u8>>, origin: Origin) -> BoxFut<'_, anyhow::Result<Option<String>>> {
        let (target, path) = (target.to_string(), path.to_string());
        Box::pin(async move {
            match self.target(&target)? {
                AiTarget::Device(s) => s.fs_write(&path, 0, true, None, expect, origin, &data).await.map(|x| x.1).map_err(e),
                AiTarget::Local(l) => {
                    let msgs = l.file(FileOp::Write { path, offset: 0, truncate: true, mode: None, expect_sha256: expect, origin }, Some(data)).await?;
                    if let Some(err) = file_err(&msgs) {
                        return Err(err);
                    }
                    Ok(msgs.into_iter().find_map(|m| if let FileMsg::Written { backup, .. } = m { backup } else { None }))
                }
                AiTarget::Ssh(s, _) => {
                    if let Some(exp) = expect {
                        let cur = s.fs_read_all(&path, 64 * 1024 * 1024).await.map(|b| galaxy_crypto::sha256(&b).to_vec()).unwrap_or_default();
                        if cur != exp {
                            anyhow::bail!("文件已被其他人修改，请重新读取后再改");
                        }
                    }
                    s.fs_write_all(&path, &data).await.map_err(e)?;
                    Ok(None)
                }
            }
        })
    }

    fn list_dir(&self, target: &str, path: &str) -> BoxFut<'_, anyhow::Result<Vec<FileEntry>>> {
        let (target, path) = (target.to_string(), path.to_string());
        Box::pin(async move {
            match self.target(&target)? {
                AiTarget::Device(s) => s.fs_list(&path).await.map_err(e),
                AiTarget::Ssh(s, _) => s.fs_list(&path).await.map_err(e),
                AiTarget::Local(l) => local_entries(l.file(FileOp::List { path }, None).await?),
            }
        })
    }

    fn search(&self, target: &str, root: &str, pattern: &str, content: bool) -> BoxFut<'_, anyhow::Result<Vec<FileEntry>>> {
        let (target, root, pattern) = (target.to_string(), root.to_string(), pattern.to_string());
        Box::pin(async move {
            match self.target(&target)? {
                AiTarget::Device(s) => s.fs_search(&root, &pattern, content, 200).await.map_err(e),
                AiTarget::Ssh(s, _) => s.fs_search(&root, &pattern, content, 200).await.map_err(e),
                AiTarget::Local(l) => local_entries(l.file(FileOp::Search { root, pattern, content, max: 200 }, None).await?),
            }
        })
    }

    fn displays(&self, target: &str) -> BoxFut<'_, anyhow::Result<Vec<DisplayInfo>>> {
        let target = target.to_string();
        Box::pin(async move {
            if let AiTarget::Local(l) = self.target(&target)? {
                return Ok(l.displays());
            }
            self.sess(&target)?.list_displays().await.map_err(e)
        })
    }

    fn screenshot(&self, target: &str, display: u32, max_w: u32, max_h: u32) -> BoxFut<'_, anyhow::Result<ImageData>> {
        let target = target.to_string();
        Box::pin(async move {
            if let AiTarget::Local(l) = self.target(&target)? {
                return l.screenshot(display, max_w, max_h).await;
            }
            self.sess(&target)?.screenshot(display, max_w, max_h).await.map_err(e)
        })
    }

    fn ui_tree(&self, target: &str, display: u32, max_nodes: u32) -> BoxFut<'_, anyhow::Result<String>> {
        let target = target.to_string();
        Box::pin(async move {
            if let AiTarget::Local(l) = self.target(&target)? {
                return l.ui_tree(display, max_nodes).await;
            }
            match self.sess(&target)?.mgmt(MgmtReq::UiTree { display, max_nodes }).await.map_err(e)? {
                MgmtResp::Text(t) => Ok(t),
                _ => anyhow::bail!("应答错误"),
            }
        })
    }

    fn inject(&self, target: &str, display: u32, events: Vec<InputEvent>, origin: Origin) -> BoxFut<'_, anyhow::Result<()>> {
        let target = target.to_string();
        Box::pin(async move {
            if let AiTarget::Local(l) = self.target(&target)? {
                return l.inject(display, events, origin).await;
            }
            self.sess(&target)?.mgmt(MgmtReq::Inject { display, events, origin }).await.map(|_| ()).map_err(e)
        })
    }

    fn terminal_open(&self, target: &str, origin: Origin) -> BoxFut<'_, anyhow::Result<String>> {
        let target = target.to_string();
        Box::pin(async move {
            match self.target(&target)? {
                AiTarget::Device(s) => {
                    let ch = s.open_terminal(TerminalOpen { attach: None, shell: None, cols: 120, rows: 36, env: vec![], origin }, Arc::new(NullSink)).await.map_err(e)?;
                    let id = ch.session_id.clone();
                    self.terms.lock().insert(id.clone(), Arc::new(AiTerm::Device(ch)));
                    Ok(id)
                }
                AiTarget::Local(l) => {
                    let tid = l.term_open(origin).await?;
                    let id = format!("local-{tid}");
                    self.terms.lock().insert(id.clone(), Arc::new(AiTerm::Local(l, tid)));
                    Ok(id)
                }
                AiTarget::Ssh(s, _) => {
                    let screen = Arc::new(Mutex::new(vt100::Parser::new(std::num::NonZeroU16::new(36).unwrap(), std::num::NonZeroU16::new(120).unwrap(), 2000)));
                    let t = s.open_terminal(120, 36, &[("TERM".into(), "xterm-256color".into())], Arc::new(ScreenSink(screen.clone()))).await.map_err(e)?;
                    let id = format!("ssh-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
                    self.terms.lock().insert(id.clone(), Arc::new(AiTerm::Ssh(t, screen)));
                    Ok(id)
                }
            }
        })
    }

    fn terminal_send(&self, term: &str, data: Vec<u8>) -> BoxFut<'_, anyhow::Result<()>> {
        let term = term.to_string();
        Box::pin(async move {
            let t = self.terms.lock().get(&term).cloned().ok_or_else(|| anyhow::anyhow!("终端不存在"))?;
            match t.as_ref() {
                AiTerm::Device(ch) => ch.write(data).await.map_err(e),
                AiTerm::Ssh(t, _) => {
                    t.write(data);
                    Ok(())
                }
                AiTerm::Local(l, id) => l.term_write(id, data).await,
            }
        })
    }

    fn terminal_read(&self, term: &str) -> BoxFut<'_, anyhow::Result<String>> {
        let term = term.to_string();
        Box::pin(async move {
            let t = self.terms.lock().get(&term).cloned().ok_or_else(|| anyhow::anyhow!("终端不存在"))?;
            match t.as_ref() {
                AiTerm::Device(ch) => ch.snapshot().await.map_err(e),
                AiTerm::Ssh(_, screen) => Ok(screen.lock().screen().contents()),
                AiTerm::Local(l, id) => l.term_snapshot(id).await,
            }
        })
    }

    fn computer_use(&self, target: &str, request: String, origin: Origin) -> BoxFut<'_, anyhow::Result<(String, Option<ImageData>)>> {
        let target = target.to_string();
        Box::pin(async move {
            match self.target(&target)? {
                AiTarget::Local(l) => l.computer_use(request, origin).await,
                AiTarget::Ssh(..) => anyhow::bail!("SSH 主机没有图形界面"),
                AiTarget::Device(s) => match s.mgmt(MgmtReq::ComputerUse { request, origin }).await.map_err(e)? {
                    MgmtResp::ComputerUse { text, image } => Ok((text, image)),
                    _ => anyhow::bail!("应答错误"),
                },
            }
        })
    }

    fn terminal_close(&self, term: &str) -> BoxFut<'_, anyhow::Result<()>> {
        let term = term.to_string();
        Box::pin(async move {
            let t = self.terms.lock().remove(&term);
            match t.as_deref() {
                Some(AiTerm::Device(ch)) => ch.kill().await,
                Some(AiTerm::Ssh(t, _)) => t.close(),
                Some(AiTerm::Local(l, id)) => l.term_close(id).await?,
                None => {}
            }
            Ok(())
        })
    }
}
