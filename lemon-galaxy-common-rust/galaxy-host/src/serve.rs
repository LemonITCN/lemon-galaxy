//! 会话内的功能流分派：权限检查、AI 上限与绝对禁止清单、各功能服务。

use crate::files::{self, FileCtx};
use crate::fio;
use crate::platform::FrameSink;
use crate::service::{ActiveSession, HostService};
use crate::term::TermEvent;
use bytes::Bytes;
use galaxy_agent::risk::{self, FileAction, ShellKind};
use galaxy_proto::peer::*;
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

static NEXT_CHANNEL: AtomicU8 = AtomicU8::new(1);

pub async fn run(host: Arc<HostService>, conn: quinn::Connection, sess: Arc<ActiveSession>) {
    loop {
        let (send, recv) = match conn.accept_bi().await {
            Ok(x) => x,
            Err(_) => break,
        };
        let host = host.clone();
        let sess = sess.clone();
        let conn = conn.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(host, conn, sess, send, recv).await {
                tracing::debug!("功能流结束: {e}");
            }
        });
    }
}

fn denied(msg: &str) -> PeerError {
    PeerError::denied(msg)
}

fn ai_gate(host: &HostService, sess: &ActiveSession, origin: &Origin, a: &risk::Assessment, what: &str) -> Result<(), PeerError> {
    if !origin.is_ai() {
        return Ok(());
    }
    {
        let mut i = sess.info.write();
        if !i.ai_active {
            i.ai_active = true;
            drop(i);
            host.emit_status();
        }
    }
    match risk::host_permits(sess.policy.ai_cap, a.level) {
        Ok(()) => {
            host.log(&sess.info.read().id, &sess.from_user_name, "ai", what, true);
            Ok(())
        }
        Err(m) => {
            host.log(&sess.info.read().id, &sess.from_user_name, "ai_denied", &format!("{what}：{m}"), true);
            Err(PeerError::new("ai_denied", m))
        }
    }
}

fn shell_kind(shell: &str) -> ShellKind {
    let s = shell.to_ascii_lowercase();
    if s.contains("powershell") || s.contains("pwsh") {
        ShellKind::PowerShell
    } else if s.ends_with("cmd") || s.ends_with("cmd.exe") {
        ShellKind::Cmd
    } else {
        ShellKind::Posix
    }
}

