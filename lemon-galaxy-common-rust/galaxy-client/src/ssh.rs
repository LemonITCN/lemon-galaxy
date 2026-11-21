//! SSH 直连（加密库里的 ssh_host）：终端、SFTP 文件、命令执行、本地端口转发。
//!
//! 传输可以是直连 TCP，也可以经某台 LemonGalaxy 设备（被控端代为连接目标主机，适合目标在对方内网），
//! 或经另一台 SSH 主机跳转（direct-tcpip）。主机密钥按加密库里的 known_host 校验，首次连接或变化时问用户。

use crate::error::{ClientError, Result};
use crate::session::{PeerSession, TerminalSink, TransferProgress};
use galaxy_proto::peer::{FileEntry, FileKind};
use parking_lot::Mutex;
use russh::client::{self, Handle, KeyboardInteractiveAuthResponse};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::ChannelMsg;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileAttributes, OpenFlags};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::{mpsc, watch};

/// 向用户要东西（在内核线程上同步调用，界面可以阻塞等用户输入）。
pub trait SshPrompt: Send + Sync {
    /// 密码、私钥口令、键盘交互的回答；None 表示用户取消。
    fn ask(&self, prompt: &str, echo: bool) -> Option<String>;
    /// 主机密钥确认。`changed` 为真表示和记录的不一致（可能被冒充）。
    fn confirm_host_key(&self, host: &str, port: u16, key_type: &str, fingerprint: &str, changed: bool) -> bool;
}

#[derive(Clone, Debug)]
pub enum SshAuthMethod {
    /// None 时连接时再问
    Password(Option<String>),
    /// 私钥（OpenSSH / PEM 文本）与口令（None 且私钥加密时再问）
    PrivateKey { key: String, passphrase: Option<String> },
    Interactive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownHostKey {
    pub key_type: String,
    pub fingerprint: String,
}

pub enum SshVia {
    Direct,
    /// 经 LemonGalaxy 设备转发（被控端打开到目标的 TCP 连接）
    Device(Arc<PeerSession>),
    /// 经另一台 SSH 主机跳转
    Ssh(Arc<SshSession>),
}

pub struct SshTarget {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: SshAuthMethod,
    pub via: SshVia,
    /// 加密库里记录的主机密钥
    pub known: Option<KnownHostKey>,
}

struct Handler {
    host: String,
    port: u16,
    known: Option<KnownHostKey>,
    prompt: Arc<dyn SshPrompt>,
    /// 校验通过后的主机密钥（新的或变了的要存回加密库）
    seen: Arc<Mutex<Option<KnownHostKey>>>,
}

impl client::Handler for Handler {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> std::result::Result<bool, Self::Error> {
        let pk = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(c) => c.public_key().clone().into(),
        };
        let fp = pk.fingerprint(HashAlg::Sha256).to_string();
        let kt = pk.algorithm().as_str().to_string();
        let now = KnownHostKey { key_type: kt.clone(), fingerprint: fp.clone() };
        if self.known.as_ref() == Some(&now) {
            *self.seen.lock() = Some(now);
            return Ok(true);
        }
        let changed = self.known.is_some();
        let (p, h, port) = (self.prompt.clone(), self.host.clone(), self.port);
        let ok = tokio::task::spawn_blocking(move || p.confirm_host_key(&h, port, &kt, &fp, changed)).await.unwrap_or(false);
        if ok {
            *self.seen.lock() = Some(now);
        }
        Ok(ok)
    }
}

pub struct SshSession {
    handle: Handle<Handler>,
    pub host: String,
    pub port: u16,
    pub username: String,
    sftp: tokio::sync::Mutex<Option<Arc<SftpSession>>>,
    home: Mutex<Option<String>>,
    /// 跳转用的上一级连接，保持存活
    _via: Option<SshViaHold>,
}

enum SshViaHold {
    Device(Arc<PeerSession>),
    Ssh(Arc<SshSession>),
}

fn net(e: impl std::fmt::Display) -> ClientError {
    ClientError::Network(e.to_string())
}

