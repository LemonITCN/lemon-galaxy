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
    /// 画面不是我们自己停的却结束了：被控端结束了画面流、流断了、或者画面没有响应（见 open_display）。
    /// 每个画面最多一次；自己调 DisplayChannel::stop 时不会来。
    fn on_closed(&self, _reason: String) {}
}

/// 画面结束的通知只发一次（被控端结束、流出错、看门狗判定没有响应，谁先到算谁；自己停的不发）。
struct CloseOnce {
    done: Arc<std::sync::atomic::AtomicBool>,
    sink: Arc<dyn VideoSink>,
}

impl CloseOnce {
    fn fire(&self, reason: String) {
        if !self.done.swap(true, std::sync::atomic::Ordering::SeqCst) {
            self.sink.on_closed(reason);
        }
    }
    fn is_done(&self) -> bool {
        self.done.load(std::sync::atomic::Ordering::SeqCst)
    }
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
    /// 正在录像时，收到的声音包也写进录像
    audio_rec: Mutex<Option<std::sync::Weak<crate::record::Recorder>>>,
    /// 本会话发给被控端的剪贴板文件清单（对方粘贴时经反向流来读，docs/protocol.md 8.4.1）
    pub(crate) clip_offers: crate::clipoffer::LocalOffers,
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
            audio_rec: Default::default(),
            clip_offers: Default::default(),
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
        // 被控端开来的反向流：对方粘贴时读本会话剪贴板清单里的文件（docs/protocol.md 8.4.1）
        let weak = Arc::downgrade(&me);
        let conn = me.conn.clone();
        tokio::spawn(async move {
            while let Ok((send, recv)) = conn.accept_bi().await {
                let Some(me) = weak.upgrade() else { break };
                tokio::spawn(async move {
                    let _ = crate::clipoffer::serve_reverse(me, send, recv).await;
                });
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
        // 关闭包要经这条路径发出去，立刻拆掉的话对方只能等空闲超时（约 30 秒）才知道会话结束
        let (ms, dev, conn, peer) = (self.ms.clone(), self.device, self.conn.clone(), self._peer.clone());
        match tokio::runtime::Handle::try_current() {
            Ok(h) => {
                h.spawn(async move {
                    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), conn.closed()).await;
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    ms.remove_peer_if_same(&dev, &peer);
                });
            }
            Err(_) => ms.remove_peer(&dev),
        }
    }

    pub async fn closed(&self) -> String {
        self.conn.closed().await.to_string()
    }

    async fn open(&self, open: StreamOpen) -> Result<(quinn::SendStream, quinn::RecvStream)> {
        let (mut s, r) = self.conn.open_bi().await?;
        frame::write(&mut s, &open).await?;
        Ok((s, r))
    }

    pub(crate) async fn open_stream(&self, open: StreamOpen) -> Result<(quinn::SendStream, quinn::RecvStream)> {
        self.open(open).await
    }

    /// 首字节为 `channel` 的数据报交给 `tx`。
    pub(crate) fn register_datagrams(&self, channel: u8, tx: mpsc::Sender<Bytes>) {
        self.video.lock().insert(channel, tx);
    }

    pub(crate) fn unregister_datagrams(&self, channel: u8) {
        self.video.lock().remove(&channel);
    }

    /// 录像时把收到的声音也写进去（传 None 停止）。
    pub fn set_audio_recorder(&self, r: Option<&Arc<crate::record::Recorder>>) {
        *self.audio_rec.lock() = r.map(Arc::downgrade);
    }

