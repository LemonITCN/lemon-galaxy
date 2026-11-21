//! 主控端与一台被控设备的会话：终端、文件、命令执行、管理、远程画面、端口映射。

use crate::error::{ClientError, Result};
use crate::signal::SignalClient;
use bytes::Bytes;
use galaxy_net::frame;
use galaxy_net::magicsock::{MagicSock, Peer};
use galaxy_net::PathInfo;
use galaxy_proto::peer::*;
use galaxy_proto::ws::WsMsg;
use galaxy_proto::{ConnId, DeviceId};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

pub trait TerminalSink: Send + Sync {
    fn on_output(&self, data: Vec<u8>);
    fn on_exit(&self, code: Option<i32>);
}

pub trait VideoSink: Send + Sync {
    fn on_config(&self, codec: VideoCodec, width: u32, height: u32);
    fn on_frame(&self, data: Vec<u8>, pts_us: u64, keyframe: bool);
    fn on_cursor(&self, x: i32, y: i32, visible: bool);
}

pub trait TransferProgress: Send + Sync {
    fn on_progress(&self, done: u64, total: u64);
    fn on_finished(&self, error: Option<String>);
}

pub struct PeerSession {
    pub conn: quinn::Connection,
    pub ack: HelloAck,
    pub device: DeviceId,
    pub conn_id: Option<ConnId>,
    path: watch::Receiver<PathInfo>,
    _control: Mutex<Option<(quinn::SendStream, quinn::RecvStream)>>,
    stop_probe: watch::Sender<bool>,
    signal: Option<Arc<SignalClient>>,
    video: Mutex<HashMap<u8, mpsc::Sender<Bytes>>>,
    next_channel: Mutex<u8>,
    ms: Arc<MagicSock>,
    _peer: Arc<Peer>,
}