fn sftp_err(e: russh_sftp::client::error::Error) -> ClientError {
    let s = e.to_string();
    let l = s.to_lowercase();
    if l.contains("no such file") {
        ClientError::NotFound("文件或目录不存在".into())
    } else if l.contains("permission denied") {
        ClientError::Permission("没有权限".into())
    } else {
        ClientError::Other(s)
    }
}

/// 连接结果：会话，以及需要存回加密库的主机密钥（首次连接或变化后被用户确认时）。
pub struct SshConnected {
    pub session: Arc<SshSession>,
    pub new_host_key: Option<KnownHostKey>,
}

impl SshSession {
    pub async fn connect(t: SshTarget, prompt: Arc<dyn SshPrompt>) -> Result<SshConnected> {
        let config = Arc::new(client::Config {
            inactivity_timeout: Some(Duration::from_secs(3600)),
            keepalive_interval: Some(Duration::from_secs(15)),
            keepalive_max: 4,
            nodelay: true,
            ..Default::default()
        });
        let seen = Arc::new(Mutex::new(None));
        let handler = Handler { host: t.host.clone(), port: t.port, known: t.known.clone(), prompt: prompt.clone(), seen: seen.clone() };
        let (mut handle, hold) = match t.via {
            SshVia::Direct => {
                let fut = client::connect(config, (t.host.as_str(), t.port), handler);
                let h = tokio::time::timeout(Duration::from_secs(20), fut).await.map_err(|_| ClientError::Network(format!("连接 {}:{} 超时", t.host, t.port)))?;
                (h.map_err(|e| connect_err(e, &t.host, t.port))?, None)
            }
            SshVia::Device(peer) => {
                let (send, recv) = peer.open_tcp(&t.host, t.port).await?;
                let stream = tokio::io::join(recv, send);
                (client::connect_stream(config, stream, handler).await.map_err(|e| connect_err(e, &t.host, t.port))?, Some(SshViaHold::Device(peer)))
            }
            SshVia::Ssh(jump) => {
                let ch = jump.handle.channel_open_direct_tcpip(t.host.clone(), t.port as u32, "127.0.0.1", 0).await.map_err(|e| ClientError::Network(format!("跳转主机无法连到 {}:{}：{e}", t.host, t.port)))?;
                (client::connect_stream(config, ch.into_stream(), handler).await.map_err(|e| connect_err(e, &t.host, t.port))?, Some(SshViaHold::Ssh(jump)))
            }
        };
        authenticate(&mut handle, &t.username, &t.auth, &prompt).await?;
        let new_host_key = seen.lock().clone().filter(|k| t.known.as_ref() != Some(k));
        Ok(SshConnected {
            session: Arc::new(SshSession { handle, host: t.host, port: t.port, username: t.username, sftp: tokio::sync::Mutex::new(None), home: Mutex::new(None), _via: hold }),
            new_host_key,
        })
    }

    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    pub async fn disconnect(&self) {
        let _ = self.handle.disconnect(russh::Disconnect::ByApplication, "", "en").await;
    }

    // ---------------- 终端 ----------------