async fn handle(host: Arc<HostService>, conn: quinn::Connection, sess: Arc<ActiveSession>, mut send: quinn::SendStream, mut recv: quinn::RecvStream) -> anyhow::Result<()> {
    let Some(open) = fio::read::<_, StreamOpen>(&mut recv).await? else { return Ok(()) };
    tracing::debug!("功能流 {:?}", std::mem::discriminant(&open));
    let p = &sess.policy;
    let sid = sess.info.read().id.clone();
    match open {
        StreamOpen::Terminal(t) => {
            if !p.terminal {
                fio::write(&mut send, &TermOut::Error(denied("没有终端权限"))).await?;
                return Ok(());
            }
            if t.origin.is_ai() {
                if let Err(e) = ai_gate(&host, &sess, &t.origin, &risk::Assessment { level: risk::Risk::Normal, reasons: vec![], commands: vec![] }, "打开交互式终端") {
                    fio::write(&mut send, &TermOut::Error(e)).await?;
                    return Ok(());
                }
            }
            let acct = sess.account.unwrap_or(0);
            let (term, existing) = match t.attach.as_deref().and_then(|id| host.terms.get(id, acct)) {
                Some(s) if s.exited().is_none() => (s, true),
                _ => match host.terms.spawn(&sess.user, t.shell.clone(), t.cols, t.rows, &t.env, acct) {
                    Ok(s) => (s, false),
                    Err(e) => {
                        fio::write(&mut send, &TermOut::Error(PeerError::new("spawn", format!("无法启动终端：{e}")))).await?;
                        return Ok(());
                    }
                },
            };
            if existing {
                term.resize(t.cols, t.rows);
            }
            host.log(&sid, &sess.from_user_name, "terminal", if existing { "接续终端会话" } else { "打开终端" }, t.origin.is_ai());
            fio::write(&mut send, &TermOut::Opened { session_id: term.id.clone(), title: term.title.clone(), attached_existing: existing }).await?;
            let (ev_tx, mut ev_rx) = mpsc::channel::<TermEvent>(256);
            let replay = term.attach(ev_tx);
            let (out_tx, mut out_rx) = mpsc::channel::<TermOut>(256);
            if !replay.is_empty() {
                let _ = out_tx.send(TermOut::Output(replay)).await;
            }
            let fwd_tx = out_tx.clone();
            tokio::spawn(async move {
                while let Some(e) = ev_rx.recv().await {
                    let m = match e {
                        TermEvent::Output(d) => TermOut::Output(d),
                        TermEvent::Exited(c) => TermOut::Exited { code: c },
                    };
                    if fwd_tx.send(m).await.is_err() {
                        break;
                    }
                }
            });
            let writer = tokio::spawn(async move {
                while let Some(m) = out_rx.recv().await {
                    let end = matches!(m, TermOut::Exited { .. });
                    if fio::write(&mut send, &m).await.is_err() || end {
                        break;
                    }
                }
                let _ = send.finish();
            });
            let mut killed = false;
            loop {
                match fio::read::<_, TermIn>(&mut recv).await {
                    Ok(Some(TermIn::Input(d))) => {
                        let _ = term.write(&d).await;
                    }
                    Ok(Some(TermIn::Resize { cols, rows })) => term.resize(cols, rows),
                    Ok(Some(TermIn::Snapshot)) => {
                        let _ = out_tx.send(TermOut::Snapshot(term.snapshot())).await;
                    }
                    Ok(Some(TermIn::Detach)) => break,
                    Ok(Some(TermIn::Kill)) => {
                        host.terms.kill(&term.id, acct);
                        killed = true;
                        break;
                    }
                    Ok(None) | Err(_) => break,
                }
            }
            if !killed {
                // 断线或主动断开：会话保留在被控端，下次可接续
                term.detach();
            }
            drop(out_tx);
            writer.abort();
        }
        StreamOpen::Mgmt => {
            while let Some(req) = fio::read::<_, MgmtReq>(&mut recv).await? {
                let resp = mgmt(&host, &sess, req).await;
                fio::write(&mut send, &resp).await?;
            }
            let _ = send.finish();
        }
        StreamOpen::Files(op) => {
            let reply_err = |e: PeerError| FileMsg::Error(e);
            if !p.files {
                fio::write(&mut send, &reply_err(denied("没有文件权限"))).await?;
                let _ = send.finish();
                return Ok(());
            }
            let ctx = FileCtx { home: sess.user.home.clone().into(), backup_dir: Some(host.cfg.data_dir.join("ai-backup")) };
            // AI 检查
            let (action, path, origin) = match &op {
                FileOp::Read { path, .. } => (FileAction::Read, path.clone(), None),
                FileOp::List { path } | FileOp::Stat { path } | FileOp::Realpath { path } => (FileAction::List, path.clone(), None),
                FileOp::Search { root, .. } => (FileAction::List, root.clone(), None),
                FileOp::Home => (FileAction::List, String::new(), None),
                FileOp::Write { path, origin, .. } => (FileAction::Write { exists: files::resolve(&ctx, path).exists() }, path.clone(), Some(origin.clone())),
                FileOp::Mkdir { path, .. } => (FileAction::Write { exists: false }, path.clone(), None),
                FileOp::Remove { path, recursive, origin } => (FileAction::Delete { recursive: *recursive }, path.clone(), Some(origin.clone())),
                FileOp::Rename { from, origin, .. } => (FileAction::Rename, from.clone(), Some(origin.clone())),
                FileOp::Chmod { path, .. } => (FileAction::Chmod, path.clone(), None),
            };
            if let Some(o) = &origin {
                let full = files::resolve(&ctx, &path).to_string_lossy().to_string();
                let a = risk::assess_file(action, &full);
                if let Err(e) = ai_gate(&host, &sess, o, &a, &format!("{action:?} {full}")) {
                    fio::write(&mut send, &reply_err(e)).await?;
                    let _ = send.finish();
                    return Ok(());
                }
            }
            if matches!(action, FileAction::Write { .. } | FileAction::Delete { .. } | FileAction::Rename | FileAction::Chmod) && origin.as_ref().map(|o| !o.is_ai()).unwrap_or(true) {
                host.log(&sid, &sess.from_user_name, "file", &format!("{action:?} {path}"), false);
            }
            if sess.user.needs_switch() {
                match &host.cfg.file_helper {
                    Some(exe) => run_helper(exe, &sess, op, &mut recv, &mut send).await?,
                    None => {
                        fio::write(&mut send, &reply_err(PeerError::new("unsupported", "无法以目标系统用户身份操作文件"))).await?;
                    }
                }
            } else {
                files::serve(op, &mut recv, &mut send, &ctx).await?;
            }
            let _ = send.finish();
        }
        StreamOpen::Exec(req) => {
            if !p.terminal {
                fio::write(&mut send, &ExecOut::Error(denied("没有终端权限"))).await?;
                let _ = send.finish();
                return Ok(());
            }
            let shell = req.shell.clone().unwrap_or_else(|| sess.user.shell.clone());
            if req.origin.is_ai() {
                let a = risk::assess_command(&req.command, shell_kind(&shell));
                if let Err(e) = ai_gate(&host, &sess, &req.origin, &a, &format!("执行：{}", req.command)) {
                    fio::write(&mut send, &ExecOut::Error(e)).await?;
                    let _ = send.finish();
                    return Ok(());
                }
            } else {
                host.log(&sid, &sess.from_user_name, "exec", &req.command, false);
            }
            let (out_tx, mut out_rx) = mpsc::channel::<ExecOut>(256);
            let (ctl_tx, ctl_rx) = mpsc::channel::<ExecIn>(16);
            let user = sess.user.clone();
            let mut ai_stop = host.ai_stop_watch();
            let is_ai = req.origin.is_ai();
            let runner = tokio::spawn(async move { crate::exec::run(req, &user, out_tx, ctl_rx).await });
            let ctl2 = ctl_tx.clone();
            let reader = tokio::spawn(async move {
                while let Ok(Some(m)) = fio::read::<_, ExecIn>(&mut recv).await {
                    if ctl2.send(m).await.is_err() {
                        break;
                    }
                }
            });
            loop {
                tokio::select! {
                    m = out_rx.recv() => {
                        let Some(m) = m else { break };
                        let end = matches!(m, ExecOut::Done(_) | ExecOut::Error(_));
                        if fio::write(&mut send, &m).await.is_err() { break; }
                        if end { break; }
                    }
                    r = ai_stop.changed(), if is_ai => {
                        if r.is_ok() { let _ = ctl_tx.send(ExecIn::Cancel).await; }
                    }
                }
            }
            let _ = send.finish();
            reader.abort();
            let _ = runner.await;
        }
        StreamOpen::PortForward { host: h, port } => {
            if !p.terminal {
                fio::write(&mut send, &ForwardReply::Error(denied("没有端口映射权限"))).await?;
                return Ok(());
            }
            host.log(&sid, &sess.from_user_name, "forward", &format!("{h}:{port}"), false);
            match tokio::time::timeout(Duration::from_secs(10), tokio::net::TcpStream::connect((h.as_str(), port))).await {
                Ok(Ok(mut tcp)) => {
                    fio::write(&mut send, &ForwardReply::Ok).await?;
                    let (mut tr, mut tw) = tcp.split();
                    let a = async {
                        let _ = tokio::io::copy(&mut recv, &mut tw).await;
                    };
                    let b = async {
                        let _ = tokio::io::copy(&mut tr, &mut send).await;
                        let _ = send.finish();
                    };
                    tokio::join!(a, b);
                }
                _ => {
                    fio::write(&mut send, &ForwardReply::Error(PeerError::new("connect", format!("被控端连不上 {h}:{port}")))).await?;
                }
            }
        }
        StreamOpen::Display(req) => {
            if !p.view {
                fio::write(&mut send, &DisplayOut::Error(denied("没有查看画面的权限"))).await?;
                return Ok(());
            }
            let channel = NEXT_CHANNEL.fetch_add(1, Ordering::Relaxed).max(1);
            let sink = Arc::new(VideoSender { conn: conn.clone(), channel, frame: AtomicU32::new(0), ctl: parking_lot::Mutex::new(None) });
            let (ctl_tx, mut ctl_rx) = mpsc::channel::<DisplayOut>(64);
            *sink.ctl.lock() = Some(ctl_tx);
            let platform = host.platform.read().clone();
            let started = platform.start_capture(req.display_id, req.max_fps.clamp(1, 240), req.bitrate_kbps.clamp(200, 200_000), req.codecs.clone(), sink.clone());
            match started {
                Ok(s) => {
                    host.log(&sid, &sess.from_user_name, "display", &format!("查看屏幕 {}", req.display_id), false);
                    sess.used_display.store(true, Ordering::Relaxed);
                    fio::write(&mut send, &DisplayOut::Started { width: s.width, height: s.height, codec: s.codec, channel }).await?;
                }
                Err(e) => {
                    fio::write(&mut send, &DisplayOut::Error(PeerError::new("unsupported", e))).await?;
                    return Ok(());
                }
            }
            let can_control = p.control && !req.view_only;
            let writer = tokio::spawn(async move {
                while let Some(m) = ctl_rx.recv().await {
                    if fio::write(&mut send, &m).await.is_err() {
                        break;
                    }
                }
            });
            // 码率自适应：每秒看一次连接的丢包与拥塞
            let rate = Arc::new(parking_lot::Mutex::new(crate::rate::RateCtl::new(req.bitrate_kbps.clamp(200, 200_000), std::time::Instant::now())));
            let adapt = {
                let (rate, conn, platform, display) = (rate.clone(), conn.clone(), platform.clone(), req.display_id);
                tokio::spawn(async move {
                    let mut max_free = conn.datagram_send_buffer_space().max(1);
                    loop {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        let st = conn.stats();
                        let free = conn.datagram_send_buffer_space();
                        max_free = max_free.max(free);
                        let sample = crate::rate::Sample {
                            sent_packets: st.path.sent_packets,
                            lost_packets: st.path.lost_packets,
                            congestion_events: st.path.congestion_events,
                            buffer_free: free as f32 / max_free as f32,
                        };
                        let next = rate.lock().on_sample(sample, std::time::Instant::now());
                        if let Some(k) = next {
                            tracing::debug!("画面码率自适应 → {k} kbps（丢包 {} / 发送 {}）", st.path.lost_packets, st.path.sent_packets);
                            let p = platform.clone();
                            let _ = tokio::task::spawn_blocking(move || p.set_bitrate(display, k)).await;
                        }
                    }
                })
            };
            while let Ok(Some(m)) = fio::read::<_, DisplayIn>(&mut recv).await {
                match m {
                    DisplayIn::Input(ev) if can_control => platform.inject(req.display_id, ev),
                    DisplayIn::Input(_) => {}
                    DisplayIn::RequestKeyframe => platform.request_keyframe(req.display_id),
                    DisplayIn::SetBitrate(k) => {
                        let k = rate.lock().set_ceiling(k.clamp(200, 200_000), std::time::Instant::now());
                        platform.set_bitrate(req.display_id, k);
                    }
                    DisplayIn::Stop => break,
                }
            }
            adapt.abort();
            platform.stop_capture(req.display_id);
            *sink.ctl.lock() = None;
            writer.abort();
        }
        StreamOpen::Audio(req) => {
            if !p.audio {
                fio::write(&mut send, &AudioOut::Error(denied("没有听声音的权限"))).await?;
                return Ok(());
            }
            if !req.codecs.contains(&AudioCodec::Opus) {
                fio::write(&mut send, &AudioOut::Error(PeerError::new("unsupported", "没有双方都支持的声音编码"))).await?;
                return Ok(());
            }
            let channel = NEXT_CHANNEL.fetch_add(1, Ordering::Relaxed).max(1);
            let platform = host.platform.read().clone();
            let id = match host.audio.subscribe(&platform, conn.clone(), channel, req.bitrate_kbps) {
                Ok(id) => id,
                Err(e) => {
                    fio::write(&mut send, &AudioOut::Error(PeerError::new("unsupported", e))).await?;
                    return Ok(());
                }
            };
            host.log(&sid, &sess.from_user_name, "audio", "收听声音", false);
            let r = fio::write(
                &mut send,
                &AudioOut::Started { codec: AudioCodec::Opus, sample_rate: crate::audio::SAMPLE_RATE, channels: crate::audio::CHANNELS as u8, channel },
            )
            .await;
            if r.is_ok() {
                // 主控端发 Stop、关流或断线都算结束
                while let Ok(Some(m)) = fio::read::<_, AudioIn>(&mut recv).await {
                    match m {
                        AudioIn::Stop => break,
                    }
                }
            }
            host.audio.unsubscribe(id);
        }
        StreamOpen::Clipboard => {
            fio::write(&mut send, &MgmtResp::Error(PeerError::new("unsupported", "请使用管理请求同步剪贴板"))).await?;
        }
    }
    Ok(())
}