impl PeerSession {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        conn: quinn::Connection,
        ack: HelloAck,
        device: DeviceId,
        conn_id: Option<ConnId>,
        control: (quinn::SendStream, quinn::RecvStream),
        ms: Arc<MagicSock>,
        peer: Arc<Peer>,
        stop_probe: watch::Sender<bool>,
        signal: Option<Arc<SignalClient>>,
    ) -> Arc<Self> {
        let me = Arc::new(Self {
            conn,
            ack,
            device,
            conn_id,
            path: peer.path(),
            _control: Mutex::new(Some(control)),
            stop_probe,
            signal,
            video: Default::default(),
            next_channel: Mutex::new(1),
            ms,
            _peer: peer,
        });
        // 画面数据报分发
        let weak = Arc::downgrade(&me);
        let conn = me.conn.clone();
        tokio::spawn(async move {
            while let Ok(d) = conn.read_datagram().await {
                let Some(me) = weak.upgrade() else { break };
                if let Some(ch) = d.first() {
                    let tx = me.video.lock().get(ch).cloned();
                    if let Some(tx) = tx {
                        let _ = tx.try_send(d);
                    }
                }
            }
        });
        // 路径变化上报服务器（用于后台展示）
        if let (Some(sig), Some(cid)) = (me.signal.clone(), conn_id) {
            let mut p = me.path.clone();
            let conn = me.conn.clone();
            tokio::spawn(async move {
                loop {
                    let info = p.borrow().clone();
                    sig.send(WsMsg::ConnState { conn_id: cid, path: info.kind, rtt_ms: info.rtt_ms });
                    tokio::select! {
                        r = p.changed() => if r.is_err() { break },
                        _ = conn.closed() => break,
                    }
                }
                sig.send(WsMsg::ConnClose { conn_id: cid });
            });
        }
        me
    }

    pub fn path(&self) -> PathInfo {
        self.path.borrow().clone()
    }

    pub fn path_watch(&self) -> watch::Receiver<PathInfo> {
        self.path.clone()
    }

    pub fn is_closed(&self) -> bool {
        self.conn.close_reason().is_some()
    }

    pub fn close(&self) {
        let _ = self.stop_probe.send(true);
        self.conn.close(0u32.into(), b"bye");
        if let (Some(sig), Some(cid)) = (&self.signal, self.conn_id) {
            sig.send(WsMsg::ConnClose { conn_id: cid });
        }
        self.ms.remove_peer(&self.device);
    }

    pub async fn closed(&self) -> String {
        self.conn.closed().await.to_string()
    }

    async fn open(&self, open: StreamOpen) -> Result<(quinn::SendStream, quinn::RecvStream)> {
        let (mut s, r) = self.conn.open_bi().await?;
        frame::write(&mut s, &open).await?;
        Ok((s, r))
    }

    // ---------------- 终端 ----------------

    pub async fn open_terminal(&self, open: TerminalOpen, sink: Arc<dyn TerminalSink>) -> Result<Arc<TerminalChannel>> {
        let (send, mut recv) = self.open(StreamOpen::Terminal(open)).await?;
        let first: TermOut = tokio::time::timeout(Duration::from_secs(15), frame::read(&mut recv))
            .await
            .map_err(|_| ClientError::Network("打开终端超时".into()))?
            .map_err(|e| ClientError::Network(e.to_string()))?
            .ok_or_else(|| ClientError::Network("对方关闭了终端".into()))?;
        let (session_id, title) = match first {
            TermOut::Opened { session_id, title, .. } => (session_id, title),
            TermOut::Error(e) => return Err(peer_err(e)),
            _ => return Err(ClientError::Protocol("终端应答错误".into())),
        };
        let (snap_tx, _) = tokio::sync::broadcast::channel::<String>(4);
        let ch = Arc::new(TerminalChannel { session_id, title, send: tokio::sync::Mutex::new(send), snap: snap_tx.clone() });
        tokio::spawn(async move {
            loop {
                match frame::read::<TermOut>(&mut recv).await {
                    Ok(Some(TermOut::Output(d))) => sink.on_output(d),
                    Ok(Some(TermOut::Snapshot(s))) => {
                        let _ = snap_tx.send(s);
                    }
                    Ok(Some(TermOut::Exited { code })) => {
                        sink.on_exit(code);
                        break;
                    }
                    Ok(Some(_)) => {}
                    Ok(None) | Err(_) => {
                        sink.on_exit(None);
                        break;
                    }
                }
            }
        });
        Ok(ch)
    }

    // ---------------- 管理 ----------------

    pub async fn mgmt(&self, req: MgmtReq) -> Result<MgmtResp> {
        let (mut s, mut r) = self.open(StreamOpen::Mgmt).await?;
        frame::write(&mut s, &req).await?;
        let _ = s.finish();
        let resp: MgmtResp = tokio::time::timeout(Duration::from_secs(30), frame::read(&mut r))
            .await
            .map_err(|_| ClientError::Network("请求超时".into()))?
            .map_err(|e| ClientError::Network(e.to_string()))?
            .ok_or_else(|| ClientError::Network("对方关闭了连接".into()))?;
        if let MgmtResp::Error(e) = resp {
            return Err(peer_err(e));
        }
        Ok(resp)
    }

    pub async fn list_terminals(&self) -> Result<Vec<TerminalSessionInfo>> {
        match self.mgmt(MgmtReq::ListTerminals).await? {
            MgmtResp::Terminals(v) => Ok(v),
            _ => Err(ClientError::Protocol("应答错误".into())),
        }
    }

    pub async fn list_displays(&self) -> Result<Vec<DisplayInfo>> {
        match self.mgmt(MgmtReq::ListDisplays).await? {
            MgmtResp::Displays(v) => Ok(v),
            _ => Err(ClientError::Protocol("应答错误".into())),
        }
    }

    pub async fn screenshot(&self, display: u32, max_width: u32, max_height: u32) -> Result<ImageData> {
        match self.mgmt(MgmtReq::Screenshot { display, max_width, max_height }).await? {
            MgmtResp::Image(i) => Ok(i),
            _ => Err(ClientError::Protocol("应答错误".into())),
        }
    }

    // ---------------- 文件 ----------------

    async fn file_simple(&self, op: FileOp) -> Result<FileMsg> {
        let (mut s, mut r) = self.open(StreamOpen::Files(op)).await?;
        let _ = s.finish();
        let m: FileMsg = tokio::time::timeout(Duration::from_secs(120), frame::read(&mut r))
            .await
            .map_err(|_| ClientError::Network("请求超时".into()))?
            .map_err(|e| ClientError::Network(e.to_string()))?
            .ok_or_else(|| ClientError::Network("对方关闭了连接".into()))?;
        if let FileMsg::Error(e) = m {
            return Err(peer_err(e));
        }
        Ok(m)
    }

    pub async fn fs_home(&self) -> Result<String> {
        match self.file_simple(FileOp::Home).await? {
            FileMsg::Path(p) => Ok(p),
            _ => Err(ClientError::Protocol("应答错误".into())),
        }
    }

    pub async fn fs_list(&self, path: &str) -> Result<Vec<FileEntry>> {
        match self.file_simple(FileOp::List { path: path.into() }).await? {
            FileMsg::Entries(v) => Ok(v),
            _ => Err(ClientError::Protocol("应答错误".into())),
        }
    }

    pub async fn fs_stat(&self, path: &str) -> Result<FileEntry> {
        match self.file_simple(FileOp::Stat { path: path.into() }).await? {
            FileMsg::Entry(e) => Ok(e),
            _ => Err(ClientError::Protocol("应答错误".into())),
        }
    }

    pub async fn fs_op(&self, op: FileOp) -> Result<()> {
        self.file_simple(op).await.map(|_| ())
    }

    pub async fn fs_search(&self, root: &str, pattern: &str, content: bool, max: u32) -> Result<Vec<FileEntry>> {
        match self.file_simple(FileOp::Search { root: root.into(), pattern: pattern.into(), content, max }).await? {
            FileMsg::Entries(v) => Ok(v),
            _ => Err(ClientError::Protocol("应答错误".into())),
        }
    }

    /// 读文件（可指定范围）。`on_chunk` 每收到一块回调一次。
    pub async fn fs_read<F: FnMut(&[u8], u64)>(&self, path: &str, offset: u64, len: Option<u64>, mut on_chunk: F) -> Result<u64> {
        let (mut s, mut r) = self.open(StreamOpen::Files(FileOp::Read { path: path.into(), offset, len })).await?;
        let _ = s.finish();
        let mut total = 0u64;
        let mut size = 0u64;
        loop {
            let m: Option<FileMsg> = tokio::time::timeout(Duration::from_secs(60), frame::read(&mut r))
                .await
                .map_err(|_| ClientError::Network("传输超时".into()))?
                .map_err(|e| ClientError::Network(e.to_string()))?;
            match m {
                Some(FileMsg::Size(n)) => size = n,
                Some(FileMsg::Chunk(d)) => {
                    total += d.len() as u64;
                    on_chunk(&d, size);
                }
                Some(FileMsg::End) | None => break,
                Some(FileMsg::Error(e)) => return Err(peer_err(e)),
                Some(_) => {}
            }
        }
        Ok(total)
    }

    pub async fn fs_read_all(&self, path: &str, max: u64) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.fs_read(path, 0, Some(max), |c, _| out.extend_from_slice(c)).await?;
        Ok(out)
    }

    /// 写文件。数据由 `chunks` 提供。
    pub async fn fs_write(&self, path: &str, offset: u64, truncate: bool, mode: Option<u32>, expect_sha256: Option<Vec<u8>>, origin: Origin, data: &[u8]) -> Result<(u64, Option<String>)> {
        let (mut s, mut r) = self.open(StreamOpen::Files(FileOp::Write { path: path.into(), offset, truncate, mode, expect_sha256, origin })).await?;
        for c in data.chunks(256 * 1024) {
            frame::write(&mut s, &FileMsg::Chunk(c.to_vec())).await?;
        }
        frame::write(&mut s, &FileMsg::End).await?;
        let _ = s.finish();
        let m: FileMsg = tokio::time::timeout(Duration::from_secs(120), frame::read(&mut r))
            .await
            .map_err(|_| ClientError::Network("传输超时".into()))?
            .map_err(|e| ClientError::Network(e.to_string()))?
            .ok_or_else(|| ClientError::Network("对方关闭了连接".into()))?;
        match m {
            FileMsg::Written { size, backup, .. } => Ok((size, backup)),
            FileMsg::Error(e) => Err(peer_err(e)),
            _ => Err(ClientError::Protocol("应答错误".into())),
        }
    }

    /// 下载到本地文件，支持断点续传（本地文件已存在则从其大小处继续）。
    pub async fn download(&self, remote: &str, local: &std::path::Path, progress: Arc<dyn TransferProgress>, cancel: watch::Receiver<bool>) -> Result<()> {
        use tokio::io::AsyncWriteExt;
        let offset = tokio::fs::metadata(local).await.map(|m| m.len()).unwrap_or(0);
        let mut f = tokio::fs::OpenOptions::new().create(true).append(true).open(local).await.map_err(|e| ClientError::Other(e.to_string()))?;
        let (mut s, mut r) = self.open(StreamOpen::Files(FileOp::Read { path: remote.into(), offset, len: None })).await?;
        let _ = s.finish();
        let mut done = offset;
        let mut total = 0u64;
        loop {
            if *cancel.borrow() {
                return Err(ClientError::Cancelled);
            }
            let m: Option<FileMsg> = tokio::time::timeout(Duration::from_secs(60), frame::read(&mut r))
                .await
                .map_err(|_| ClientError::Network("传输超时".into()))?
                .map_err(|e| ClientError::Network(e.to_string()))?;
            match m {
                Some(FileMsg::Size(n)) => total = n,
                Some(FileMsg::Chunk(d)) => {
                    f.write_all(&d).await.map_err(|e| ClientError::Other(e.to_string()))?;
                    done += d.len() as u64;
                    progress.on_progress(done, total);
                }
                Some(FileMsg::End) | None => break,
                Some(FileMsg::Error(e)) => return Err(peer_err(e)),
                Some(_) => {}
            }
        }
        f.flush().await.map_err(|e| ClientError::Other(e.to_string()))?;
        Ok(())
    }

    /// 上传本地文件；远端已有同名部分文件时从其大小处续传。
    pub async fn upload(&self, local: &std::path::Path, remote: &str, progress: Arc<dyn TransferProgress>, cancel: watch::Receiver<bool>) -> Result<()> {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};
        let total = tokio::fs::metadata(local).await.map_err(|e| ClientError::Other(e.to_string()))?.len();
        let offset = match self.fs_stat(remote).await {
            Ok(e) if e.size <= total => e.size,
            _ => 0,
        };
        let mut f = tokio::fs::File::open(local).await.map_err(|e| ClientError::Other(e.to_string()))?;
        f.seek(std::io::SeekFrom::Start(offset)).await.map_err(|e| ClientError::Other(e.to_string()))?;
        let (mut s, mut r) = self
            .open(StreamOpen::Files(FileOp::Write { path: remote.into(), offset, truncate: offset == 0, mode: None, expect_sha256: None, origin: Origin::User }))
            .await?;
        let mut buf = vec![0u8; 256 * 1024];
        let mut done = offset;
        loop {
            if *cancel.borrow() {
                return Err(ClientError::Cancelled);
            }
            let n = f.read(&mut buf).await.map_err(|e| ClientError::Other(e.to_string()))?;
            if n == 0 {
                break;
            }
            frame::write(&mut s, &FileMsg::Chunk(buf[..n].to_vec())).await?;
            done += n as u64;
            progress.on_progress(done, total);
        }
        frame::write(&mut s, &FileMsg::End).await?;
        let _ = s.finish();
        match frame::read::<FileMsg>(&mut r).await.map_err(|e| ClientError::Network(e.to_string()))? {
            Some(FileMsg::Written { .. }) => Ok(()),
            Some(FileMsg::Error(e)) => Err(peer_err(e)),
            _ => Err(ClientError::Protocol("应答错误".into())),
        }
    }

    // ---------------- 命令执行 ----------------

    /// 执行命令，流式回调输出；返回结束信息与合并后的输出。
    pub async fn exec<F: FnMut(bool, &[u8])>(&self, req: ExecRequest, mut on_out: F, mut cancel: Option<watch::Receiver<bool>>) -> Result<(ExecDone, Vec<u8>, Vec<u8>)> {
        let max = req.max_output.max(1024) as usize;
        let timeout = Duration::from_millis(req.timeout_ms.max(1000) + 15_000);
        let (mut s, mut r) = self.open(StreamOpen::Exec(req)).await?;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let read = frame::read::<ExecOut>(&mut r);
            let msg = tokio::select! {
                m = read => m,
                _ = tokio::time::sleep_until(deadline) => return Err(ClientError::Network("命令执行超时".into())),
                _ = async {
                    match cancel.as_mut() {
                        Some(c) => { while !*c.borrow() { if c.changed().await.is_err() { futures::future::pending::<()>().await; } } }
                        None => futures::future::pending::<()>().await,
                    }
                } => {
                    let _ = frame::write(&mut s, &ExecIn::Cancel).await;
                    cancel = None;
                    continue;
                }
            };
            match msg.map_err(|e| ClientError::Network(e.to_string()))? {
                Some(ExecOut::Started) => {}
                Some(ExecOut::Stdout(d)) => {
                    on_out(false, &d);
                    if stdout.len() < max {
                        stdout.extend_from_slice(&d);
                    }
                }
                Some(ExecOut::Stderr(d)) => {
                    on_out(true, &d);
                    if stderr.len() < max {
                        stderr.extend_from_slice(&d);
                    }
                }
                Some(ExecOut::Done(d)) => return Ok((d, stdout, stderr)),
                Some(ExecOut::Error(e)) => return Err(peer_err(e)),
                None => return Err(ClientError::Network("对方关闭了连接".into())),
            }
        }
    }

    // ---------------- 远程画面 ----------------

    pub async fn open_display(&self, mut req: DisplayRequest, sink: Arc<dyn VideoSink>) -> Result<Arc<DisplayChannel>> {
        if !self.ack.permissions.control {
            req.view_only = true;
        }
        let (send, mut recv) = self.open(StreamOpen::Display(req)).await?;
        let first: DisplayOut = tokio::time::timeout(Duration::from_secs(20), frame::read(&mut recv))
            .await
            .map_err(|_| ClientError::Network("打开画面超时".into()))?
            .map_err(|e| ClientError::Network(e.to_string()))?
            .ok_or_else(|| ClientError::Network("对方关闭了画面".into()))?;
        let (w, h, codec, channel) = match first {
            DisplayOut::Started { width, height, codec, channel } => (width, height, codec, channel),
            DisplayOut::Error(e) => return Err(peer_err(e)),
            _ => return Err(ClientError::Protocol("应答错误".into())),
        };
        sink.on_config(codec, w, h);
        let (tx, mut rx) = mpsc::channel::<Bytes>(1024);
        self.video.lock().insert(channel, tx);
        let ch = Arc::new(DisplayChannel { send: tokio::sync::Mutex::new(send), channel });
        // 控制消息
        let sink2 = sink.clone();
        tokio::spawn(async move {
            while let Ok(Some(m)) = frame::read::<DisplayOut>(&mut recv).await {
                match m {
                    DisplayOut::Cursor { x, y, visible } => sink2.on_cursor(x, y, visible),
                    DisplayOut::Resized { width, height } => sink2.on_config(codec, width, height),
                    _ => {}
                }
            }
        });
        // 重组
        let ch2 = Arc::downgrade(&ch);
        tokio::spawn(async move {
            let mut asm = Reassembler::default();
            let mut last_kf_req = std::time::Instant::now() - Duration::from_secs(10);
            while let Some(d) = rx.recv().await {
                let Some((h, pos)) = VideoFragHeader::decode(&d) else { continue };
                let (complete, lost) = asm.push(h, d.slice(pos..));
                if let Some((data, pts, kf)) = complete {
                    sink.on_frame(data, pts, kf);
                }
                if lost && last_kf_req.elapsed() > Duration::from_millis(500) {
                    last_kf_req = std::time::Instant::now();
                    if let Some(c) = ch2.upgrade() {
                        c.request_keyframe().await;
                    }
                }
            }
        });
        Ok(ch)
    }

    // ---------------- 端口映射 ----------------

    /// 经被控端打开一条到 `host:port` 的 TCP 连接（SSH 经设备跳转等用）。
    pub async fn open_tcp(&self, host: &str, port: u16) -> Result<(quinn::SendStream, quinn::RecvStream)> {
        let (s, mut r) = self.open(StreamOpen::PortForward { host: host.to_string(), port }).await?;
        match tokio::time::timeout(Duration::from_secs(20), frame::read::<ForwardReply>(&mut r)).await {
            Ok(Ok(Some(ForwardReply::Ok))) => Ok((s, r)),
            Ok(Ok(Some(ForwardReply::Error(e)))) => Err(ClientError::Network(format!("被控端连不上 {host}:{port}：{}", e.message))),
            Err(_) => Err(ClientError::Network(format!("被控端连接 {host}:{port} 超时"))),
            _ => Err(ClientError::Network(format!("被控端连不上 {host}:{port}"))),
        }
    }

    /// 在本机监听 `local_port`，每个连接经被控端转发到 `host:port`。返回实际端口。
    pub async fn forward_port(self: &Arc<Self>, local_port: u16, host: String, port: u16) -> Result<(u16, tokio::task::JoinHandle<()>)> {
        let l = tokio::net::TcpListener::bind(("127.0.0.1", local_port)).await.map_err(|e| ClientError::Other(e.to_string()))?;
        let actual = l.local_addr().map(|a| a.port()).unwrap_or(local_port);
        let me = self.clone();
        let h = tokio::spawn(async move {
            while let Ok((mut tcp, _)) = l.accept().await {
                let me = me.clone();
                let host = host.clone();
                tokio::spawn(async move {
                    let Ok((mut s, mut r)) = me.open(StreamOpen::PortForward { host, port }).await else { return };
                    match frame::read::<ForwardReply>(&mut r).await {
                        Ok(Some(ForwardReply::Ok)) => {}
                        _ => return,
                    }
                    let (mut tr, mut tw) = tcp.split();
                    let a = async {
                        let _ = tokio::io::copy(&mut tr, &mut s).await;
                        let _ = s.finish();
                    };
                    let b = async {
                        let _ = tokio::io::copy(&mut r, &mut tw).await;
                    };
                    tokio::join!(a, b);
                });
            }
        });
        Ok((actual, h))
    }

    pub async fn clipboard_get(&self) -> Result<Option<ClipboardData>> {
        match self.mgmt(MgmtReq::ClipboardGet).await? {
            MgmtResp::Clipboard(c) => Ok(c),
            _ => Err(ClientError::Protocol("应答错误".into())),
        }
    }

    pub async fn clipboard_set(&self, d: ClipboardData) -> Result<()> {
        self.mgmt(MgmtReq::ClipboardSet(d)).await.map(|_| ())
    }
}