    pub(crate) fn audio_recorder(&self) -> Option<Arc<crate::record::Recorder>> {
        self.audio_rec.lock().as_ref().and_then(|w| w.upgrade())
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

    /// 主控最多能加几块虚拟屏；被控端不支持或没有控制权限时为 0。
    pub fn virtual_display_max(&self) -> u32 {
        if !self.ack.capabilities.iter().any(|c| c == CAP_VIRTUAL_DISPLAY) {
            return 0;
        }
        capability_value(&self.ack.capabilities, CAP_VIRTUAL_DISPLAY_MAX).and_then(|v| v.parse().ok()).unwrap_or(1)
    }

    /// 在被控端加一块虚拟屏（扩展桌面），返回它的屏幕信息，之后按 id 开画面。会话结束时被控端自动删掉。
    pub async fn add_virtual_display(&self, width: u32, height: u32, refresh: u32) -> Result<DisplayInfo> {
        match self.mgmt(MgmtReq::VirtualDisplayAdd { width, height, refresh }).await? {
            MgmtResp::Display(d) => Ok(d),
            _ => Err(ClientError::Protocol("应答错误".into())),
        }
    }

    /// 删掉本会话加的虚拟屏。
    pub async fn remove_virtual_display(&self, display_id: u32) -> Result<()> {
        match self.mgmt(MgmtReq::VirtualDisplayRemove { display_id }).await? {
            MgmtResp::Ok => Ok(()),
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
        let recorder: Arc<Mutex<Option<Arc<crate::record::Recorder>>>> = Default::default();
        let dims = Arc::new(std::sync::atomic::AtomicU64::new(((w as u64) << 32) | h as u64));
        let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ch = Arc::new(DisplayChannel { send: tokio::sync::Mutex::new(send), channel, recorder: recorder.clone(), dims: dims.clone(), closed: closed.clone() });
        let close_once = Arc::new(CloseOnce { done: closed, sink: sink.clone() });
        // 被控端空闲时也每秒至少发一帧（能力 video_keepalive）时才能用「多久没数据」判断画面卡死
        let keepalive = self.ack.capabilities.iter().any(|c| c == CAP_VIDEO_KEEPALIVE);
        // 被控端能补发丢了的分片（能力 video_nack）：按帧号顺序交、缺片就要（vrecv.rs）；老被控端照旧丢帧后要关键帧
        let nack_mode = self.ack.capabilities.iter().any(|c| c == CAP_VIDEO_NACK);
        if nack_mode {
            // 打招呼：告诉被控端这一路会补要分片（它据此不把零星丢包当拥塞，见 galaxy-host rate.rs）
            let _ = frame::write(&mut *ch.send.lock().await, &DisplayIn::Nack(vec![])).await;
        }
        let conn = self.conn.clone();
        // 控制消息；这条流结束（被控端结束画面、出错、断线）要告诉界面，不然界面停在最后一帧还以为连着
        let sink2 = sink.clone();
        let rec2 = recorder.clone();
        let close2 = close_once.clone();
        tokio::spawn(async move {
            let reason = loop {
                let m = match frame::read::<DisplayOut>(&mut recv).await {
                    Ok(Some(m)) => m,
                    Ok(None) => break "被控端结束了画面".to_string(),
                    Err(e) => break format!("画面连接中断：{e}"),
                };
                match m {
                    DisplayOut::Cursor { x, y, visible } => sink2.on_cursor(x, y, visible),
                    DisplayOut::Resized { width, height } => {
                        dims.store(((width as u64) << 32) | height as u64, std::sync::atomic::Ordering::Relaxed);
                        if let Some(r) = rec2.lock().clone() {
                            r.on_config(width, height);
                        }
                        sink2.on_config(codec, width, height)
                    }
                    DisplayOut::Error(e) => break e.message,
                    _ => {}
                }
            };
            close2.fire(reason);
        });
        // 重组：收不齐、跳号的帧都算丢帧；丢帧后到关键帧之前的帧不交给解码器，并要关键帧（见 Reassembler）。
        // 丢片重传时缺片先要被控端补发，补不回来才算丢帧（见 vrecv.rs）
        let ch2 = Arc::downgrade(&ch);
        tokio::spawn(async move {
            let mut asm = Reassembler::default();
            let mut oasm = crate::vrecv::OrderedAssembler::default();
            let mut rtt = conn.rtt();
            let mut rtt_at = std::time::Instant::now();
            let mut last_tick = std::time::Instant::now();
            // 刚打开时被控端第一帧就是关键帧，先等一会再催
            let mut last_kf_req = std::time::Instant::now();
            // 看门狗：最后一次收到画面数据的时间，以及这段静默里有没有探过（要过关键帧）
            let mut last_data = std::time::Instant::now();
            let mut got_any = false;
            let mut probed = false;
            loop {
                // 没有新数据时也要定时检查：收不齐的帧超时、等关键帧时重催；丢片重传时有没收齐的帧就勤点查（尾部丢片要靠它发现）
                let poll = if nack_mode && oasm.incomplete() { NACK_POLL } else { KEYFRAME_POLL };
                let got = match tokio::time::timeout(poll, rx.recv()).await {
                    Ok(Some(d)) => Some(d),
                    Ok(None) => break,
                    Err(_) => None,
                };
                let now = std::time::Instant::now();
                if got.is_some() {
                    last_data = now;
                    got_any = true;
                    probed = false;
                }
                if close_once.is_done() {
                    break;
                }
                if keepalive {
                    let quiet = now.duration_since(last_data);
                    if quiet >= STALL_PROBE && !probed {
                        // 被控端本该每秒至少一帧：先要个关键帧探一下
                        probed = true;
                        last_kf_req = now;
                        if let Some(c) = ch2.upgrade() {
                            c.request_keyframe().await;
                        }
                    }
                    if quiet >= if got_any { STALL_DEAD } else { STALL_DEAD_FIRST } {
                        close_once.fire("画面没有响应".into());
                        if let Some(c) = ch2.upgrade() {
                            c.stop().await;
                        }
                        break;
                    }
                }
                let frag = got.as_ref().and_then(|d| VideoFragHeader::decode(d).map(|(h, pos)| (h, d.slice(pos..))));
                let (frames, nacks, new_loss, waiting) = if nack_mode {
                    if now.duration_since(rtt_at) >= Duration::from_millis(500) {
                        rtt = conn.rtt();
                        rtt_at = now;
                    }
                    let mut out = match frag {
                        Some((h, payload)) => oasm.push(h, payload, now, rtt),
                        None => crate::vrecv::Out::default(),
                    };
                    // 数据一直在来时也要定时查尾部丢片、补发超时与放弃
                    if got.is_none() || now.duration_since(last_tick) >= NACK_POLL {
                        last_tick = now;
                        let t = oasm.tick(now, rtt);
                        out.frames.extend(t.frames);
                        out.nacks.extend(t.nacks);
                        out.new_loss |= t.new_loss;
                    }
                    (out.frames, out.nacks, out.new_loss, oasm.waiting_keyframe())
                } else {
                    let step = match frag {
                        Some((h, payload)) => asm.push(h, payload, now),
                        None => asm.tick(now),
                    };
                    (step.frame.into_iter().collect(), vec![], step.new_loss, asm.waiting_keyframe())
                };
                for (data, pts, kf) in frames {
                    if let Some(r) = recorder.lock().clone() {
                        r.on_video(&data, kf);
                    }
                    sink.on_frame(data, pts, kf);
                }
                if !nacks.is_empty() {
                    let Some(c) = ch2.upgrade() else { break };
                    c.nack(nacks).await;
                }
                if waiting {
                    // 刚发现丢帧立刻要；之后关键帧也可能丢，隔一会再催
                    let gap = if new_loss { KEYFRAME_REQ_MIN_GAP } else { KEYFRAME_RETRY };
                    if now.duration_since(last_kf_req) >= gap {
                        last_kf_req = now;
                        let Some(c) = ch2.upgrade() else { break };
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

    /// 隐私屏开关（需要控制权限；被控端不支持时报错）。
    pub async fn set_privacy(&self, on: bool) -> Result<()> {
        match self.mgmt(MgmtReq::SetPrivacy { on }).await? {
            MgmtResp::Ok => Ok(()),
            MgmtResp::Error(e) => Err(ClientError::Permission(e.message)),
            _ => Err(ClientError::Protocol("应答错误".into())),
        }
    }

    /// 远程升级被控端（能力列表要有 "remote_update"）：返回要升级到的版本，None 表示对方已是最新。
    /// 返回后对方会断开、以新版本重新上线；主控端据设备列表里的版本号判断升级完成。
    pub async fn apply_update(&self) -> Result<Option<String>> {
        if !self.ack.capabilities.iter().any(|c| c == galaxy_proto::peer::CAP_REMOTE_UPDATE) {
            return Err(ClientError::Protocol("对方版本太旧，不支持远程升级，需要先在那台设备上升级一次".into()));
        }
        match self.mgmt(MgmtReq::ApplyUpdate).await? {
            MgmtResp::Update { version } => Ok(version),
            MgmtResp::Error(e) if e.code == "permission_denied" => Err(ClientError::Permission(e.message)),
            MgmtResp::Error(e) => Err(ClientError::Other(e.message)),
            _ => Err(ClientError::Protocol("应答错误".into())),
        }
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

pub(crate) fn peer_err(e: PeerError) -> ClientError {
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
    recorder: Arc<Mutex<Option<Arc<crate::record::Recorder>>>>,
    /// 当前画面尺寸（高 32 位宽、低 32 位高）
    dims: Arc<std::sync::atomic::AtomicU64>,
    /// 画面已结束（自己停的或已通知过界面），之后不再通知
    closed: Arc<std::sync::atomic::AtomicBool>,
}

impl DisplayChannel {
    /// 开始录像（分片 MP4，不重新编码）。马上请求一个关键帧，从关键帧开始写。
    pub async fn start_recording(&self, path: &std::path::Path, with_audio: bool) -> std::result::Result<Arc<crate::record::Recorder>, String> {
        if self.recorder.lock().is_some() {
            return Err("已经在录像了".into());
        }
        let r = crate::record::Recorder::new(path, with_audio)?;
        let d = self.dims.load(std::sync::atomic::Ordering::Relaxed);
        r.on_config((d >> 32) as u32, d as u32);
        *self.recorder.lock() = Some(r.clone());
        self.request_keyframe().await;
        Ok(r)
    }

    /// 停止录像，返回写出的文件；没在录返回 None。
    pub fn stop_recording(&self) -> Option<std::result::Result<crate::record::RecordingSummary, String>> {
        let r = self.recorder.lock().take()?;
        Some(r.finish())
    }

    pub fn is_recording(&self) -> bool {
        self.recorder.lock().is_some()
    }

    pub async fn send_input(&self, ev: InputEvent) {
        let _ = frame::write(&mut *self.send.lock().await, &DisplayIn::Input(ev)).await;
    }
    pub async fn request_keyframe(&self) {
        let _ = frame::write(&mut *self.send.lock().await, &DisplayIn::RequestKeyframe).await;
    }

    /// 要被控端补发丢了的分片（只在被控端有能力 video_nack 时调用）
    async fn nack(&self, v: Vec<FragNack>) {
        let _ = frame::write(&mut *self.send.lock().await, &DisplayIn::Nack(v)).await;
    }
    /// 主控端自己画光标（被控端能力里有 "client_cursor" 才发）。
    pub async fn set_client_cursor(&self, on: bool) {
        let _ = frame::write(&mut *self.send.lock().await, &DisplayIn::ClientCursor(on)).await;
    }
    pub async fn set_bitrate(&self, kbps: u32) {
        let _ = frame::write(&mut *self.send.lock().await, &DisplayIn::SetBitrate(kbps)).await;
    }
    pub async fn stop(&self) {
        // 自己停的：之后这条流结束不再当成「画面断了」通知界面
        self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
        let mut s = self.send.lock().await;
        let _ = frame::write(&mut *s, &DisplayIn::Stop).await;
        let _ = s.finish();
    }
}

/// 一帧的分片隔这么久没有新的到达，就认为剩下的丢了。按「最后一片」算而不是「第一片」：
/// 码率低时一个大关键帧本身就要传几百毫秒，按第一片算会把正在传的关键帧当丢失，再要、再丢，一直循环。
const FRAME_STALL: Duration = Duration::from_millis(150);
/// 发现丢帧后立刻要关键帧，只防同一瞬间重复要
const KEYFRAME_REQ_MIN_GAP: Duration = Duration::from_millis(100);
/// 等关键帧期间每隔这么久再催一次（关键帧本身也可能丢）
const KEYFRAME_RETRY: Duration = Duration::from_millis(500);
/// 没有新数据时多久检查一次超时与重催
const KEYFRAME_POLL: Duration = Duration::from_millis(100);
/// 丢片重传时有没收齐的帧，多久查一次（尾部丢片、补发超时）
const NACK_POLL: Duration = Duration::from_millis(5);
/// 看门狗（被控端有 video_keepalive 时）：这么久没有画面数据先要关键帧探一下
const STALL_PROBE: Duration = Duration::from_secs(4);
/// 再久还没有就判定画面没有响应，结束画面让界面重连
const STALL_DEAD: Duration = Duration::from_secs(10);
/// 刚打开、一帧都还没收到时多给点时间（被控端可能在开虚拟桌面）
const STALL_DEAD_FIRST: Duration = Duration::from_secs(20);

/// `a` 不晚于 `b`（帧号回绕也对）
fn seq_le(a: u32, b: u32) -> bool {
    b.wrapping_sub(a) < 1 << 31
}

struct Partial {
    last_seen: std::time::Instant,
    parts: Vec<Option<Bytes>>,
    pts: u64,
    keyframe: bool,
}

#[derive(Default)]
struct Assembled {
    /// 可以交给解码器的完整帧
    frame: Option<(Vec<u8>, u64, bool)>,
    /// 这一步刚发现丢帧（之前没在等关键帧）
    new_loss: bool,
}

/// 画面分片重组与丢帧处理。
///
/// 各被控端都是无限 GOP、只在被要求时出关键帧，P 帧引用上一帧：中间丢了一帧，后面的 P 帧送去解码就是花屏，
/// 而且不会自己好（只有变化过的区域慢慢被重画）。所以：
/// - 丢帧的判定：帧收不齐超时；后一帧先收齐（前面没收齐的再也用不上）；帧号跳号（整帧一片都没收到，
///   小变化、光标移动时一帧往往只有一两片，最常见）。
/// - 丢帧后到下一个关键帧之前，收齐的帧也不交给解码器，画面停在最后一张好的帧上，调用方去要关键帧。
/// - 刚打开时也一样：从第一个关键帧开始交。
struct Reassembler {
    frames: HashMap<u32, Partial>,
    /// 最后一个收齐的帧号（不管交没交给解码器）
    last_done: Option<u32>,
    want_key: bool,
}

impl Default for Reassembler {
    fn default() -> Self {
        Self { frames: HashMap::new(), last_done: None, want_key: true }
    }
}

impl Reassembler {
    fn waiting_keyframe(&self) -> bool {
        self.want_key
    }

    fn mark_lost(&mut self, out: &mut Assembled) {
        if !self.want_key {
            self.want_key = true;
            out.new_loss = true;
        }
    }

    /// 没有新分片时调用：清掉卡住的帧
    fn tick(&mut self, now: std::time::Instant) -> Assembled {
        let mut out = Assembled::default();
        let before = self.frames.len();
        self.frames.retain(|_, p| now.duration_since(p.last_seen) < FRAME_STALL);
        if self.frames.len() < before {
            self.mark_lost(&mut out);
        }
        out
    }

    fn push(&mut self, h: VideoFragHeader, payload: Bytes, now: std::time::Instant) -> Assembled {
        if self.last_done.is_some_and(|ld| seq_le(h.frame_id, ld)) {
            // 已经收齐或已经放弃的帧，迟到的分片
            return self.tick(now);
        }
        let id = h.frame_id;
        let e = self.frames.entry(id).or_insert_with(|| Partial { last_seen: now, parts: vec![None; h.frag_count as usize], pts: 0, keyframe: false });
        e.last_seen = now;
        if let Some(slot) = e.parts.get_mut(h.frag_idx as usize) {
            *slot = Some(payload);
        }
        if h.frag_idx == 0 {
            e.pts = h.pts_us;
        }
        e.keyframe |= h.keyframe;
        if !e.parts.iter().all(|x| x.is_some()) {
            return self.tick(now);
        }
        let p = self.frames.remove(&id).expect("刚取到");
        let mut data = Vec::with_capacity(p.parts.iter().map(|x| x.as_ref().map_or(0, |b| b.len())).sum());
        for part in p.parts.into_iter().flatten() {
            data.extend_from_slice(&part);
        }
        let mut out = self.tick(now);
        // 前面没收齐的帧再也用不上了；帧号不连续说明中间有帧整个没收到
        let expected = self.last_done.map(|ld| ld.wrapping_add(1));
        self.frames.retain(|fid, _| !seq_le(*fid, id));
        self.last_done = Some(id);
        if expected.is_some_and(|x| x != id) {
            self.mark_lost(&mut out);
        }
        if self.want_key {
            if !p.keyframe {
                return out;
            }
            self.want_key = false;
            out.new_loss = false;
        }
        out.frame = Some((data, p.pts, p.keyframe));
        out
    }
}

#[cfg(test)]
mod reassembler_tests {
    use super::*;
    use std::time::Instant;

    fn frag(id: u32, idx: u16, count: u16, key: bool) -> (VideoFragHeader, Bytes) {
        (VideoFragHeader { channel: 1, frame_id: id, frag_idx: idx, frag_count: count, keyframe: key, pts_us: id as u64 }, Bytes::from(vec![id as u8, idx as u8]))
    }

    /// 整帧按顺序送进去，返回交给解码器的帧号
    fn feed(a: &mut Reassembler, id: u32, count: u16, key: bool, now: Instant) -> (Option<u32>, bool) {
        let mut last = Assembled::default();
        let mut loss = false;
        for i in 0..count {
            let (h, p) = frag(id, i, count, key);
            last = a.push(h, p, now);
            loss |= last.new_loss;
        }
        (last.frame.map(|f| f.1 as u32), loss)
    }

    #[test]
    fn waits_for_first_keyframe() {
        let mut a = Reassembler::default();
        let t = Instant::now();
        assert_eq!(feed(&mut a, 0, 1, false, t).0, None);
        assert!(a.waiting_keyframe());
        assert_eq!(feed(&mut a, 1, 3, true, t).0, Some(1));
        assert!(!a.waiting_keyframe());
        assert_eq!(feed(&mut a, 2, 1, false, t).0, Some(2));
    }

    #[test]
    fn whole_frame_missing_holds_until_keyframe() {
        let mut a = Reassembler::default();
        let t = Instant::now();
        feed(&mut a, 0, 2, true, t);
        assert_eq!(feed(&mut a, 1, 1, false, t).0, Some(1));
        // 帧 2 一片都没到
        let (f, loss) = feed(&mut a, 3, 1, false, t);
        assert_eq!(f, None);
        assert!(loss && a.waiting_keyframe());
        assert_eq!(feed(&mut a, 4, 1, false, t), (None, false));
        assert_eq!(feed(&mut a, 5, 4, true, t).0, Some(5));
        assert_eq!(feed(&mut a, 6, 1, false, t).0, Some(6));
    }

    #[test]
    fn superseded_partial_frame_counts_as_loss() {
        let mut a = Reassembler::default();
        let t = Instant::now();
        feed(&mut a, 0, 1, true, t);
        // 帧 1 只到了一片
        let (h, p) = frag(1, 0, 3, false);
        assert!(a.push(h, p, t).frame.is_none());
        let (f, loss) = feed(&mut a, 2, 1, false, t);
        assert_eq!(f, None);
        assert!(loss);
        // 帧 1 迟到的分片不再理会
        let (h, p) = frag(1, 1, 3, false);
        assert!(a.push(h, p, t).frame.is_none());
        assert!(a.waiting_keyframe());
    }

    #[test]
    fn stalled_frame_times_out_but_slow_keyframe_survives() {
        let mut a = Reassembler::default();
        let t = Instant::now();
        feed(&mut a, 0, 1, true, t);
        // 大关键帧慢慢传：每片间隔 100 毫秒，总共 600 毫秒，不能被当成丢失
        for i in 0..6u16 {
            let (h, p) = frag(1, i, 7, true);
            let now = t + Duration::from_millis(100 * i as u64);
            assert!(a.push(h, p, now).frame.is_none());
            assert!(!a.tick(now + Duration::from_millis(50)).new_loss);
        }
        let (h, p) = frag(1, 6, 7, true);
        assert_eq!(a.push(h, p, t + Duration::from_millis(600)).frame.map(|f| f.1), Some(1));
        // 卡住不动的帧超时算丢
        let (h, p) = frag(2, 0, 2, false);
        a.push(h, p, t + Duration::from_millis(700));
        assert!(!a.tick(t + Duration::from_millis(800)).new_loss);
        assert!(a.tick(t + Duration::from_millis(900)).new_loss);
        assert!(a.waiting_keyframe());
    }

    #[test]
    fn frame_id_wraps() {
        let mut a = Reassembler::default();
        let t = Instant::now();
        assert!(feed(&mut a, u32::MAX, 1, true, t).0.is_some());
        let (h, p) = frag(0, 0, 1, false);
        assert!(a.push(h, p, t).frame.is_some());
        assert!(!a.waiting_keyframe());
    }
}