async fn mgmt(host: &Arc<HostService>, sess: &Arc<ActiveSession>, req: MgmtReq) -> MgmtResp {
    let p = &sess.policy;
    let acct = sess.account.unwrap_or(0);
    let platform = host.platform.read().clone();
    match req {
        MgmtReq::ListTerminals => MgmtResp::Terminals(host.terms.list(acct)),
        MgmtReq::KillTerminal { id } => {
            if host.terms.kill(&id, acct) {
                MgmtResp::Ok
            } else {
                MgmtResp::Error(PeerError::new("not_found", "终端会话不存在"))
            }
        }
        MgmtReq::ListDisplays => {
            if !p.view {
                return MgmtResp::Error(denied("没有查看画面的权限"));
            }
            MgmtResp::Displays(platform.list_displays())
        }
        MgmtReq::Screenshot { display, max_width, max_height } => {
            if !p.view {
                return MgmtResp::Error(denied("没有查看画面的权限"));
            }
            match tokio::task::spawn_blocking(move || platform.screenshot(display, max_width, max_height)).await.ok().flatten() {
                Some(i) => MgmtResp::Image(i),
                None => MgmtResp::Error(PeerError::new("unsupported", "无法截屏（设备没有图形界面或未授权屏幕录制）")),
            }
        }
        MgmtReq::UiTree { display, max_nodes } => {
            if !p.view {
                return MgmtResp::Error(denied("没有查看画面的权限"));
            }
            match tokio::task::spawn_blocking(move || platform.ui_tree(display, max_nodes)).await.ok().flatten() {
                Some(t) => MgmtResp::Text(t),
                None => MgmtResp::Error(PeerError::new("unsupported", "无法读取界面控件（未授权辅助功能或不支持）")),
            }
        }
        MgmtReq::ClipboardGet => {
            if !p.clipboard {
                return MgmtResp::Error(denied("没有剪贴板权限"));
            }
            MgmtResp::Clipboard(platform.clipboard_get())
        }
        MgmtReq::ClipboardSet(d) => {
            if !p.clipboard {
                return MgmtResp::Error(denied("没有剪贴板权限"));
            }
            platform.clipboard_set(d);
            MgmtResp::Ok
        }
        MgmtReq::WakeOnLan { mac } => {
            if !p.control && !p.terminal {
                return MgmtResp::Error(denied("没有权限"));
            }
            match wake_on_lan(&mac).await {
                Ok(()) => MgmtResp::Ok,
                Err(e) => MgmtResp::Error(PeerError::new("wol", e)),
            }
        }
        MgmtReq::HostStatus => MgmtResp::Text(serde_json::to_string(&host.status()).unwrap_or_default()),
        MgmtReq::SetPrivacy { on } => {
            if !p.control {
                return MgmtResp::Error(denied("没有控制权限"));
            }
            let sid = sess.info.read().id.clone();
            if platform.set_privacy(on) {
                *host.privacy_owner.lock() = on.then_some(sid.clone());
                host.log(&sid, &sess.from_user_name, "privacy", if on { "开启隐私屏" } else { "关闭隐私屏" }, false);
                MgmtResp::Ok
            } else {
                MgmtResp::Error(PeerError::new("unsupported", "被控端不支持隐私屏"))
            }
        }
        MgmtReq::Inject { display, events, origin } => {
            if !p.control {
                return MgmtResp::Error(denied("没有控制键鼠的权限"));
            }
            let a = risk::Assessment { level: risk::Risk::Normal, reasons: vec![], commands: vec![] };
            if let Err(e) = ai_gate(host, sess, &origin, &a, &format!("图形界面操作 {} 步", events.len())) {
                return MgmtResp::Error(e);
            }
            let n = events.len();
            tokio::task::spawn_blocking(move || {
                for ev in events {
                    platform.inject(display, ev);
                    std::thread::sleep(std::time::Duration::from_millis(8));
                }
            })
            .await
            .ok();
            let _ = n;
            MgmtResp::Ok
        }
    }
}