    pub async fn open_terminal(self: &Arc<Self>, cols: u16, rows: u16, env: &[(String, String)], sink: Arc<dyn TerminalSink>) -> Result<SshTerminal> {
        let ch = self.handle.channel_open_session().await.map_err(net)?;
        for (k, v) in env {
            // 服务器一般只接受 LANG / LC_* 等白名单变量，失败不影响
            let _ = ch.set_env(false, k.clone(), v.clone()).await;
        }
        let term = env.iter().find(|(k, _)| k == "TERM").map(|(_, v)| v.as_str()).unwrap_or("xterm-256color");
        ch.request_pty(true, term, cols.max(10) as u32, rows.max(2) as u32, 0, 0, &[]).await.map_err(net)?;
        ch.request_shell(true).await.map_err(net)?;
        let (mut read, write) = ch.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<TermOp>();
        tokio::spawn(async move {
            let mut code = None;
            while let Some(m) = read.wait().await {
                match m {
                    ChannelMsg::Data { data } => sink.on_output(data.to_vec()),
                    ChannelMsg::ExtendedData { data, .. } => sink.on_output(data.to_vec()),
                    ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status as i32),
                    ChannelMsg::Close => break,
                    _ => {}
                }
            }
            sink.on_exit(code);
        });
        tokio::spawn(async move {
            while let Some(op) = rx.recv().await {
                let r = match op {
                    TermOp::Write(d) => write.data_bytes(d).await,
                    TermOp::Resize(c, r) => write.window_change(c.max(10) as u32, r.max(2) as u32, 0, 0).await,
                    TermOp::Close => {
                        let _ = write.eof().await;
                        let _ = write.close().await;
                        break;
                    }
                };
                if r.is_err() {
                    break;
                }
            }
        });
        Ok(SshTerminal { tx, title: format!("{}@{}", self.username, self.host) })
    }

    // ---------------- 命令 ----------------

    /// 执行命令，`on_out(是否错误输出, 数据)` 流式回调。返回 (退出码, 是否超时, 耗时毫秒)。
    pub async fn exec<F: FnMut(bool, &[u8])>(&self, command: &str, timeout: Duration, mut on_out: F, mut cancel: Option<watch::Receiver<bool>>) -> Result<(Option<i32>, bool, u64)> {
        let started = Instant::now();
        let mut ch = self.handle.channel_open_session().await.map_err(net)?;
        ch.exec(true, command.as_bytes().to_vec()).await.map_err(net)?;
        let _ = ch.eof().await;
        let deadline = tokio::time::Instant::now() + timeout;
        let mut code = None;
        let mut timed_out = false;
        loop {
            let cancelled = async {
                match cancel.as_mut() {
                    Some(c) => {
                        let _ = c.wait_for(|v| *v).await;
                    }
                    None => futures::future::pending::<()>().await,
                }
            };
            tokio::select! {
                m = ch.wait() => match m {
                    Some(ChannelMsg::Data { data }) => on_out(false, &data),
                    Some(ChannelMsg::ExtendedData { data, .. }) => on_out(true, &data),
                    Some(ChannelMsg::ExitStatus { exit_status }) => code = Some(exit_status as i32),
                    Some(ChannelMsg::Close) | None => break,
                    _ => {}
                },
                _ = tokio::time::sleep_until(deadline) => {
                    timed_out = true;
                    let _ = ch.signal(russh::Sig::KILL).await;
                    let _ = ch.close().await;
                    break;
                }
                _ = cancelled => {
                    let _ = ch.signal(russh::Sig::KILL).await;
                    let _ = ch.close().await;
                    return Err(ClientError::Cancelled);
                }
            }
        }
        Ok((code, timed_out, started.elapsed().as_millis() as u64))
    }

    // ---------------- 端口转发 ----------------

    /// 本地端口转发：在 127.0.0.1:local_port（0 为自动）监听，每个连接经 SSH 连到 host:port。
    pub async fn forward_local(self: &Arc<Self>, local_port: u16, host: String, port: u16) -> Result<(u16, tokio::task::JoinHandle<()>)> {
        let l = tokio::net::TcpListener::bind(("127.0.0.1", local_port)).await.map_err(net)?;
        let bound = l.local_addr().map_err(net)?.port();
        let me = self.clone();
        let h = tokio::spawn(async move {
            while let Ok((mut sock, peer)) = l.accept().await {
                let (me, host) = (me.clone(), host.clone());
                tokio::spawn(async move {
                    let Ok(ch) = me.handle.channel_open_direct_tcpip(host, port as u32, peer.ip().to_string(), peer.port() as u32).await else { return };
                    let mut s = ch.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut sock, &mut s).await;
                });
            }
        });
        Ok((bound, h))
    }

    // ---------------- SFTP ----------------

    async fn sftp(&self) -> Result<Arc<SftpSession>> {
        let mut g = self.sftp.lock().await;
        if let Some(s) = g.as_ref() {
            return Ok(s.clone());
        }
        let ch = self.handle.channel_open_session().await.map_err(net)?;
        ch.request_subsystem(true, "sftp").await.map_err(net)?;
        let s = Arc::new(SftpSession::new(ch.into_stream()).await.map_err(|e| ClientError::Protocol(format!("对方不支持 SFTP：{e}")))?);
        *g = Some(s.clone());
        Ok(s)
    }

    pub async fn fs_home(&self) -> Result<String> {
        if let Some(h) = self.home.lock().clone() {
            return Ok(h);
        }
        let h = self.sftp().await?.canonicalize(".").await.map_err(sftp_err)?;
        *self.home.lock() = Some(h.clone());
        Ok(h)
    }

    async fn resolve(&self, p: &str) -> Result<String> {
        let p = p.trim();
        if p.is_empty() || p == "~" {
            return self.fs_home().await;
        }
        if let Some(r) = p.strip_prefix("~/") {
            return Ok(format!("{}/{r}", self.fs_home().await?.trim_end_matches('/')));
        }
        if p.starts_with('/') {
            return Ok(p.to_string());
        }
        Ok(format!("{}/{p}", self.fs_home().await?.trim_end_matches('/')))
    }

    pub async fn fs_list(&self, path: &str) -> Result<Vec<FileEntry>> {
        let dir = self.resolve(path).await?;
        let s = self.sftp().await?;
        let rd = s.read_dir(dir.clone()).await.map_err(sftp_err)?;
        let mut v: Vec<FileEntry> = Vec::new();
        for e in rd {
            let name = e.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let full = join(&dir, &name);
            let md = e.metadata();
            let mut fe = entry(&name, &full, &md);
            if md.is_symlink() {
                if let Ok(t) = s.metadata(full.clone()).await {
                    if t.is_dir() {
                        fe.kind = FileKind::Dir;
                    }
                }
                fe.link_target = s.read_link(full).await.ok();
            }
            v.push(fe);
        }
        v.sort_by(|a, b| (b.kind == FileKind::Dir).cmp(&(a.kind == FileKind::Dir)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(v)
    }

    pub async fn fs_stat(&self, path: &str) -> Result<FileEntry> {
        let p = self.resolve(path).await?;
        let s = self.sftp().await?;
        let md = s.metadata(p.clone()).await.map_err(sftp_err)?;
        let name = p.rsplit('/').find(|x| !x.is_empty()).unwrap_or("/").to_string();
        Ok(entry(&name, &p, &md))
    }

    pub async fn fs_realpath(&self, path: &str) -> Result<String> {
        let p = self.resolve(path).await?;
        self.sftp().await?.canonicalize(p).await.map_err(sftp_err)
    }

    pub async fn fs_mkdir(&self, path: &str) -> Result<()> {
        let p = self.resolve(path).await?;
        let s = self.sftp().await?;
        // 逐级创建
        let mut cur = String::new();
        for part in p.split('/').filter(|x| !x.is_empty()) {
            cur = format!("{cur}/{part}");
            if s.try_exists(cur.clone()).await.unwrap_or(false) {
                continue;
            }
            s.create_dir(cur.clone()).await.map_err(sftp_err)?;
        }
        Ok(())
    }

    pub async fn fs_rename(&self, from: &str, to: &str) -> Result<()> {
        let (a, b) = (self.resolve(from).await?, self.resolve(to).await?);
        let s = self.sftp().await?;
        if s.try_exists(b.clone()).await.unwrap_or(false) {
            return Err(ClientError::Other("目标已存在".into()));
        }
        s.rename(a, b).await.map_err(sftp_err)
    }

    pub async fn fs_remove(&self, path: &str, recursive: bool) -> Result<()> {
        let p = self.resolve(path).await?;
        let s = self.sftp().await?;
        let md = s.symlink_metadata(p.clone()).await.map_err(sftp_err)?;
        if md.is_dir() {
            if recursive {
                remove_tree(&s, &p, 0).await
            } else {
                s.remove_dir(p).await.map_err(sftp_err)
            }
        } else {
            s.remove_file(p).await.map_err(sftp_err)
        }
    }

    pub async fn fs_chmod(&self, path: &str, mode: u32) -> Result<()> {
        let p = self.resolve(path).await?;
        let mut a = FileAttributes::empty();
        a.permissions = Some(mode & 0o7777);
        self.sftp().await?.set_metadata(p, a).await.map_err(sftp_err)
    }

    /// 按文件名搜索（SFTP 没法在服务器端搜内容，content 为真时只搜 2 MB 以内的文本文件）。
    pub async fn fs_search(&self, root: &str, pattern: &str, content: bool, max: usize) -> Result<Vec<FileEntry>> {
        let root = self.resolve(root).await?;
        let s = self.sftp().await?;
        let pat = pattern.to_lowercase();
        let mut out = vec![];
        let mut stack = vec![(root, 0u32)];
        let mut visited = 0usize;
        while let Some((dir, depth)) = stack.pop() {
            if out.len() >= max || visited > 20_000 {
                break;
            }
            let Ok(rd) = s.read_dir(dir.clone()).await else { continue };
            for e in rd {
                visited += 1;
                let name = e.file_name();
                if name == "." || name == ".." {
                    continue;
                }
                let full = join(&dir, &name);
                let md = e.metadata();
                let mut hit = name.to_lowercase().contains(&pat);
                if !hit && content && md.is_regular() && md.size.unwrap_or(0) < 2 * 1024 * 1024 {
                    hit = s.read(full.clone()).await.map(|b| String::from_utf8_lossy(&b).to_lowercase().contains(&pat)).unwrap_or(false);
                }
                if hit {
                    out.push(entry(&name, &full, &md));
                    if out.len() >= max {
                        break;
                    }
                }
                if md.is_dir() && depth < 8 && !name.starts_with('.') && name != "node_modules" && name != "proc" && name != "sys" {
                    stack.push((full, depth + 1));
                }
            }
        }
        Ok(out)
    }

    pub async fn fs_read_all(&self, path: &str, max: u64) -> Result<Vec<u8>> {
        let p = self.resolve(path).await?;
        let s = self.sftp().await?;
        let mut f = s.open(p).await.map_err(sftp_err)?;
        let mut buf = Vec::new();
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            let n = f.read(&mut chunk).await.map_err(|e| ClientError::Other(e.to_string()))?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.len() as u64 > max {
                return Err(ClientError::Other(format!("文件超过 {} 字节", max)));
            }
        }
        Ok(buf)
    }

    pub async fn fs_write_all(&self, path: &str, data: &[u8]) -> Result<()> {
        let p = self.resolve(path).await?;
        let s = self.sftp().await?;
        let mut f = s.open_with_flags(p, OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE).await.map_err(sftp_err)?;
        f.write_all(data).await.map_err(|e| ClientError::Other(e.to_string()))?;
        f.shutdown().await.map_err(|e| ClientError::Other(e.to_string()))?;
        Ok(())
    }

    pub async fn fs_exists(&self, path: &str) -> Result<bool> {
        let p = self.resolve(path).await?;
        Ok(self.sftp().await?.try_exists(p).await.unwrap_or(false))
    }

    /// 下载（本地已有部分则续传）。
    pub async fn download(&self, remote: &str, local: &std::path::Path, progress: Arc<dyn TransferProgress>, mut cancel: watch::Receiver<bool>) -> Result<()> {
        let p = self.resolve(remote).await?;
        let s = self.sftp().await?;
        let total = s.metadata(p.clone()).await.map_err(sftp_err)?.size.unwrap_or(0);
        let have = std::fs::metadata(local).map(|m| m.len()).unwrap_or(0);
        let start = if have > 0 && have <= total { have } else { 0 };
        let mut lf = tokio::fs::OpenOptions::new().create(true).write(true).truncate(start == 0).open(local).await.map_err(|e| ClientError::Other(e.to_string()))?;
        lf.seek(std::io::SeekFrom::Start(start)).await.map_err(|e| ClientError::Other(e.to_string()))?;
        let mut rf = s.open(p).await.map_err(sftp_err)?;
        rf.seek(std::io::SeekFrom::Start(start)).await.map_err(|e| ClientError::Other(e.to_string()))?;
        let mut done = start;
        let mut buf = vec![0u8; 256 * 1024];
        let mut last = Instant::now();
        loop {
            if *cancel.borrow_and_update() {
                return Err(ClientError::Cancelled);
            }
            let n = rf.read(&mut buf).await.map_err(|e| ClientError::Network(e.to_string()))?;
            if n == 0 {
                break;
            }
            lf.write_all(&buf[..n]).await.map_err(|e| ClientError::Other(e.to_string()))?;
            done += n as u64;
            if last.elapsed() > Duration::from_millis(200) {
                last = Instant::now();
                progress.on_progress(done, total);
            }
        }
        lf.flush().await.map_err(|e| ClientError::Other(e.to_string()))?;
        progress.on_progress(done, total);
        Ok(())
    }

    /// 上传（远端已有部分则续传）。
    pub async fn upload(&self, local: &std::path::Path, remote: &str, progress: Arc<dyn TransferProgress>, mut cancel: watch::Receiver<bool>) -> Result<()> {
        let p = self.resolve(remote).await?;
        let s = self.sftp().await?;
        let total = std::fs::metadata(local).map_err(|e| ClientError::Other(e.to_string()))?.len();
        let have = s.metadata(p.clone()).await.ok().and_then(|m| m.size).unwrap_or(0);
        let start = if have > 0 && have <= total { have } else { 0 };
        let flags = if start == 0 { OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE } else { OpenFlags::WRITE };
        let mut rf = s.open_with_flags(p, flags).await.map_err(sftp_err)?;
        rf.seek(std::io::SeekFrom::Start(start)).await.map_err(|e| ClientError::Other(e.to_string()))?;
        let mut lf = tokio::fs::File::open(local).await.map_err(|e| ClientError::Other(e.to_string()))?;
        lf.seek(std::io::SeekFrom::Start(start)).await.map_err(|e| ClientError::Other(e.to_string()))?;
        let mut done = start;
        let mut buf = vec![0u8; 256 * 1024];
        let mut last = Instant::now();
        loop {
            if *cancel.borrow_and_update() {
                return Err(ClientError::Cancelled);
            }
            let n = lf.read(&mut buf).await.map_err(|e| ClientError::Other(e.to_string()))?;
            if n == 0 {
                break;
            }
            rf.write_all(&buf[..n]).await.map_err(|e| ClientError::Network(e.to_string()))?;
            done += n as u64;
            if last.elapsed() > Duration::from_millis(200) {
                last = Instant::now();
                progress.on_progress(done, total);
            }
        }
        rf.shutdown().await.map_err(|e| ClientError::Network(e.to_string()))?;
        progress.on_progress(done, total);
        Ok(())
    }
}

