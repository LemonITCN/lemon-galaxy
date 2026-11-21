//! 与一台被控设备的会话及其功能对象。

use crate::callbacks::*;
use crate::types::*;
use crate::{run, rt, GalaxyError, Result};
use galaxy_client::session as cs;
use galaxy_proto::peer as pp;
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};

#[derive(uniffi::Object)]
pub struct PeerSession {
    pub(crate) inner: Arc<cs::PeerSession>,
    #[allow(dead_code)]
    pub(crate) name: String,
}

impl PeerSession {
    pub(crate) fn wrap(inner: Arc<cs::PeerSession>, name: String) -> Arc<Self> {
        Arc::new(Self { inner, name })
    }
}

#[uniffi::export]
impl PeerSession {
    pub fn info(&self) -> PeerInfo {
        let a = &self.inner.ack;
        PeerInfo {
            device_id: self.inner.device.to_hex(),
            hostname: a.host.hostname.clone(),
            platform: a.host.platform.into(),
            os_version: a.host.os_version.clone(),
            arch: a.host.arch.clone(),
            system_user: a.host.system_user.clone(),
            shell: a.host.shell.clone(),
            permissions: a.permissions.clone().into(),
            capabilities: a.capabilities.clone(),
            path: self.inner.path().into(),
        }
    }

    pub fn path(&self) -> PathInfo {
        self.inner.path().into()
    }

    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    pub fn set_listener(&self, listener: Arc<dyn PeerListener>) {
        let mut pw = self.inner.path_watch();
        let s = self.inner.clone();
        rt().spawn(async move {
            loop {
                tokio::select! {
                    r = pw.changed() => {
                        if r.is_err() { break; }
                        let p: PathInfo = pw.borrow().clone().into();
                        listener.on_path_changed(p);
                    }
                    reason = s.closed() => {
                        listener.on_closed(reason);
                        break;
                    }
                }
            }
        });
    }

    /// 断开会话（不叫 close，避免与绑定自动生成的释放方法冲突）。
    pub fn disconnect(&self) {
        let s = self.inner.clone();
        rt().spawn(async move { s.close() });
    }

    pub async fn open_terminal(&self, options: TerminalOptions, sink: Arc<dyn TerminalSink>) -> Result<Arc<TerminalChannel>> {
        let s = self.inner.clone();
        run(async move {
            struct Sink(Arc<dyn TerminalSink>);
            impl cs::TerminalSink for Sink {
                fn on_output(&self, data: Vec<u8>) {
                    self.0.on_output(data)
                }
                fn on_exit(&self, code: Option<i32>) {
                    self.0.on_exit(code)
                }
            }
            let open = pp::TerminalOpen {
                attach: options.attach_session,
                shell: options.shell,
                cols: options.cols,
                rows: options.rows,
                env: options.env.into_iter().map(|e| (e.name, e.value)).collect(),
                origin: pp::Origin::User,
            };
            let ch = s.open_terminal(open, Arc::new(Sink(sink))).await?;
            Ok(TerminalChannel::new(ch))
        })
        .await
    }

    pub async fn list_terminals(&self) -> Result<Vec<TerminalSessionInfo>> {
        let s = self.inner.clone();
        run(async move { Ok(s.list_terminals().await?.into_iter().map(Into::into).collect()) }).await
    }

    pub async fn kill_terminal(&self, session_id: String) -> Result<()> {
        let s = self.inner.clone();
        run(async move { s.mgmt(pp::MgmtReq::KillTerminal { id: session_id }).await.map(|_| ()).map_err(Into::into) }).await
    }

    pub fn files(&self) -> Arc<FileClient> {
        Arc::new(FileClient { b: FsBackend::Peer(self.inner.clone()) })
    }

