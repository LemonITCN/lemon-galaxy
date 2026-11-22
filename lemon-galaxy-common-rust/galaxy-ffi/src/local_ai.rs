//! AI 的「本机」目标：桌面客户端在自己进程里直接执行，复用被控端（galaxy-host）的同一套
//! 命令执行、文件读写（含 AI 改动前备份）、终端与截图 / 控件树 / 输入注入代码，不经过网络。
//! 每个动作照样记进被控端日志（标记为 AI），本机被控页面的记录里能看到。

use galaxy_agent::agent::{BoxFut, CmdResult};
use galaxy_client::ai::{LocalHost, LocalMeta};
use galaxy_host::files::FileCtx;
use galaxy_host::sysuser::SysUser;
use galaxy_proto::peer::{DisplayInfo, ExecIn, ExecOut, ExecRequest, FileMsg, FileOp, ImageData, InputEvent, Origin};
use std::sync::Arc;
use tokio::sync::{mpsc, watch};

/// 本机终端归在一个不存在的账号下，远端用户列不到、接不上
const LOCAL_TERM_ACCOUNT: u64 = u64::MAX;

pub struct LocalAi {
    /// 进程内的本机被控服务（Mac 主控端有；Windows 主控端的被控是系统服务，这里为空）
    pub host: Option<Arc<galaxy_host::HostService>>,
    pub platform: galaxy_proto::Platform,
    pub os: String,
    pub user_name: String,
}

/// 没有进程内被控服务时，本机 AI 终端自己管
static OWN_TERMS: std::sync::OnceLock<Arc<galaxy_host::term::TermManager>> = std::sync::OnceLock::new();

impl LocalAi {
    fn terms(&self) -> Arc<galaxy_host::term::TermManager> {
        match &self.host {
            Some(h) => h.terms.clone(),
            None => OWN_TERMS.get_or_init(|| Arc::new(galaxy_host::term::TermManager::default())).clone(),
        }
    }

    fn hp(&self) -> Arc<dyn galaxy_host::platform::HostPlatform> {
        match &self.host {
            Some(h) => h.platform.read().clone(),
            None => Arc::new(galaxy_host::platform::NoPlatform),
        }
    }

    fn user(&self) -> SysUser {
        SysUser::current()
    }

    fn log(&self, kind: &str, detail: &str) {
        match &self.host {
            Some(h) => h.log("local-ai", &self.user_name, kind, detail, true),
            None => tracing::info!("本机 AI {kind}：{detail}"),
        }
    }

    fn ctx(&self) -> FileCtx {
        let u = self.user();
        FileCtx { home: u.home.clone().into(), backup_dir: Some(std::path::Path::new(&u.home).join(".lemon-galaxy").join("ai-backup")) }
    }
}

fn op_detail(op: &FileOp) -> Option<(&'static str, String)> {
    match op {
        FileOp::Write { path, .. } => Some(("file_write", format!("写入 {path}"))),
        FileOp::Remove { path, .. } => Some(("file_remove", format!("删除 {path}"))),
        FileOp::Rename { from, to, .. } => Some(("file_rename", format!("{from} → {to}"))),
        FileOp::Mkdir { path, .. } => Some(("file_mkdir", format!("新建目录 {path}"))),
        _ => None,
    }
}

impl LocalHost for LocalAi {
    fn meta(&self) -> LocalMeta {
        let u = self.user();
        let hp = self.hp();
        let gui = !hp.list_displays().is_empty() || cfg!(windows);
        let computer_use = hp.capabilities().iter().any(|c| c == galaxy_proto::peer::CAP_COMPUTER_USE) || cfg!(windows);
        LocalMeta { platform: self.platform, os: self.os.clone(), user: u.name, shell: u.shell, gui, computer_use }
    }