fn connect_err(e: russh::Error, host: &str, port: u16) -> ClientError {
    match e {
        russh::Error::UnknownKey => ClientError::Permission("没有信任对方的主机密钥，已取消连接".into()),
        russh::Error::IO(io) => ClientError::Network(format!("无法连接 {host}:{port}：{io}")),
        other => ClientError::Network(format!("无法连接 {host}:{port}：{other}")),
    }
}

async fn authenticate(handle: &mut Handle<Handler>, user: &str, auth: &SshAuthMethod, prompt: &Arc<dyn SshPrompt>) -> Result<()> {
    let ask = |p: String, echo: bool| {
        let pr = prompt.clone();
        async move { tokio::task::spawn_blocking(move || pr.ask(&p, echo)).await.ok().flatten() }
    };
    let ok = match auth {
        SshAuthMethod::Password(pw) => {
            let pw = match pw {
                Some(p) if !p.is_empty() => p.clone(),
                _ => ask(format!("{user} 的密码"), false).await.ok_or(ClientError::Cancelled)?,
            };
            let r = handle.authenticate_password(user, pw.clone()).await.map_err(net)?;
            if r.success() {
                true
            } else {
                // 有的服务器只开了键盘交互方式的密码登录
                keyboard_interactive(handle, user, prompt, Some(pw)).await?
            }
        }
        SshAuthMethod::PrivateKey { key, passphrase } => {
            let parsed = match russh::keys::decode_secret_key(key, passphrase.as_deref()) {
                Ok(k) => k,
                Err(_) if passphrase.is_none() => {
                    let p = ask("私钥口令".into(), false).await.ok_or(ClientError::Cancelled)?;
                    russh::keys::decode_secret_key(key, Some(&p)).map_err(|_| ClientError::Auth { code: "bad_passphrase".into(), message: "私钥口令不正确".into() })?
                }
                Err(e) => return Err(ClientError::Auth { code: "bad_key".into(), message: format!("无法读取私钥：{e}") }),
            };
            let hash = handle.best_supported_rsa_hash().await.map_err(net)?.flatten();
            let r = handle.authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(parsed), hash)).await.map_err(net)?;
            r.success()
        }
        SshAuthMethod::Interactive => keyboard_interactive(handle, user, prompt, None).await?,
    };
    if ok {
        Ok(())
    } else {
        Err(ClientError::Auth { code: "ssh_auth_failed".into(), message: "SSH 认证失败".into() })
    }
}