    pub async fn exec(&self, request: ExecRequest) -> Result<ExecResult> {
        let s = self.inner.clone();
        run(async move {
            let req = pp::ExecRequest {
                command: request.command,
                shell: request.shell,
                cwd: request.cwd,
                env: vec![],
                timeout_ms: if request.timeout_ms == 0 { 120_000 } else { request.timeout_ms },
                origin: pp::Origin::User,
                max_output: if request.max_output == 0 { 4 << 20 } else { request.max_output },
            };
            let (d, o, e) = s.exec(req, |_, _| {}, None).await?;
            Ok(ExecResult { code: d.code, stdout: String::from_utf8_lossy(&o).to_string(), stderr: String::from_utf8_lossy(&e).to_string(), timed_out: d.timed_out, truncated: d.truncated, duration_ms: d.duration_ms })
        })
        .await
    }

    pub async fn list_displays(&self) -> Result<Vec<DisplayInfo>> {
        let s = self.inner.clone();
        run(async move { Ok(s.list_displays().await?.into_iter().map(Into::into).collect()) }).await
    }

    pub async fn open_display(&self, options: DisplayOptions, sink: Arc<dyn VideoSink>) -> Result<Arc<DisplayChannel>> {
        let s = self.inner.clone();
        run(async move {
            struct Sink(Arc<dyn VideoSink>);
            impl cs::VideoSink for Sink {
                fn on_config(&self, codec: pp::VideoCodec, width: u32, height: u32) {
                    self.0.on_config(codec.into(), width, height)
                }
                fn on_frame(&self, data: Vec<u8>, pts_us: u64, keyframe: bool) {
                    self.0.on_frame(data, pts_us, keyframe)
                }
                fn on_cursor(&self, x: i32, y: i32, visible: bool) {
                    self.0.on_cursor(x, y, visible)
                }
            }
            let req = pp::DisplayRequest {
                display_id: options.display_id,
                codecs: if options.codecs.is_empty() { vec![pp::VideoCodec::H264] } else { options.codecs.into_iter().map(Into::into).collect() },
                max_fps: if options.max_fps == 0 { 60 } else { options.max_fps },
                bitrate_kbps: if options.bitrate_kbps == 0 { 8000 } else { options.bitrate_kbps },
                view_only: options.view_only,
            };
            let ch = s.open_display(req, Arc::new(Sink(sink.clone()))).await?;
            Ok(DisplayChannel::new(ch, sink))
        })
        .await
    }

    pub async fn screenshot(&self, display_id: u32, max_width: u32) -> Result<ImageData> {
        let s = self.inner.clone();
        run(async move { Ok(s.screenshot(display_id, max_width, max_width).await?.into()) }).await
    }

    pub async fn clipboard_get(&self) -> Result<Option<ClipboardData>> {
        let s = self.inner.clone();
        run(async move { Ok(s.clipboard_get().await?.map(Into::into)) }).await
    }

    pub async fn clipboard_set(&self, data: ClipboardData) -> Result<()> {
        let s = self.inner.clone();
        run(async move { s.clipboard_set(data.into()).await.map_err(Into::into) }).await
    }

    pub async fn forward_port(&self, local_port: u16, remote_host: String, remote_port: u16) -> Result<Arc<PortForward>> {
        let s = self.inner.clone();
        run(async move {
            let (port, h) = s.forward_port(local_port, remote_host, remote_port).await?;
            Ok(Arc::new(PortForward { port, handle: Mutex::new(Some(h)) }))
        })
        .await
    }

    pub async fn wake_on_lan(&self, mac: String) -> Result<()> {
        let s = self.inner.clone();
        run(async move { s.mgmt(pp::MgmtReq::WakeOnLan { mac }).await.map(|_| ()).map_err(Into::into) }).await
    }
}

// ---------------- 终端 ----------------

enum TermCmd {
    Write(Vec<u8>),
    Resize(u16, u16),
    Detach,
    Kill,
}

#[derive(uniffi::Object)]
pub struct TerminalChannel {
    id: String,
    title: String,
    tx: mpsc::UnboundedSender<TermCmd>,
}