async fn wake_on_lan(mac: &str) -> Result<(), String> {
    galaxy_net::wol::send_magic(mac).await
}

async fn run_helper(exe: &std::path::Path, sess: &ActiveSession, op: FileOp, recv: &mut quinn::RecvStream, send: &mut quinn::SendStream) -> anyhow::Result<()> {
    let u = &sess.user;
    let backup = std::path::Path::new(&u.home).join(".lemon-galaxy").join("ai-backup").to_string_lossy().to_string();
    #[cfg(not(windows))]
    let (mut stdin, mut stdout, mut child) = {
        let mut child = tokio::process::Command::new(exe)
            .args(["__lg-file-helper", &u.uid.to_string(), &u.gid.to_string(), &u.home, &backup])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        (child.stdin.take().unwrap(), child.stdout.take().unwrap(), child)
    };
    #[cfg(windows)]
    let (mut stdin, mut stdout, child, _token) = {
        let acc = crate::win::Account { name: u.name.clone(), domain: u.domain.clone(), sid: u.sid.clone() };
        let token = crate::win::token_for(&acc)?;
        let exe_s = exe.to_string_lossy().to_string();
        let cmd = crate::win::cmdline(&exe_s, &["__lg-file-helper", "0", "0", &u.home, &backup]);
        let (child, i, o, _e) = crate::win::spawn_piped(cmd, Some(u.home.clone()), Some(&token), &[])?;
        (i, o, child, token)
    };
    let is_write = matches!(op, FileOp::Write { .. });
    fio::write(&mut stdin, &op).await?;
    let up = async {
        if is_write {
            let _ = tokio::io::copy(recv, &mut stdin).await;
        }
        drop(stdin);
    };
    let down = async {
        let _ = tokio::io::copy(&mut stdout, send).await;
    };
    tokio::join!(up, down);
    #[cfg(not(windows))]
    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    #[cfg(windows)]
    {
        if tokio::time::timeout(Duration::from_secs(5), child.wait()).await.is_err() {
            child.kill();
        }
    }
    Ok(())
}