async fn keyboard_interactive(handle: &mut Handle<Handler>, user: &str, prompt: &Arc<dyn SshPrompt>, password: Option<String>) -> Result<bool> {
    let mut r = handle.authenticate_keyboard_interactive_start(user, None).await.map_err(net)?;
    let mut used_password = false;
    for _ in 0..10 {
        match r {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
            KeyboardInteractiveAuthResponse::InfoRequest { name, instructions, prompts } => {
                let mut answers = Vec::with_capacity(prompts.len());
                for p in prompts {
                    // 第一个不回显的问题若是密码，直接用已知密码
                    if !p.echo && !used_password {
                        if let Some(pw) = password.clone() {
                            used_password = true;
                            answers.push(pw);
                            continue;
                        }
                    }
                    let text = [name.as_str(), instructions.as_str(), p.prompt.as_str()].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join("\n");
                    let pr = prompt.clone();
                    let echo = p.echo;
                    let a = tokio::task::spawn_blocking(move || pr.ask(&text, echo)).await.ok().flatten().ok_or(ClientError::Cancelled)?;
                    answers.push(a);
                }
                r = handle.authenticate_keyboard_interactive_respond(answers).await.map_err(net)?;
            }
        }
    }
    Ok(false)
}

enum TermOp {
    Write(Vec<u8>),
    Resize(u16, u16),
    Close,
}