fn peer_err(e: PeerError) -> ClientError {
    match e.code.as_str() {
        "permission_denied" | "ai_denied" | "forbidden" => ClientError::Permission(e.message),
        "not_found" => ClientError::NotFound(e.message),
        _ => ClientError::Other(e.message),
    }
}

pub struct TerminalChannel {
    pub session_id: String,
    pub title: String,
    send: tokio::sync::Mutex<quinn::SendStream>,
    snap: tokio::sync::broadcast::Sender<String>,
}

impl TerminalChannel {
    pub async fn write(&self, data: Vec<u8>) -> Result<()> {
        frame::write(&mut *self.send.lock().await, &TermIn::Input(data)).await.map_err(|e| ClientError::Network(e.to_string()))
    }
    pub async fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        frame::write(&mut *self.send.lock().await, &TermIn::Resize { cols, rows }).await.map_err(|e| ClientError::Network(e.to_string()))
    }
    pub async fn detach(&self) {
        let mut s = self.send.lock().await;
        let _ = frame::write(&mut *s, &TermIn::Detach).await;
        let _ = s.finish();
    }
    pub async fn kill(&self) {
        let mut s = self.send.lock().await;
        let _ = frame::write(&mut *s, &TermIn::Kill).await;
        let _ = s.finish();
    }
    /// 读取当前屏幕文字（AI 用）。
    pub async fn snapshot(&self) -> Result<String> {
        let mut rx = self.snap.subscribe();
        frame::write(&mut *self.send.lock().await, &TermIn::Snapshot).await.map_err(|e| ClientError::Network(e.to_string()))?;
        tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .map_err(|_| ClientError::Network("读取屏幕超时".into()))?
            .map_err(|_| ClientError::Network("终端已关闭".into()))
    }
}