    fn exec(&self, req: ExecRequest, on_out: Box<dyn Fn(&[u8]) + Send + Sync>, mut cancel: watch::Receiver<bool>) -> BoxFut<'_, anyhow::Result<CmdResult>> {
        Box::pin(async move {
            self.log("exec", &req.command);
            let user = self.user();
            let (out_tx, mut out_rx) = mpsc::channel::<ExecOut>(64);
            let (ctl_tx, ctl_rx) = mpsc::channel::<ExecIn>(4);
            let runner = tokio::spawn(async move { galaxy_host::exec::run(req, &user, out_tx, ctl_rx).await });
            let (mut so, mut se) = (Vec::new(), Vec::new());
            let mut done = None;
            loop {
                tokio::select! {
                    m = out_rx.recv() => match m {
                        Some(ExecOut::Stdout(d)) => { on_out(&d); so.extend_from_slice(&d); }
                        Some(ExecOut::Stderr(d)) => { on_out(&d); se.extend_from_slice(&d); }
                        Some(ExecOut::Done(d)) => { done = Some(d); }
                        Some(ExecOut::Error(e)) => anyhow::bail!("{}", e.message),
                        Some(ExecOut::Started) => {}
                        None => break,
                    },
                    r = cancel.changed() => {
                        if r.is_err() || *cancel.borrow() {
                            let _ = ctl_tx.send(ExecIn::Cancel).await;
                        }
                    }
                }
            }
            let _ = runner.await;
            let d = done.ok_or_else(|| anyhow::anyhow!("命令没有正常结束"))?;
            Ok(CmdResult {
                code: d.code,
                stdout: String::from_utf8_lossy(&so).to_string(),
                stderr: String::from_utf8_lossy(&se).to_string(),
                timed_out: d.timed_out,
                truncated: d.truncated,
                duration_ms: d.duration_ms,
            })
        })
    }

    fn file(&self, op: FileOp, upload: Option<Vec<u8>>) -> BoxFut<'_, anyhow::Result<Vec<FileMsg>>> {
        Box::pin(async move {
            if let Some((kind, detail)) = op_detail(&op) {
                self.log(kind, &detail);
            }
            let ctx = self.ctx();
            let (client, server) = tokio::io::duplex(1024 * 1024);
            let (mut sr, mut sw) = tokio::io::split(server);
            let srv = tokio::spawn(async move {
                let r = galaxy_host::files::serve(op, &mut sr, &mut sw, &ctx).await;
                drop(sw);
                r
            });
            let (mut cr, mut cw) = tokio::io::split(client);
            let writer = tokio::spawn(async move {
                if let Some(data) = upload {
                    for c in data.chunks(256 * 1024) {
                        galaxy_host::fio::write(&mut cw, &FileMsg::Chunk(c.to_vec())).await?;
                    }
                    galaxy_host::fio::write(&mut cw, &FileMsg::End).await?;
                }
                anyhow::Ok(cw)
            });
            let mut msgs = vec![];
            while let Some(m) = galaxy_host::fio::read::<_, FileMsg>(&mut cr).await? {
                msgs.push(m);
            }
            let _ = writer.await;
            srv.await.map_err(|e| anyhow::anyhow!("{e}"))??;
            Ok(msgs)
        })
    }

    fn displays(&self) -> Vec<DisplayInfo> {
        self.hp().list_displays()
    }

    fn screenshot(&self, display: u32, max_w: u32, max_h: u32) -> BoxFut<'_, anyhow::Result<ImageData>> {
        Box::pin(async move {
            let p = self.hp();
            tokio::task::spawn_blocking(move || p.screenshot(display, max_w, max_h))
                .await
                .ok()
                .flatten()
                .ok_or_else(|| anyhow::anyhow!("无法截屏（没有授权屏幕录制）"))
        })
    }

    fn ui_tree(&self, display: u32, max_nodes: u32) -> BoxFut<'_, anyhow::Result<String>> {
        Box::pin(async move {
            let p = self.hp();
            tokio::task::spawn_blocking(move || p.ui_tree(display, max_nodes))
                .await
                .ok()
                .flatten()
                .ok_or_else(|| anyhow::anyhow!("无法读取界面控件（没有授权辅助功能）"))
        })
    }

    fn inject(&self, display: u32, events: Vec<InputEvent>, _origin: Origin) -> BoxFut<'_, anyhow::Result<()>> {
        Box::pin(async move {
            self.log("input", &format!("图形界面操作 {} 步", events.len()));
            let p = self.hp();
            tokio::task::spawn_blocking(move || {
                for ev in events {
                    p.inject(display, ev);
                    std::thread::sleep(std::time::Duration::from_millis(8));
                }
            })
            .await
            .ok();
            Ok(())
        })
    }

    fn computer_use(&self, request: String, _origin: Origin) -> BoxFut<'_, anyhow::Result<(String, Option<ImageData>)>> {
        Box::pin(async move {
            self.log("computer_use", &galaxy_agent::agent::truncate(&request, 300));
            let p = self.hp();
            let r = tokio::task::spawn_blocking(move || {
                // Windows 主控端没有进程内被控服务：直接用共用的 Windows 实现
                #[cfg(windows)]
                if let Some(r) = galaxy_host::win_cu::handle(&request) {
                    return Some(r);
                }
                p.computer_use(&request)
            })
            .await
            .ok()
            .flatten();
            r.ok_or_else(|| anyhow::anyhow!("这台电脑不支持应用操作"))
        })
    }

    fn term_open(&self, _origin: Origin) -> BoxFut<'_, anyhow::Result<String>> {
        Box::pin(async move {
            self.log("terminal", "打开交互式终端");
            let t = self.terms().spawn(&self.user(), None, 120, 36, &[("TERM".into(), "xterm-256color".into())], LOCAL_TERM_ACCOUNT)?;
            Ok(t.id.clone())
        })
    }

    fn term_write(&self, id: &str, data: Vec<u8>) -> BoxFut<'_, anyhow::Result<()>> {
        let id = id.to_string();
        Box::pin(async move {
            let t = self.terms().get(&id, LOCAL_TERM_ACCOUNT).ok_or_else(|| anyhow::anyhow!("终端不存在"))?;
            t.write(&data).await?;
            Ok(())
        })
    }

    fn term_snapshot(&self, id: &str) -> BoxFut<'_, anyhow::Result<String>> {
        let id = id.to_string();
        Box::pin(async move {
            let t = self.terms().get(&id, LOCAL_TERM_ACCOUNT).ok_or_else(|| anyhow::anyhow!("终端不存在"))?;
            Ok(t.snapshot())
        })
    }

    fn term_close(&self, id: &str) -> BoxFut<'_, anyhow::Result<()>> {
        let id = id.to_string();
        Box::pin(async move {
            self.terms().kill(&id, LOCAL_TERM_ACCOUNT);
            Ok(())
        })
    }
}