/// SSH 终端（关掉即结束，SSH 没有断线继承）。
pub struct SshTerminal {
    tx: mpsc::UnboundedSender<TermOp>,
    pub title: String,
}

impl SshTerminal {
    pub fn write(&self, data: Vec<u8>) {
        let _ = self.tx.send(TermOp::Write(data));
    }
    pub fn resize(&self, cols: u16, rows: u16) {
        let _ = self.tx.send(TermOp::Resize(cols, rows));
    }
    pub fn close(&self) {
        let _ = self.tx.send(TermOp::Close);
    }
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

fn entry(name: &str, path: &str, md: &FileAttributes) -> FileEntry {
    let kind = if md.is_dir() {
        FileKind::Dir
    } else if md.is_symlink() {
        FileKind::Symlink
    } else if md.is_regular() {
        FileKind::File
    } else {
        FileKind::Other
    };
    FileEntry {
        name: name.to_string(),
        path: path.to_string(),
        kind,
        size: if kind == FileKind::Dir { 0 } else { md.size.unwrap_or(0) },
        mode: md.permissions.unwrap_or(0) & 0o7777,
        modified_at: md.mtime.map(|t| t as i64 * 1000).unwrap_or(0),
        owner: md.user.clone().or_else(|| md.uid.map(|u| u.to_string())).unwrap_or_default(),
        group: md.group.clone().or_else(|| md.gid.map(|g| g.to_string())).unwrap_or_default(),
        link_target: None,
    }
}

fn remove_tree<'a>(s: &'a SftpSession, path: &'a str, depth: u32) -> futures::future::BoxFuture<'a, Result<()>> {
    Box::pin(async move {
        if depth > 64 {
            return Err(ClientError::Other("目录层级太深".into()));
        }
        let rd = s.read_dir(path.to_string()).await.map_err(sftp_err)?;
        for e in rd {
            let name = e.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let full = join(path, &name);
            if e.metadata().is_dir() {
                remove_tree(s, &full, depth + 1).await?;
            } else {
                s.remove_file(full).await.map_err(sftp_err)?;
            }
        }
        s.remove_dir(path.to_string()).await.map_err(sftp_err)
    })
}