pub struct DisplayChannel {
    send: tokio::sync::Mutex<quinn::SendStream>,
    pub channel: u8,
}

impl DisplayChannel {
    pub async fn send_input(&self, ev: InputEvent) {
        let _ = frame::write(&mut *self.send.lock().await, &DisplayIn::Input(ev)).await;
    }
    pub async fn request_keyframe(&self) {
        let _ = frame::write(&mut *self.send.lock().await, &DisplayIn::RequestKeyframe).await;
    }
    pub async fn set_bitrate(&self, kbps: u32) {
        let _ = frame::write(&mut *self.send.lock().await, &DisplayIn::SetBitrate(kbps)).await;
    }
    pub async fn stop(&self) {
        let mut s = self.send.lock().await;
        let _ = frame::write(&mut *s, &DisplayIn::Stop).await;
        let _ = s.finish();
    }
}

/// 画面分片重组：帧在 100 毫秒内收不齐就丢弃并报告丢失。
#[derive(Default)]
struct Reassembler {
    frames: HashMap<u32, (std::time::Instant, u16, Vec<Option<Bytes>>, u64, bool)>,
    last_done: Option<u32>,
}

impl Reassembler {
    fn push(&mut self, h: VideoFragHeader, payload: Bytes) -> (Option<(Vec<u8>, u64, bool)>, bool) {
        if let Some(ld) = self.last_done {
            if h.frame_id <= ld && ld.wrapping_sub(h.frame_id) < 1_000_000 {
                return (None, false);
            }
        }
        let e = self.frames.entry(h.frame_id).or_insert_with(|| (std::time::Instant::now(), h.frag_count, vec![None; h.frag_count as usize], 0, false));
        if (h.frag_idx as usize) < e.2.len() {
            e.2[h.frag_idx as usize] = Some(payload);
        }
        if h.frag_idx == 0 {
            e.3 = h.pts_us;
        }
        e.4 |= h.keyframe;
        let mut out = None;
        if e.2.iter().all(|x| x.is_some()) {
            let (_, _, parts, pts, kf) = self.frames.remove(&h.frame_id).unwrap();
            let mut data = Vec::with_capacity(parts.iter().map(|p| p.as_ref().unwrap().len()).sum());
            for p in parts {
                data.extend_from_slice(&p.unwrap());
            }
            self.last_done = Some(h.frame_id);
            out = Some((data, pts, kf));
        }
        let before = self.frames.len();
        let done = self.last_done;
        self.frames.retain(|id, f| f.0.elapsed() < Duration::from_millis(100) && done.map(|d| *id > d).unwrap_or(true));
        let lost = self.frames.len() < before && out.is_none() || self.frames.len() < before.saturating_sub(1);
        (out, lost)
    }
}