impl TerminalChannel {
    fn new(ch: Arc<cs::TerminalChannel>) -> Arc<Self> {
        let (tx, mut rx) = mpsc::unbounded_channel::<TermCmd>();
        let c = ch.clone();
        // 单一写任务，保证按键顺序
        rt().spawn(async move {
            while let Some(m) = rx.recv().await {
                match m {
                    TermCmd::Write(d) => {
                        let _ = c.write(d).await;
                    }
                    TermCmd::Resize(w, h) => {
                        let _ = c.resize(w, h).await;
                    }
                    TermCmd::Detach => {
                        c.detach().await;
                        break;
                    }
                    TermCmd::Kill => {
                        c.kill().await;
                        break;
                    }
                }
            }
        });
        Arc::new(Self { id: ch.session_id.clone(), title: ch.title.clone(), tx })
    }

    /// SSH 终端（没有断线继承：断开即结束）。
    pub(crate) fn from_ssh(t: galaxy_client::ssh::SshTerminal) -> Arc<Self> {
        let (tx, mut rx) = mpsc::unbounded_channel::<TermCmd>();
        let title = t.title.clone();
        rt().spawn(async move {
            while let Some(m) = rx.recv().await {
                match m {
                    TermCmd::Write(d) => t.write(d),
                    TermCmd::Resize(w, h) => t.resize(w, h),
                    TermCmd::Detach | TermCmd::Kill => {
                        t.close();
                        break;
                    }
                }
            }
        });
        Arc::new(Self { id: format!("ssh-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]), title, tx })
    }
}

#[uniffi::export]
impl TerminalChannel {
    pub fn session_id(&self) -> String {
        self.id.clone()
    }
    pub fn title(&self) -> String {
        self.title.clone()
    }
    pub fn write(&self, data: Vec<u8>) {
        let _ = self.tx.send(TermCmd::Write(data));
    }
    pub fn resize(&self, cols: u16, rows: u16) {
        let _ = self.tx.send(TermCmd::Resize(cols, rows));
    }
    /// 断开但被控端保留会话
    pub fn detach(&self) {
        let _ = self.tx.send(TermCmd::Detach);
    }
    pub fn kill(&self) {
        let _ = self.tx.send(TermCmd::Kill);
    }
}

// ---------------- 文件 ----------------

/// 文件操作的两种来源：LemonGalaxy 设备（自有协议）与 SSH 主机（SFTP）。
#[derive(Clone)]
pub(crate) enum FsBackend {
    Peer(Arc<cs::PeerSession>),
    Ssh(Arc<galaxy_client::ssh::SshSession>),
}

#[derive(uniffi::Object)]
pub struct FileClient {
    pub(crate) b: FsBackend,
}

#[uniffi::export]
impl FileClient {
    pub async fn home(&self) -> Result<String> {
        let b = self.b.clone();
        run(async move {
            match b {
                FsBackend::Peer(s) => s.fs_home().await.map_err(Into::into),
                FsBackend::Ssh(s) => s.fs_home().await.map_err(Into::into),
            }
        })
        .await
    }

    pub async fn list(&self, path: String) -> Result<Vec<FileEntry>> {
        let b = self.b.clone();
        run(async move {
            let v = match b {
                FsBackend::Peer(s) => s.fs_list(&path).await?,
                FsBackend::Ssh(s) => s.fs_list(&path).await?,
            };
            Ok(v.into_iter().map(Into::into).collect())
        })
        .await
    }

    pub async fn stat(&self, path: String) -> Result<FileEntry> {
        let b = self.b.clone();
        run(async move {
            let e = match b {
                FsBackend::Peer(s) => s.fs_stat(&path).await?,
                FsBackend::Ssh(s) => s.fs_stat(&path).await?,
            };
            Ok(e.into())
        })
        .await
    }

    pub async fn realpath(&self, path: String) -> Result<String> {
        let b = self.b.clone();
        run(async move {
            match b {
                FsBackend::Peer(s) => {
                    let _ = s.fs_op(pp::FileOp::Realpath { path: path.clone() }).await;
                    Ok(s.fs_stat(&path).await?.path)
                }
                FsBackend::Ssh(s) => s.fs_realpath(&path).await.map_err(Into::into),
            }
        })
        .await
    }

    pub async fn mkdir(&self, path: String) -> Result<()> {
        let b = self.b.clone();
        run(async move {
            match b {
                FsBackend::Peer(s) => s.fs_op(pp::FileOp::Mkdir { path, parents: true }).await.map_err(Into::into),
                FsBackend::Ssh(s) => s.fs_mkdir(&path).await.map_err(Into::into),
            }
        })
        .await
    }

    pub async fn rename(&self, from: String, to: String) -> Result<()> {
        let b = self.b.clone();
        run(async move {
            match b {
                FsBackend::Peer(s) => s.fs_op(pp::FileOp::Rename { from, to, overwrite: false, origin: pp::Origin::User }).await.map_err(Into::into),
                FsBackend::Ssh(s) => s.fs_rename(&from, &to).await.map_err(Into::into),
            }
        })
        .await
    }

    pub async fn remove(&self, path: String, recursive: bool) -> Result<()> {
        let b = self.b.clone();
        run(async move {
            match b {
                FsBackend::Peer(s) => s.fs_op(pp::FileOp::Remove { path, recursive, origin: pp::Origin::User }).await.map_err(Into::into),
                FsBackend::Ssh(s) => s.fs_remove(&path, recursive).await.map_err(Into::into),
            }
        })
        .await
    }

    pub async fn chmod(&self, path: String, mode: u32) -> Result<()> {
        let b = self.b.clone();
        run(async move {
            match b {
                FsBackend::Peer(s) => s.fs_op(pp::FileOp::Chmod { path, mode }).await.map_err(Into::into),
                FsBackend::Ssh(s) => s.fs_chmod(&path, mode).await.map_err(Into::into),
            }
        })
        .await
    }

    pub async fn search(&self, root: String, pattern: String, content: bool) -> Result<Vec<FileEntry>> {
        let b = self.b.clone();
        run(async move {
            let v = match b {
                FsBackend::Peer(s) => s.fs_search(&root, &pattern, content, 500).await?,
                FsBackend::Ssh(s) => s.fs_search(&root, &pattern, content, 500).await?,
            };
            Ok(v.into_iter().map(Into::into).collect())
        })
        .await
    }

    pub async fn read_text(&self, path: String, max_bytes: u64) -> Result<String> {
        let b = self.b.clone();
        run(async move {
            let max = if max_bytes == 0 { 8 << 20 } else { max_bytes };
            let data = match b {
                FsBackend::Peer(s) => s.fs_read_all(&path, max).await?,
                FsBackend::Ssh(s) => s.fs_read_all(&path, max).await?,
            };
            String::from_utf8(data).map_err(|_| GalaxyError::Other { detail: "不是 UTF-8 文本文件".into() })
        })
        .await
    }

    pub async fn write_text(&self, path: String, text: String) -> Result<()> {
        let b = self.b.clone();
        run(async move {
            match b {
                FsBackend::Peer(s) => s.fs_write(&path, 0, true, None, None, pp::Origin::User, text.as_bytes()).await.map(|_| ()).map_err(Into::into),
                FsBackend::Ssh(s) => s.fs_write_all(&path, text.as_bytes()).await.map_err(Into::into),
            }
        })
        .await
    }

    /// 下载到本地文件（本地已有部分则续传）。
    pub fn download(&self, remote: String, local: String, listener: Arc<dyn TransferListener>) -> Arc<Transfer> {
        Transfer::start(self.b.clone(), TransferDir::Download, remote, local, listener)
    }

    /// 上传本地文件（远端已有部分则续传）。
    pub fn upload(&self, local: String, remote: String, listener: Arc<dyn TransferListener>) -> Arc<Transfer> {
        Transfer::start(self.b.clone(), TransferDir::Upload, remote, local, listener)
    }
}

#[derive(Clone, Copy)]
enum TransferDir {
    Download,
    Upload,
}

struct Progress(Arc<dyn TransferListener>);
impl cs::TransferProgress for Progress {
    fn on_progress(&self, done: u64, total: u64) {
        self.0.on_progress(done, total)
    }
    fn on_finished(&self, e: Option<String>) {
        self.0.on_finished(e)
    }
}

#[derive(uniffi::Object)]
pub struct Transfer {
    s: FsBackend,
    dir: TransferDir,
    remote: String,
    local: String,
    listener: Arc<dyn TransferListener>,
    cancel: Mutex<Option<watch::Sender<bool>>>,
    paused: Mutex<bool>,
}

impl Transfer {
    fn start(s: FsBackend, dir: TransferDir, remote: String, local: String, listener: Arc<dyn TransferListener>) -> Arc<Self> {
        let t = Arc::new(Self { s, dir, remote, local, listener, cancel: Mutex::new(None), paused: Mutex::new(false) });
        t.clone().launch();
        t
    }

    fn launch(self: Arc<Self>) {
        let (tx, rx) = watch::channel(false);
        *self.cancel.lock() = Some(tx);
        let me = self.clone();
        rt().spawn(async move {
            let p: Arc<dyn cs::TransferProgress> = Arc::new(Progress(me.listener.clone()));
            let local = std::path::Path::new(&me.local);
            let r = match (&me.s, me.dir) {
                (FsBackend::Peer(s), TransferDir::Download) => s.download(&me.remote, local, p, rx).await,
                (FsBackend::Peer(s), TransferDir::Upload) => s.upload(local, &me.remote, p, rx).await,
                (FsBackend::Ssh(s), TransferDir::Download) => s.download(&me.remote, local, p, rx).await,
                (FsBackend::Ssh(s), TransferDir::Upload) => s.upload(local, &me.remote, p, rx).await,
            };
            match r {
                Ok(()) => me.listener.on_finished(None),
                Err(galaxy_client::ClientError::Cancelled) if *me.paused.lock() => {}
                Err(e) => me.listener.on_finished(Some(e.to_string())),
            }
        });
    }
}

#[uniffi::export]
impl Transfer {
    pub fn cancel(&self) {
        *self.paused.lock() = false;
        if let Some(c) = self.cancel.lock().as_ref() {
            let _ = c.send(true);
        }
    }
    pub fn pause(&self) {
        *self.paused.lock() = true;
        if let Some(c) = self.cancel.lock().as_ref() {
            let _ = c.send(true);
        }
    }
    pub fn resume(self: Arc<Self>) {
        if *self.paused.lock() {
            *self.paused.lock() = false;
            self.launch();
        }
    }
}

// ---------------- 远程画面 ----------------

enum DispCmd {
    Input(pp::InputEvent),
    Keyframe,
    Bitrate(u32),
    Stop,
}

#[derive(uniffi::Object)]
pub struct DisplayChannel {
    tx: mpsc::UnboundedSender<DispCmd>,
}

impl DisplayChannel {
    fn new(ch: Arc<cs::DisplayChannel>, sink: Arc<dyn VideoSink>) -> Arc<Self> {
        let (tx, mut rx) = mpsc::unbounded_channel::<DispCmd>();
        rt().spawn(async move {
            while let Some(m) = rx.recv().await {
                match m {
                    DispCmd::Input(e) => ch.send_input(e).await,
                    DispCmd::Keyframe => ch.request_keyframe().await,
                    DispCmd::Bitrate(k) => ch.set_bitrate(k).await,
                    DispCmd::Stop => {
                        ch.stop().await;
                        sink.on_closed("stopped".into());
                        break;
                    }
                }
            }
        });
        Arc::new(Self { tx })
    }
}

#[uniffi::export]
impl DisplayChannel {
    pub fn send_input(&self, event: InputEvent) {
        let _ = self.tx.send(DispCmd::Input(event.into()));
    }
    pub fn request_keyframe(&self) {
        let _ = self.tx.send(DispCmd::Keyframe);
    }
    pub fn set_bitrate(&self, kbps: u32) {
        let _ = self.tx.send(DispCmd::Bitrate(kbps));
    }
    pub fn stop(&self) {
        let _ = self.tx.send(DispCmd::Stop);
    }
}

#[derive(uniffi::Object)]
pub struct PortForward {
    pub(crate) port: u16,
    pub(crate) handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

#[uniffi::export]
impl PortForward {
    pub fn local_port(&self) -> u16 {
        self.port
    }
    pub fn stop(&self) {
        if let Some(h) = self.handle.lock().take() {
            h.abort();
        }
    }
}