/// 把平台层编码好的帧切片成数据报发出。
struct VideoSender {
    conn: quinn::Connection,
    channel: u8,
    frame: AtomicU32,
    ctl: parking_lot::Mutex<Option<mpsc::Sender<DisplayOut>>>,
}

impl FrameSink for VideoSender {
    fn push(&self, data: Vec<u8>, pts_us: u64, keyframe: bool) {
        let max = self.conn.max_datagram_size().unwrap_or(1100).saturating_sub(VideoFragHeader::BASE_LEN + 8).max(256);
        let frame_id = self.frame.fetch_add(1, Ordering::Relaxed);
        let count = data.len().div_ceil(max).max(1);
        if count > u16::MAX as usize {
            return;
        }
        for (i, chunk) in data.chunks(max).enumerate() {
            let h = VideoFragHeader { channel: self.channel, frame_id, frag_idx: i as u16, frag_count: count as u16, keyframe, pts_us };
            let mut b = Vec::with_capacity(chunk.len() + 20);
            h.encode(&mut b);
            b.extend_from_slice(chunk);
            if self.conn.send_datagram(Bytes::from(b)).is_err() {
                break;
            }
        }
    }
    fn resized(&self, width: u32, height: u32) {
        if let Some(tx) = self.ctl.lock().as_ref() {
            let _ = tx.try_send(DisplayOut::Resized { width, height });
        }
    }
}
