//! SSH 直连会话（加密库里的 ssh_host）：终端、文件（SFTP）、命令、端口转发。
//! 终端与文件复用设备会话的 TerminalChannel / FileClient / Transfer / PortForward，界面不用区分来源。

use crate::callbacks::*;
use crate::session::{FileClient, FsBackend, PortForward, TerminalChannel};
use crate::types::*;
use crate::{run, Result};
use galaxy_client::ssh as gs;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Duration;

pub(crate) struct PromptBridge(pub Arc<dyn SecretPrompt>);

impl gs::SshPrompt for PromptBridge {
    fn ask(&self, prompt: &str, echo: bool) -> Option<String> {
        self.0.ask(prompt.to_string(), echo)
    }
    fn confirm_host_key(&self, host: &str, port: u16, key_type: &str, fingerprint: &str, changed: bool) -> bool {
        self.0.confirm_host_key(host.to_string(), port, key_type.to_string(), fingerprint.to_string(), changed)
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct SshInfo {
    /// 加密库条目 id
    pub host_id: String,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    /// 怎么连过去的：direct / device:<设备名> / ssh:<跳转主机名>
    pub via: String,
}

#[derive(uniffi::Object)]
pub struct SshSession {
    pub(crate) inner: Arc<gs::SshSession>,
    pub(crate) info: SshInfo,
}

#[uniffi::export]
impl SshSession {
    pub fn info(&self) -> SshInfo {
        self.info.clone()
    }

    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    /// 打开终端。SSH 没有断线继承：detach 与 kill 都会结束这个终端。
    pub async fn open_terminal(&self, options: TerminalOptions, sink: Arc<dyn TerminalSink>) -> Result<Arc<TerminalChannel>> {
        let s = self.inner.clone();
        run(async move {
            struct Sink(Arc<dyn TerminalSink>);
            impl galaxy_client::session::TerminalSink for Sink {
                fn on_output(&self, data: Vec<u8>) {
                    self.0.on_output(data)
                }
                fn on_exit(&self, code: Option<i32>) {
                    self.0.on_exit(code)
                }
            }
            let env: Vec<(String, String)> = options.env.into_iter().map(|e| (e.name, e.value)).collect();
            let t = s.open_terminal(options.cols, options.rows, &env, Arc::new(Sink(sink))).await?;
            Ok(TerminalChannel::from_ssh(t))
        })
        .await
    }

    /// 文件（SFTP）。
    pub fn files(&self) -> Arc<FileClient> {
        Arc::new(FileClient { b: FsBackend::Ssh(self.inner.clone()) })
    }

    pub async fn exec(&self, request: ExecRequest) -> Result<ExecResult> {
        let s = self.inner.clone();
        run(async move {
            let max = if request.max_output == 0 { 4 << 20 } else { request.max_output } as usize;
            let timeout = Duration::from_millis(if request.timeout_ms == 0 { 120_000 } else { request.timeout_ms });
            let cmd = match request.cwd.filter(|c| !c.is_empty()) {
                Some(c) => format!("cd {} && {}", shell_quote(&c), request.command),
                None => request.command,
            };
            let (mut out, mut err, mut truncated) = (Vec::new(), Vec::new(), false);
            let (code, timed_out, ms) = s
                .exec(
                    &cmd,
                    timeout,
                    |is_err, d| {
                        let buf = if is_err { &mut err } else { &mut out };
                        if buf.len() + d.len() > max {
                            truncated = true;
                            let room = max.saturating_sub(buf.len());
                            buf.extend_from_slice(&d[..room.min(d.len())]);
                        } else {
                            buf.extend_from_slice(d);
                        }
                    },
                    None,
                )
                .await?;
            Ok(ExecResult { code, stdout: String::from_utf8_lossy(&out).to_string(), stderr: String::from_utf8_lossy(&err).to_string(), timed_out, truncated, duration_ms: ms })
        })
        .await
    }

    /// 本地端口转发（在本机 127.0.0.1 监听，经 SSH 连到 remote_host:remote_port；local_port 为 0 时自动选）。
    pub async fn forward_port(&self, local_port: u16, remote_host: String, remote_port: u16) -> Result<Arc<PortForward>> {
        let s = self.inner.clone();
        run(async move {
            let (port, h) = s.forward_local(local_port, remote_host, remote_port).await?;
            Ok(Arc::new(PortForward { port, handle: Mutex::new(Some(h)) }))
        })
        .await
    }

    pub fn disconnect(&self) {
        let s = self.inner.clone();
        crate::rt().spawn(async move { s.disconnect().await });
    }
}

/// POSIX shell 单引号转义。
pub(crate) fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
