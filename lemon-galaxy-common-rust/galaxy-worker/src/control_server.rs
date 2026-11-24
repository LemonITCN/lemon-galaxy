//! 本机控制接口的服务端（docs/linux-desktop.md 第 7 节；消息见 control.rs）。
//!
//! Unix 套接字，一行一个 JSON，按对端凭据（SO_PEERCRED）鉴权。改配置一律经 host.json：先把命令行的改动读进来，
//! 改文件，再让服务重新读一遍，所以命令行与界面改的是同一份配置，不会互相覆盖。

use crate::control::*;
use crate::localsys::{self, PolkitResult};
use crate::{meta, store, WorkerOs, VERSION};
use galaxy_client::account::{self, FirstLogin, LoggedIn, LoginOutcome};
use galaxy_client::api::Api;
use galaxy_client::identity::Identity;
use galaxy_host::accounts::HostState;
use galaxy_host::service::{HostLogEvent, SessionInfo};
use galaxy_host::sysuser::SysUser;
use galaxy_host::{AccountStatus, HostConfig, HostEvents, HostPolicy, HostService, HostStatus};
use galaxy_net::ServerAddr;
use galaxy_proto::DeviceKind;
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

/// systemd 服务名（os.rs 安装的、打包装的都是这个）
const UNIT: &str = "lemon-galaxy-worker.service";

// ---------------- 鉴权 ----------------

/// 鉴权的输入（拆出来方便测试）。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RightsInput {
    /// 服务以 root 运行（系统服务）
    pub service_root: bool,
    pub service_uid: u32,
    pub peer_uid: u32,
    /// 对端是屏幕前图形会话的用户
    pub graphical: bool,
    /// 对端在 sudo / wheel / admin 组
    pub admin_group: bool,
    /// 对端是某个账号绑定的系统用户
    pub bound_user: bool,
    /// 系统上可能有 polkit
    pub polkit: bool,
}

/// 契约第 7 节的表。
pub(crate) fn rights_for(i: &RightsInput) -> Rights {
    if i.peer_uid == 0 || (!i.service_root && i.peer_uid == i.service_uid) {
        return Rights { operate: true, view_config: true, manage: ManageRight::Direct };
    }
    if !i.service_root {
        // 用户级服务的套接字是 0600，别人本来连不上；连上了也只给看状态
        return Rights { operate: false, view_config: false, manage: ManageRight::Denied };
    }
    Rights { operate: i.graphical, view_config: i.bound_user || i.admin_group, manage: if i.polkit { ManageRight::Polkit } else { ManageRight::Denied } }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Need {
    Operate,
    ViewConfig,
    Manage,
}

// ---------------- 连接 ----------------

struct Conn {
    id: u64,
    uid: u32,
    pid: Option<u32>,
    name: String,
    zh: AtomicBool,
    tx: mpsc::UnboundedSender<String>,
    login: tokio::sync::Mutex<Option<PendingLogin>>,
}

impl Conn {
    fn tr(&self, zh: &str, en: &str) -> String {
        if self.zh.load(Ordering::Relaxed) {
            zh.into()
        } else {
            en.into()
        }
    }
    fn lang(&self) -> &'static str {
        if self.zh.load(Ordering::Relaxed) {
            "zh-CN"
        } else {
            "en"
        }
    }
    fn send(&self, e: Event) -> bool {
        self.tx.send(encode(&EventLine { event: e })).is_ok()
    }
}

struct PendingLogin {
    api: Api,
    server: ServerAddr,
    stage: Stage,
}

enum Stage {
    FirstLogin(FirstLogin),
    Ready(Box<LoggedIn>),
}

// ---------------- 临时协助 ----------------

/// 协助请求交给开协助的那个连接回答。
struct AssistShared {
    owner: u64,
    owner_tx: mpsc::UnboundedSender<String>,
    pending: Mutex<HashMap<u64, oneshot::Sender<bool>>>,
    next: AtomicU64,
}

struct Assist {
    host: Arc<HostService>,
    shared: Arc<AssistShared>,
    tmp: PathBuf,
}

/// 等回答的协助请求被丢下（超时、对方断开）时，告诉界面关掉对话框。
struct PendingGuard {
    shared: Arc<AssistShared>,
    id: u64,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.shared.pending.lock().remove(&self.id);
        let _ = self.shared.owner_tx.send(encode(&EventLine { event: Event::AssistRequestClosed { request_id: self.id } }));
    }
}

// ---------------- 服务端 ----------------

pub(crate) struct Control {
    os: Arc<dyn WorkerOs>,
    dir: PathBuf,
    socket: PathBuf,
    root: bool,
    uid: u32,
    service_user: String,
    mode: ServiceMode,
    device_id: String,
    host: RwLock<Option<Arc<HostService>>>,
    assist: Mutex<Option<Assist>>,
    assist_gate: tokio::sync::Mutex<()>,
    sessions: Mutex<HashMap<String, Session>>,
    tick: watch::Sender<u64>,
    events: broadcast::Sender<Event>,
    wake: tokio::sync::Notify,
    edit: tokio::sync::Mutex<()>,
    next_conn: AtomicU64,
    closed: AtomicBool,
}

fn service_mode(root: bool) -> ServiceMode {
    let supervised = std::env::var("LEMON_GALAXY_SUPERVISED").as_deref() == Ok("1") || std::env::var_os("INVOCATION_ID").is_some();
    match (supervised, root) {
        (true, true) => ServiceMode::System,
        (true, false) => ServiceMode::User,
        _ => ServiceMode::Foreground,
    }
}

#[cfg(unix)]
fn chmod(p: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode));
}

/// 开始监听。另一个实例正在用这个套接字时返回 None（只记日志，不影响服务本身）。
pub(crate) async fn start(os: Arc<dyn WorkerOs>, dir: &Path) -> Option<Arc<Control>> {
    let root = galaxy_host::sysuser::is_root();
    // 环境变量 LEMON_GALAXY_CONTROL_SOCKET 可指定位置（测试、同一台机器跑多份时用；界面也认它）
    let socket = match std::env::var_os("LEMON_GALAXY_CONTROL_SOCKET").filter(|v| !v.is_empty()) {
        Some(p) => PathBuf::from(p),
        None if root => system_socket_path(),
        None => user_socket_path(dir),
    };
    start_at(os, dir, socket).await
}

pub(crate) async fn start_at(os: Arc<dyn WorkerOs>, dir: &Path, socket: PathBuf) -> Option<Arc<Control>> {
    let root = galaxy_host::sysuser::is_root();
    let parent = socket.parent()?.to_path_buf();
    let created = !parent.exists();
    if let Err(e) = std::fs::create_dir_all(&parent) {
        tracing::warn!("本机控制接口：无法创建 {}: {e}", parent.display());
        return None;
    }
    if parent.ends_with("lemon-galaxy-worker") && parent != dir {
        chmod(&parent, if root { 0o755 } else { 0o700 });
    } else if created {
        chmod(&parent, 0o700);
    }
    if socket.exists() {
        if UnixStream::connect(&socket).await.is_ok() {
            tracing::warn!("本机控制接口：{} 已被另一个实例占用，这个实例不提供控制接口", socket.display());
            return None;
        }
        let _ = std::fs::remove_file(&socket);
    }
    let listener = match UnixListener::bind(&socket) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("本机控制接口：无法监听 {}: {e}", socket.display());
            return None;
        }
    };
    chmod(&socket, if root { 0o666 } else { 0o600 });
    // 上次没清掉的临时协助目录
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with("assist-session-") {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    let id = Identity::load_or_create(&store(dir));
    let (tick, _) = watch::channel(0u64);
    let (events, _) = broadcast::channel(64);
    let me = SysUser::current();
    let ctl = Arc::new(Control {
        os,
        dir: dir.to_path_buf(),
        socket: socket.clone(),
        root,
        uid: me.uid,
        service_user: me.name,
        mode: service_mode(root),
        device_id: id.device_id.to_hex(),
        host: RwLock::new(None),
        assist: Mutex::new(None),
        assist_gate: tokio::sync::Mutex::new(()),
        sessions: Mutex::new(HashMap::new()),
        tick,
        events,
        wake: tokio::sync::Notify::new(),
        edit: tokio::sync::Mutex::new(()),
        next_conn: AtomicU64::new(1),
        closed: AtomicBool::new(false),
    });
    tracing::info!("本机控制接口：{}", socket.display());
    let weak = Arc::downgrade(&ctl);
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            };
            let Some(c) = weak.upgrade() else { break };
            if c.closed.load(Ordering::Relaxed) {
                break;
            }
            tokio::spawn(async move { c.serve(stream).await });
        }
    });
    Some(ctl)
}

impl Control {
    fn state_path(&self) -> PathBuf {
        self.dir.join("host.json")
    }

    /// 状态变了：各连接稍后推一次 Status
    pub(crate) fn bump(&self) {
        self.tick.send_modify(|v| *v += 1);
    }

    /// 被控服务换了（登录后启动、换服务器重启、停止）
    pub(crate) fn set_host(self: &Arc<Self>, h: Option<Arc<HostService>>) {
        if let Some(h) = &h {
            h.set_events(Arc::new(Bridge { ctl: Arc::downgrade(self), assist: None }));
        }
        *self.host.write() = h;
        self.bump();
    }

    /// 等界面登录、换服务器等需要被控服务马上处理的变化
    pub(crate) async fn woken(&self) {
        self.wake.notified().await
    }

    /// 服务退出：删套接字、结束临时协助
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_file(&self.socket);
        if let Some(a) = self.assist.lock().take() {
            a.host.shutdown();
            let _ = std::fs::remove_dir_all(&a.tmp);
        }
    }

    fn host(&self) -> Option<Arc<HostService>> {
        self.host.read().clone()
    }

    fn assist_host(&self) -> Option<Arc<HostService>> {
        self.assist.lock().as_ref().map(|a| a.host.clone())
    }

    fn current_state(&self) -> HostState {
        match self.host() {
            Some(h) => h.state(),
            None => HostState::load(&self.state_path()),
        }
    }

    fn rights(&self, c: &Conn) -> Rights {
        let input = if c.uid == 0 || c.uid == self.uid || !self.root {
            RightsInput { service_root: self.root, service_uid: self.uid, peer_uid: c.uid, ..Default::default() }
        } else {
            let st = self.current_state();
            RightsInput {
                service_root: self.root,
                service_uid: self.uid,
                peer_uid: c.uid,
                graphical: localsys::graphical_uids().contains(&c.uid),
                admin_group: localsys::in_admin_group(c.uid),
                bound_user: st.accounts.iter().any(|a| a.policy.system_user == c.name),
                polkit: localsys::system_bus_present(),
            }
        };
        rights_for(&input)
    }

    async fn check(&self, c: &Conn, need: Need) -> Result<(), Response> {
        let r = self.rights(c);
        let denied = |zh: &str, en: &str| Response::err(ErrorCode::Denied, c.tr(zh, en));
        match need {
            Need::Operate if r.operate => Ok(()),
            Need::Operate => Err(denied("只有 root 或当前坐在这台电脑前的用户能这样做。", "Only root or the user signed in at this computer can do this.")),
            Need::ViewConfig if r.view_config => Ok(()),
            Need::ViewConfig => Err(denied("只有管理员（sudo、wheel、admin 组）或账号绑定的系统用户能查看。", "Only administrators (sudo, wheel or admin group) or the bound system user can view this.")),
            Need::Manage => match r.manage {
                ManageRight::Direct => Ok(()),
                ManageRight::Denied if self.root => Err(Response::err(
                    ErrorCode::PolkitUnavailable,
                    c.tr(
                        "本机没有 polkit，只有 root 能修改配置。请用 sudo lemon-galaxy-worker 命令修改，或安装 polkit 后重试。",
                        "polkit is not available, so only root can change settings. Use sudo lemon-galaxy-worker, or install polkit and try again.",
                    ),
                )),
                ManageRight::Denied => Err(denied("这个服务属于另一个用户。", "This service belongs to another user.")),
                ManageRight::Polkit => {
                    let Some(pid) = c.pid else {
                        return Err(Response::err(ErrorCode::AuthFailed, c.tr("无法确认请求来自哪个进程。", "Cannot identify the requesting process.")));
                    };
                    match localsys::polkit_check(pid, c.uid, MANAGE_ACTION).await {
                        PolkitResult::Authorized => Ok(()),
                        PolkitResult::Denied { dismissed: true, .. } => Err(Response::err(ErrorCode::AuthFailed, c.tr("已取消授权。", "Authorization was cancelled."))),
                        PolkitResult::Denied { challenge: true, .. } => Err(Response::err(
                            ErrorCode::AuthFailed,
                            c.tr("需要管理员授权，但没有通过。如果没有弹出密码框，说明桌面没有运行 polkit 授权代理。", "Administrator authorization failed. If no password dialog appeared, no polkit agent is running in your session."),
                        )),
                        PolkitResult::Denied { .. } => Err(Response::err(ErrorCode::AuthFailed, c.tr("没有通过管理员授权。", "Not authorized."))),
                        PolkitResult::Unavailable(e) => {
                            tracing::warn!("polkit 不可用：{e}");
                            Err(Response::err(
                                ErrorCode::PolkitUnavailable,
                                format!(
                                    "{} ({e})",
                                    c.tr(
                                        "无法向 polkit 请求授权，只有 root 能修改配置。请用 sudo lemon-galaxy-worker 命令修改，或安装 polkit 并确认 /usr/share/polkit-1/actions/cn.lemonit.galaxy.worker.policy 存在。",
                                        "Cannot ask polkit for authorization, so only root can change settings. Use sudo lemon-galaxy-worker, or install polkit and make sure /usr/share/polkit-1/actions/cn.lemonit.galaxy.worker.policy exists."
                                    )
                                ),
                            ))
                        }
                    }
                }
            },
        }
    }

    // ---------------- 连接处理 ----------------

    async fn serve(self: Arc<Self>, stream: UnixStream) {
        let Ok(cred) = stream.peer_cred() else { return };
        let uid = cred.uid();
        let pid = cred.pid().and_then(|p| u32::try_from(p).ok());
        let (rd, mut wr) = stream.into_split();
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let conn = Arc::new(Conn {
            id: self.next_conn.fetch_add(1, Ordering::Relaxed),
            uid,
            pid,
            name: localsys::user_name(uid),
            zh: AtomicBool::new(crate::zh()),
            tx,
            login: tokio::sync::Mutex::new(None),
        });
        tracing::debug!("本机控制接口：{}（uid {uid}）已连接", conn.name);
        let writer = tokio::spawn(async move {
            while let Some(l) = rx.recv().await {
                if wr.write_all(l.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
        // 状态变了推 Status（合并 150 ms 内的变化）
        let pusher = {
            let (me, c) = (self.clone(), conn.clone());
            let mut tick = self.tick.subscribe();
            tokio::spawn(async move {
                while tick.changed().await.is_ok() {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    tick.borrow_and_update();
                    if !c.send(Event::Status { status: me.status_for(&c) }) {
                        break;
                    }
                }
            })
        };
        let forwarder = {
            let c = conn.clone();
            let mut sub = self.events.subscribe();
            tokio::spawn(async move {
                loop {
                    match sub.recv().await {
                        Ok(e) => {
                            if !c.send(e) {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(_) => break,
                    }
                }
            })
        };
        let mut reader = BufReader::new(rd);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match (&mut reader).take(MAX_LINE as u64 + 1).read_until(b'\n', &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            if buf.len() > MAX_LINE {
                break;
            }
            let line = String::from_utf8_lossy(&buf);
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<RequestLine>(line) {
                Ok(r) => {
                    let (me, c) = (self.clone(), conn.clone());
                    tokio::spawn(async move {
                        let resp = me.handle(&c, r.req).await;
                        let _ = c.tx.send(encode(&ResponseLine { id: r.id, resp }));
                    });
                }
                Err(e) => {
                    let id = serde_json::from_str::<serde_json::Value>(line).ok().and_then(|v| v.get("id").and_then(|i| i.as_u64())).unwrap_or(0);
                    let _ = conn.tx.send(encode(&ResponseLine { id, resp: Response::err(ErrorCode::BadRequest, e.to_string()) }));
                }
            }
        }
        pusher.abort();
        forwarder.abort();
        // 开协助的窗口关了，协助随之结束
        let owned = self.assist.lock().as_ref().is_some_and(|a| a.shared.owner == conn.id);
        if owned {
            self.stop_assist("owner_closed");
        }
        drop(conn);
        // 还在跑的请求、临时协助的回调可能还拿着发送端：最多等 2 秒把剩下的写完
        let abort = writer.abort_handle();
        if tokio::time::timeout(Duration::from_secs(2), writer).await.is_err() {
            abort.abort();
        }
    }

    async fn handle(self: &Arc<Self>, c: &Arc<Conn>, req: Request) -> Response {
        match self.dispatch(c, req).await {
            Ok(r) => r,
            Err(r) => r,
        }
    }

    async fn dispatch(self: &Arc<Self>, c: &Arc<Conn>, req: Request) -> Result<Response, Response> {
        Ok(match req {
            Request::Hello { lang } => {
                c.zh.store(lang.to_ascii_lowercase().starts_with("zh"), Ordering::Relaxed);
                Response::Hello { protocol: PROTOCOL_VERSION, version: VERSION.into() }
            }
            Request::Status => Response::Status { status: self.status_for(c) },
            Request::Accounts => {
                self.check(c, Need::ViewConfig).await?;
                Response::Accounts { accounts: self.current_state().accounts.iter().map(account_view).collect() }
            }
            Request::SystemUsers => {
                self.check(c, Need::ViewConfig).await?;
                let (users, default_user) = self.system_users(c);
                Response::SystemUsers { users, can_choose: self.root, default_user, superuser: self.os.superuser_name().into() }
            }
            Request::Login { server, username, password, totp, replace_server } => {
                self.check(c, Need::Manage).await?;
                self.login(c, &server, &username, &password, totp, replace_server).await?
            }
            Request::LoginSetPassword { new_password } => {
                self.check(c, Need::Manage).await?;
                self.login_set_password(c, &new_password).await?
            }
            Request::LoginFinish { system_user, confirm_root } => {
                self.check(c, Need::Manage).await?;
                self.login_finish(c, &system_user, confirm_root).await?
            }
            Request::LoginCancel => {
                *c.login.lock().await = None;
                Response::ok()
            }
            Request::Logout { user_id } => {
                self.check(c, Need::Manage).await?;
                self.logout(c, user_id).await?
            }
            Request::SetPolicy { user_id, policy, confirm_root } => {
                self.check(c, Need::Manage).await?;
                let sys = self.check_system_user(c, &policy.system_user)?;
                let st = self.current_state();
                let Some(old) = st.accounts.iter().find(|a| a.user_id == user_id) else {
                    return Err(Response::err(ErrorCode::NotFound, c.tr("本机没有登录这个账号。", "This account is not signed in on this computer.")));
                };
                if sys.eq_ignore_ascii_case(self.os.superuser_name()) && !old.policy.system_user.eq_ignore_ascii_case(self.os.superuser_name()) && !confirm_root {
                    return Err(self.root_confirm(c));
                }
                let mut p = policy_from(&policy);
                p.system_user = sys;
                self.edit(c, |st| match st.accounts.iter_mut().find(|a| a.user_id == user_id) {
                    Some(a) => {
                        a.policy = p;
                        Ok(())
                    }
                    None => Err(Response::err(ErrorCode::NotFound, c.tr("本机没有登录这个账号。", "This account is not signed in on this computer."))),
                })
                .await?;
                Response::ok()
            }
            Request::SetRemoteEnabled { enabled } => {
                // 禁止远程是保护动作，屏幕前的人不用管理员密码；重新允许要管理员
                if enabled || !self.rights(c).operate {
                    self.check(c, Need::Manage).await?;
                }
                self.edit(c, |st| {
                    st.remote_enabled = enabled;
                    Ok(())
                })
                .await?;
                Response::ok()
            }
            Request::Disconnect { session_id } => {
                self.check(c, Need::Operate).await?;
                let found = self.host().is_some_and(|h| h.disconnect_session(&session_id)) || self.assist_host().is_some_and(|h| h.disconnect_session(&session_id));
                if !found {
                    return Err(Response::err(ErrorCode::NotFound, c.tr("这个连接已经结束了。", "This session has already ended.")));
                }
                Response::ok()
            }
            Request::DisconnectAll => {
                self.check(c, Need::Operate).await?;
                for h in [self.host(), self.assist_host()].into_iter().flatten() {
                    h.disconnect_all();
                }
                Response::ok()
            }
            Request::StopAi => {
                self.check(c, Need::Operate).await?;
                for h in [self.host(), self.assist_host()].into_iter().flatten() {
                    h.stop_ai();
                }
                Response::ok()
            }
            Request::AssistStart { server } => {
                self.check(c, Need::Operate).await?;
                self.assist_start(c, server).await?
            }
            Request::AssistStop => {
                self.check(c, Need::Operate).await?;
                if self.assist.lock().is_none() {
                    return Ok(Response::ok());
                }
                self.stop_assist("stopped");
                Response::ok()
            }
            Request::AssistAnswer { request_id, allow } => {
                let sender = {
                    let a = self.assist.lock();
                    match a.as_ref() {
                        Some(a) if a.shared.owner == c.id => a.shared.pending.lock().remove(&request_id),
                        Some(_) => return Err(Response::err(ErrorCode::Denied, c.tr("这个协助不是从这个窗口开启的。", "This assistance was started from another window."))),
                        None => None,
                    }
                };
                match sender {
                    Some(s) => {
                        let _ = s.send(allow);
                        Response::ok()
                    }
                    None => return Err(Response::err(ErrorCode::NotFound, c.tr("这个请求已经过期。", "This request has expired."))),
                }
            }
            Request::Settings => {
                self.check(c, Need::ViewConfig).await?;
                Response::Settings { settings: self.settings(c).await }
            }
            Request::SetAssistCode { code } => {
                self.check(c, Need::Manage).await?;
                let code = match code.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
                    None => None,
                    Some(s) if (8..=16).contains(&s.len()) && s.chars().all(|ch| ch.is_ascii_alphanumeric()) => Some(s),
                    Some(_) => return Err(Response::err(ErrorCode::Invalid, c.tr("验证码要 8 到 16 位字母或数字。", "The code must be 8 to 16 letters or digits."))),
                };
                let code2 = code.clone();
                self.edit(c, move |st| {
                    st.assist_code = code2;
                    Ok(())
                })
                .await?;
                if let Some(h) = self.assist_host() {
                    h.set_assist_code(code);
                    self.bump();
                }
                Response::ok()
            }
            Request::SetAutoUpdate { enabled } => {
                self.check(c, Need::Manage).await?;
                self.edit(c, |st| {
                    st.auto_update = enabled;
                    Ok(())
                })
                .await?;
                Response::ok()
            }
            Request::SetLockOnEnd { enabled } => {
                self.check(c, Need::Manage).await?;
                self.edit(c, |st| {
                    st.lock_on_end = enabled;
                    Ok(())
                })
                .await?;
                Response::ok()
            }
            Request::SetHeadlessDisplay { value } => {
                self.check(c, Need::Manage).await?;
                let v = value.trim().to_string();
                let off = matches!(v.to_ascii_lowercase().as_str(), "off" | "no" | "false" | "0");
                if !off && galaxy_host::accounts::parse_headless_display(&v).is_none() {
                    return Err(Response::err(ErrorCode::Invalid, c.tr("分辨率写法不对，例如 1920x1080 或 2560x1440@60。", "Invalid resolution, e.g. 1920x1080 or 2560x1440@60.")));
                }
                self.edit(c, |st| {
                    st.headless_display = if off { "off".into() } else { v };
                    Ok(())
                })
                .await?;
                Response::ok()
            }
            Request::SetAutostart { enabled } => {
                self.check(c, Need::Manage).await?;
                self.set_autostart(c, enabled).await?
            }
            Request::SetLoginScreenX11 { enabled } => {
                self.check(c, Need::Manage).await?;
                if !self.root {
                    return Err(Response::err(ErrorCode::NotApplicable, c.tr("要以系统服务运行才能修改登录界面。", "The service must run as a system service to change the login screen.")));
                }
                crate::gdm::set(enabled).map_err(|e| {
                    let code = match e {
                        crate::gdm::GdmError::Io(_) => ErrorCode::Failed,
                        _ => ErrorCode::NotApplicable,
                    };
                    Response::err(code, e.message(c.zh.load(Ordering::Relaxed)))
                })?;
                Response::Ok { message: Some(c.tr("重启电脑或注销后生效。", "Takes effect after you restart or sign out.")) }
            }
            Request::Events { limit } => {
                self.check(c, Need::ViewConfig).await?;
                Response::Events { events: read_events(&self.dir, limit.clamp(1, 2000) as usize) }
            }
            Request::CheckUpdate => {
                self.check(c, Need::ViewConfig).await?;
                let latest = crate::check_update(self.os.as_ref(), &self.dir).await.map_err(|e| Response::err(ErrorCode::Failed, e))?;
                Response::UpdateCheck { current: VERSION.into(), latest: latest.map(|i| UpdateAvailable { version: i.version, size: i.size, notes: i.notes }) }
            }
            Request::ApplyUpdate => {
                self.check(c, Need::Manage).await?;
                let v = crate::apply_update(self.os.as_ref(), &self.dir).await.map_err(|e| Response::err(ErrorCode::Failed, e))?;
                let restarting = v.is_some() && self.mode != ServiceMode::Foreground;
                if restarting {
                    // 回复发出去后退出，由服务管理器拉起新版本（与自动更新相同）
                    let socket = self.socket.clone();
                    let dir = self.dir.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(800)).await;
                        let _ = std::fs::remove_file(socket);
                        let _ = std::fs::remove_file(dir.join("status.json"));
                        tracing::info!("已更新，重启服务");
                        std::process::exit(75);
                    });
                }
                Response::Updated { version: v, restarting }
            }
        })
    }

    // ---------------- 状态 ----------------

    fn status_for(&self, c: &Conn) -> Status {
        let st = self.current_state();
        let hs: Option<HostStatus> = self.host().map(|h| h.status());
        let mut sessions: Vec<Session> = hs.as_ref().map(|s| s.sessions.iter().map(|x| session_view(x, false, &st)).collect()).unwrap_or_default();
        let assist = {
            let a = self.assist.lock();
            a.as_ref().map(|a| (a.host.status(), a.shared.owner == c.id))
        };
        let assist = assist.map(|(s, mine)| {
            sessions.extend(s.sessions.iter().map(|x| session_view(x, true, &st)));
            let (dc, code) = s.assist.unwrap_or_default();
            if mine || c.uid == 0 {
                AssistInfo { mine, device_code: dc, code }
            } else {
                AssistInfo { mine, device_code: String::new(), code: String::new() }
            }
        });
        sessions.sort_by_key(|s| s.started_at);
        Status {
            version: VERSION.into(),
            device_id: self.device_id.clone(),
            hostname: galaxy_host::hostname(),
            data_dir: self.dir.display().to_string(),
            service: ServiceInfo { mode: self.mode, user: self.service_user.clone(), pid: std::process::id() },
            account_count: st.accounts.len() as u32,
            needs_relogin: st.accounts.iter().filter(|a| a.status == AccountStatus::NeedsRelogin).count() as u32,
            online: hs.as_ref().is_some_and(|s| s.online),
            server: st.server.clone(),
            public_ip: hs.as_ref().filter(|s| s.online).map(|s| s.public_ip.clone()).unwrap_or_default(),
            remote_enabled: st.remote_enabled,
            sessions,
            assist,
            rights: self.rights(c),
            you: c.name.clone(),
            config_rev: std::fs::metadata(self.state_path()).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0),
        }
    }

    fn system_users(&self, c: &Conn) -> (Vec<String>, String) {
        if !self.root {
            return (vec![self.service_user.clone()], self.service_user.clone());
        }
        let users = localsys::login_users();
        let default = if c.uid != 0 { c.name.clone() } else { self.os.default_system_user() };
        (users, default)
    }

    /// 检查系统用户，返回规范写法。
    fn check_system_user(&self, c: &Conn, name: &str) -> Result<String, Response> {
        let name = name.trim();
        let Some(u) = SysUser::lookup(name) else {
            return Err(Response::err(ErrorCode::Invalid, format!("{}{name}", c.tr("系统用户不存在：", "No such system user: "))));
        };
        let n = u.display_name();
        if !self.root && n != self.service_user {
            return Err(Response::err(
                ErrorCode::Invalid,
                c.tr("服务以普通用户运行，只能用这个用户。要选其他用户，请安装为系统服务。", "The service runs as a regular user and can only use that user. Install the system service to choose another user."),
            ));
        }
        Ok(n)
    }

    fn root_confirm(&self, c: &Conn) -> Response {
        Response::err(
            ErrorCode::ConfirmRoot,
            c.tr(
                "以 root 身份开放远程终端和文件风险很高：对方能完全控制这台电脑。确定要这样做吗？",
                "Running remote terminal and files as root is risky: the other side gets full control of this computer. Continue?",
            ),
        )
    }

    async fn settings(&self, c: &Conn) -> Settings {
        let st = HostState::load(&self.state_path());
        let login_screen_x11 = if !self.root {
            Feature { available: false, enabled: false, detail: c.tr("要以系统服务运行才能修改。", "Requires the system service.") }
        } else {
            match crate::gdm::state() {
                crate::gdm::GdmState::NotGdm => Feature { available: false, enabled: false, detail: c.tr("登录界面不是 GDM。", "The login screen is not GDM.") },
                // 没有 X11 会话的系统（GNOME 49 起）不能打开；已经打开的仍可以关掉
                crate::gdm::GdmState::Gdm { x11, x11_available, .. } => Feature {
                    available: x11_available || x11,
                    enabled: x11,
                    detail: if x11_available { String::new() } else { crate::gdm::GdmError::NoX11Session.message(c.zh.load(Ordering::Relaxed)) },
                },
            }
        };
        Settings {
            auto_update: st.auto_update,
            lock_on_end: st.lock_on_end,
            headless_display: galaxy_host::accounts::parse_headless_display(&st.headless_display).map(|(width, height, refresh)| DisplayMode { width, height, refresh }),
            assist_code: st.assist_code.clone(),
            autostart: self.autostart(c).await,
            login_screen_x11,
        }
    }

    fn systemctl(&self) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new("systemctl");
        if !self.root {
            cmd.arg("--user");
        }
        cmd
    }

    async fn autostart(&self, c: &Conn) -> Feature {
        if !Path::new("/run/systemd/system").exists() {
            return Feature { available: false, enabled: false, detail: c.tr("这台电脑没有使用 systemd。", "This computer does not use systemd.") };
        }
        let out = self.systemctl().args(["is-enabled", UNIT]).output().await;
        let s = out.as_ref().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        match s.as_str() {
            "enabled" | "enabled-runtime" | "alias" | "linked" | "linked-runtime" => Feature { available: true, enabled: true, detail: String::new() },
            "disabled" | "indirect" => Feature { available: true, enabled: false, detail: String::new() },
            "static" | "generated" | "transient" => Feature { available: false, enabled: true, detail: s },
            _ => Feature { available: false, enabled: false, detail: c.tr("还没有安装为服务。", "Not installed as a service.") },
        }
    }

    async fn set_autostart(&self, c: &Conn, on: bool) -> Result<Response, Response> {
        let f = self.autostart(c).await;
        if !f.available {
            return Err(Response::err(ErrorCode::NotApplicable, if f.detail.is_empty() { c.tr("不适用。", "Not applicable.") } else { f.detail }));
        }
        let out = self.systemctl().args([if on { "enable" } else { "disable" }, UNIT]).output().await.map_err(|e| Response::err(ErrorCode::Failed, e.to_string()))?;
        if !out.status.success() {
            return Err(Response::err(ErrorCode::Failed, String::from_utf8_lossy(&out.stderr).trim().to_string()));
        }
        self.bump();
        Ok(Response::ok())
    }

    /// 改 host.json：先读进命令行的改动，改完让被控服务重新读。
    async fn edit(&self, c: &Conn, f: impl FnOnce(&mut HostState) -> Result<(), Response>) -> Result<(), Response> {
        let _g = self.edit.lock().await;
        let host = self.host();
        if let Some(h) = &host {
            h.reload_from_disk();
        }
        let path = self.state_path();
        let mut st = HostState::load(&path);
        f(&mut st)?;
        st.save(&path).map_err(|e| Response::err(ErrorCode::Failed, format!("{}: {e}", c.tr("保存失败", "Save failed"))))?;
        if let Some(h) = &host {
            h.reload_from_disk();
        }
        self.wake.notify_one();
        self.bump();
        Ok(())
    }

    // ---------------- 登录 ----------------

    async fn login(&self, c: &Conn, server: &str, username: &str, password: &str, totp: Option<String>, replace: bool) -> Result<Response, Response> {
        let server = ServerAddr::parse(server.trim()).map_err(|e| bad_server(c, &e))?;
        if username.trim().is_empty() || password.is_empty() {
            return Err(Response::err(ErrorCode::Invalid, c.tr("请填写用户名和密码。", "Enter your username and password.")));
        }
        let st = HostState::load(&self.state_path());
        if !st.server.is_empty() && st.server != server.to_string_full() && !st.accounts.is_empty() && !replace {
            return Ok(Response::Login { step: LoginStep::ConfirmServerChange { current_server: st.server.clone(), accounts: st.accounts.len() as u32 } });
        }
        let api = Api::new(server.clone(), c.lang()).map_err(|e| Response::err(ErrorCode::Failed, e.to_string()))?;
        let id = Identity::load_or_create(&store(&self.dir));
        let m = meta(self.os.as_ref(), DeviceKind::Worker);
        let totp = totp.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let out = account::login(&api, &id, &m, username.trim(), password, totp.as_deref()).await;
        let mut pending = c.login.lock().await;
        *pending = None;
        match out {
            Ok(LoginOutcome::NeedTotp) if totp.is_some() => Err(Response::err(ErrorCode::InvalidTotp, c.tr("两步验证码不正确。", "Incorrect two-factor code."))),
            Ok(LoginOutcome::NeedTotp) => Ok(Response::Login { step: LoginStep::NeedTotp }),
            Ok(LoginOutcome::MustChangePassword(fl)) => {
                let step = LoginStep::NeedNewPassword { username: fl.user.username.clone(), display_name: fl.user.display_name.clone() };
                *pending = Some(PendingLogin { api, server, stage: Stage::FirstLogin(fl) });
                Ok(Response::Login { step })
            }
            Ok(LoginOutcome::Done(l)) => {
                let step = self.choose_user_step(c, &l);
                *pending = Some(PendingLogin { api, server, stage: Stage::Ready(Box::new(l)) });
                Ok(Response::Login { step })
            }
            Err(e) => Err(login_error(c, e)),
        }
    }

    fn choose_user_step(&self, c: &Conn, l: &LoggedIn) -> LoginStep {
        let (users, default_user) = self.system_users(c);
        LoginStep::ChooseSystemUser {
            username: l.user.username.clone(),
            display_name: l.user.display_name.clone(),
            default_user,
            users,
            can_choose: self.root,
            superuser: self.os.superuser_name().into(),
        }
    }

    async fn login_set_password(&self, c: &Conn, new_password: &str) -> Result<Response, Response> {
        if new_password.chars().count() < 8 {
            return Err(Response::err(ErrorCode::Invalid, c.tr("新密码至少 8 位。", "The new password must be at least 8 characters.")));
        }
        let mut pending = c.login.lock().await;
        let Some(p) = pending.take() else {
            return Err(Response::err(ErrorCode::Invalid, c.tr("登录已失效，请重新登录。", "The sign-in has expired. Please start again.")));
        };
        let Stage::FirstLogin(fl) = p.stage else {
            *pending = Some(p);
            return Err(Response::err(ErrorCode::Invalid, c.tr("不需要设置新密码。", "No new password is needed.")));
        };
        let keep = FirstLogin { token: fl.token.clone(), user: fl.user.clone(), epoch: fl.epoch };
        let id = Identity::load_or_create(&store(&self.dir));
        let m = meta(self.os.as_ref(), DeviceKind::Worker);
        match account::complete_first_login(&p.api, &id, &m, fl, new_password).await {
            Ok(l) => {
                let step = self.choose_user_step(c, &l);
                *pending = Some(PendingLogin { api: p.api, server: p.server, stage: Stage::Ready(Box::new(l)) });
                Ok(Response::Login { step })
            }
            Err(e) => {
                *pending = Some(PendingLogin { api: p.api, server: p.server, stage: Stage::FirstLogin(keep) });
                Err(login_error(c, e))
            }
        }
    }

    async fn login_finish(&self, c: &Conn, system_user: &str, confirm_root: bool) -> Result<Response, Response> {
        let sys = self.check_system_user(c, system_user)?;
        if sys.eq_ignore_ascii_case(self.os.superuser_name()) && !confirm_root {
            return Err(self.root_confirm(c));
        }
        let mut pending = c.login.lock().await;
        let p = match pending.take() {
            Some(PendingLogin { stage: Stage::Ready(l), api, server }) => (l, api, server),
            other => {
                *pending = other;
                return Err(Response::err(ErrorCode::Invalid, c.tr("登录已失效，请重新登录。", "The sign-in has expired. Please start again.")));
            }
        };
        let (logged, api, server) = p;
        let full = server.to_string_full();
        let st = HostState::load(&self.state_path());
        let sign_key = if st.server != full || st.server_sign_pubkey.is_empty() {
            api.get::<galaxy_proto::api::ServerInfoResp>("/info").await.ok().map(|i| i.server_sign_pubkey)
        } else {
            None
        };
        let acct = galaxy_host::HostAccount {
            user_id: logged.user.id,
            username: logged.user.username.clone(),
            display_name: logged.user.display_name.clone(),
            account_pubkey: logged.account_pubkey,
            epoch: logged.epoch,
            cert: logged.cert.clone(),
            token: logged.token.clone(),
            status: AccountStatus::Active,
            policy: HostPolicy::default_for(&sys),
            revoked: vec![],
            added_at: galaxy_proto::now_ms(),
        };
        self.edit(c, move |st| {
            if st.server != full {
                st.accounts.clear();
                st.server_sign_pubkey.clear();
            }
            st.server = full;
            if let Some(k) = sign_key {
                st.server_sign_pubkey = k;
            }
            st.accounts.retain(|a| a.user_id != acct.user_id);
            st.accounts.push(acct);
            Ok(())
        })
        .await?;
        self.os.secure_data_dir(&self.dir);
        tracing::info!("界面登录了账号 {}（系统用户 {sys}）", logged.user.username);
        Ok(Response::Login { step: LoginStep::Done { username: logged.user.username.clone(), display_name: logged.user.display_name.clone(), system_user: sys } })
    }

    async fn logout(&self, c: &Conn, user_id: u64) -> Result<Response, Response> {
        let not_found = || Response::err(ErrorCode::NotFound, c.tr("本机没有登录这个账号。", "This account is not signed in on this computer."));
        if let Some(h) = self.host() {
            let _g = self.edit.lock().await;
            h.reload_from_disk();
            if !h.state().accounts.iter().any(|a| a.user_id == user_id) {
                return Err(not_found());
            }
            h.remove_account(user_id).await;
            self.bump();
            return Ok(Response::ok());
        }
        let st = HostState::load(&self.state_path());
        let Some(a) = st.accounts.iter().find(|a| a.user_id == user_id).cloned() else { return Err(not_found()) };
        if let Ok(server) = ServerAddr::parse(&st.server) {
            if let Ok(api) = Api::new(server, c.lang()) {
                api.set_token(Some(a.token.clone()));
                let _ = tokio::time::timeout(Duration::from_secs(10), api.post::<_, serde_json::Value>("/auth/logout", &serde_json::json!({}))).await;
            }
        }
        self.edit(c, |st| {
            st.accounts.retain(|x| x.user_id != user_id);
            Ok(())
        })
        .await?;
        Ok(Response::ok())
    }

    // ---------------- 临时协助 ----------------

    async fn assist_start(self: &Arc<Self>, c: &Arc<Conn>, server: Option<String>) -> Result<Response, Response> {
        let _gate = self.assist_gate.lock().await;
        {
            let a = self.assist.lock();
            if let Some(a) = a.as_ref() {
                if a.shared.owner != c.id {
                    return Err(Response::err(ErrorCode::Busy, c.tr("临时协助已经在另一个窗口里开启了。", "Assistance is already running in another window.")));
                }
            }
        }
        if self.assist.lock().is_some() {
            return Ok(Response::Assist { assist: self.assist_info(c) });
        }
        let st = HostState::load(&self.state_path());
        let server_s = server.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or(st.server.clone());
        if server_s.is_empty() {
            return Err(Response::err(ErrorCode::Invalid, c.tr("请填写服务器地址。", "Enter the server address.")));
        }
        let server = ServerAddr::parse(&server_s).map_err(|e| bad_server(c, &e))?;
        let tmp = self.dir.join(format!("assist-session-{}", c.id));
        let _ = std::fs::remove_dir_all(&tmp);
        let id = Identity::load_or_create(&store(&tmp));
        let helper = if self.root { std::env::current_exe().ok() } else { None };
        let cfg = HostConfig { server, data_dir: tmp.clone(), meta: meta(self.os.as_ref(), DeviceKind::Worker), multi_account: false, file_helper: helper };
        let host = HostService::start(cfg, id).await.map_err(|e| Response::err(ErrorCode::Failed, e.to_string()))?;
        let shared = Arc::new(AssistShared { owner: c.id, owner_tx: c.tx.clone(), pending: Mutex::new(HashMap::new()), next: AtomicU64::new(1) });
        host.set_events(Arc::new(Bridge { ctl: Arc::downgrade(self), assist: Some(shared.clone()) }));
        if let Some(p) = crate::platform_for(self.os.as_ref()) {
            host.set_platform(p);
        }
        if st.assist_code.is_some() {
            host.set_assist_code(st.assist_code.clone());
        }
        host.start_assist();
        let mut ok = false;
        for _ in 0..50 {
            if host.status().assist.is_some() {
                ok = true;
                break;
            }
            if c.tx.is_closed() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        if !ok || c.tx.is_closed() {
            host.shutdown();
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(Response::err(ErrorCode::Failed, c.tr("连不上服务器，无法开启临时协助。", "Cannot reach the server, so assistance could not start.")));
        }
        *self.assist.lock() = Some(Assist { host, shared, tmp });
        tracing::info!("临时协助已开启（{}）", c.name);
        self.bump();
        Ok(Response::Assist { assist: self.assist_info(c) })
    }

    fn assist_info(&self, c: &Conn) -> AssistInfo {
        let a = self.assist.lock();
        match a.as_ref() {
            Some(a) => {
                let (device_code, code) = a.host.status().assist.unwrap_or_default();
                AssistInfo { mine: a.shared.owner == c.id, device_code, code }
            }
            None => AssistInfo { mine: false, device_code: String::new(), code: String::new() },
        }
    }

    fn stop_assist(&self, reason: &str) {
        let Some(a) = self.assist.lock().take() else { return };
        a.host.disconnect_all();
        a.host.shutdown();
        // 没回答的请求一律拒绝
        a.shared.pending.lock().clear();
        let _ = std::fs::remove_dir_all(&a.tmp);
        let _ = a.shared.owner_tx.send(encode(&EventLine { event: Event::AssistEnded { reason: reason.into() } }));
        tracing::info!("临时协助已结束（{reason}）");
        self.bump();
    }

    // ---------------- 被控服务的回调 ----------------

    fn session_started(&self, s: &SessionInfo, assist: bool) {
        let v = session_view(s, assist, &self.current_state());
        self.sessions.lock().insert(v.id.clone(), v.clone());
        let _ = self.events.send(Event::SessionStarted { session: v });
        self.bump();
    }

    fn session_ended(&self, id: &str) {
        if let Some(v) = self.sessions.lock().remove(id) {
            let _ = self.events.send(Event::SessionEnded { session: v });
        }
        self.bump();
    }

    fn ai_activity(&self, session: &str, desc: &str) {
        let from_user = self.sessions.lock().get(session).map(|s| s.from_user.clone()).unwrap_or_default();
        let _ = self.events.send(Event::AiActivity { session_id: session.into(), from_user, description: desc.chars().take(300).collect() });
    }
}

/// 被控服务（主服务或临时协助）的事件接到控制接口。每个 HostService 只能设一个 HostEvents，这里是唯一的一个。
struct Bridge {
    ctl: Weak<Control>,
    assist: Option<Arc<AssistShared>>,
}

impl HostEvents for Bridge {
    fn on_status(&self, _s: &HostStatus) {
        if let Some(c) = self.ctl.upgrade() {
            c.bump();
        }
    }

    fn ask_assist(&self, from_user: String, from_device: String) -> futures::future::BoxFuture<'static, bool> {
        let Some(a) = self.assist.clone() else { return Box::pin(async { false }) };
        let request_id = a.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        a.pending.lock().insert(request_id, tx);
        if a.owner_tx.send(encode(&EventLine { event: Event::AssistRequest { request_id, from_user, from_device } })).is_err() {
            a.pending.lock().remove(&request_id);
            return Box::pin(async { false });
        }
        let guard = PendingGuard { shared: a, id: request_id };
        Box::pin(async move {
            let r = rx.await.unwrap_or(false);
            drop(guard);
            r
        })
    }

    fn on_session_started(&self, s: &SessionInfo) {
        if let Some(c) = self.ctl.upgrade() {
            c.session_started(s, self.assist.is_some());
        }
    }

    fn on_session_ended(&self, id: &str) {
        if let Some(c) = self.ctl.upgrade() {
            c.session_ended(id);
        }
    }

    fn on_ai_activity(&self, session: &str, desc: &str) {
        if let Some(c) = self.ctl.upgrade() {
            c.ai_activity(session, desc);
        }
    }

    /// 临时协助的服务把日志写在它自己的临时目录里（结束就删）：再抄一份到主日志，「事件日志」里才看得到
    fn on_event(&self, e: &HostLogEvent) {
        if self.assist.is_none() {
            return;
        }
        let Some(c) = self.ctl.upgrade() else { return };
        if let Ok(line) = serde_json::to_string(e) {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(c.dir.join("events.log")) {
                let _ = writeln!(f, "{line}");
            }
        }
    }
}

// ---------------- 转换 ----------------

fn session_view(s: &SessionInfo, assist: bool, st: &HostState) -> Session {
    let notify = s.account.and_then(|id| st.accounts.iter().find(|a| a.user_id == id)).map(|a| a.policy.notify).unwrap_or(true);
    Session { id: s.id.clone(), from_user: s.from_user.clone(), from_device: s.from_device.clone(), mode: s.mode, path: s.path, started_at: s.started_at, ai_active: s.ai_active, assist, notify }
}

fn account_view(a: &galaxy_host::HostAccount) -> Account {
    Account { user_id: a.user_id, username: a.username.clone(), display_name: a.display_name.clone(), active: a.status == AccountStatus::Active, added_at: a.added_at, policy: policy_view(&a.policy) }
}

pub(crate) fn policy_view(p: &HostPolicy) -> Policy {
    Policy { system_user: p.system_user.clone(), view: p.view, control: p.control, files: p.files, terminal: p.terminal, clipboard: p.clipboard, audio: p.audio, ai_cap: p.ai_cap, notify: p.notify }
}

pub(crate) fn policy_from(p: &Policy) -> HostPolicy {
    HostPolicy { system_user: p.system_user.clone(), view: p.view, control: p.control, files: p.files, terminal: p.terminal, clipboard: p.clipboard, audio: p.audio, ai_cap: p.ai_cap, notify: p.notify }
}

fn bad_server(c: &Conn, e: &str) -> Response {
    let m = if c.zh.load(Ordering::Relaxed) { format!("服务器地址不对：{e}。格式为 主机:端口#sha256-指纹") } else { "Invalid server address. Use host:port#sha256-fingerprint.".to_string() };
    Response::err(ErrorCode::Invalid, m)
}

fn login_error(c: &Conn, e: galaxy_client::ClientError) -> Response {
    use galaxy_client::ClientError;
    match &e {
        ClientError::Auth { code, .. } if code == "invalid_credentials" => Response::err(ErrorCode::InvalidCredentials, c.tr("用户名或密码不正确。", "Incorrect username or password.")),
        ClientError::Auth { code, .. } if code.contains("totp") => Response::err(ErrorCode::InvalidTotp, c.tr("两步验证码不正确。", "Incorrect two-factor code.")),
        ClientError::Auth { message, .. } => Response::err(ErrorCode::Failed, message.clone()),
        ClientError::Network(m) => Response::err(ErrorCode::Failed, format!("{}: {m}", c.tr("连不上服务器", "Cannot reach the server"))),
        _ => Response::err(ErrorCode::Failed, e.to_string()),
    }
}

fn read_events(dir: &Path, n: usize) -> Vec<LogEntry> {
    let Ok(s) = std::fs::read_to_string(dir.join("events.log")) else { return vec![] };
    let mut v: Vec<LogEntry> = s
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<HostLogEvent>(l).ok())
        .take(n)
        .map(|e| LogEntry { at: e.at, session: e.session, user: e.user, kind: e.kind, detail: e.detail, ai: e.ai })
        .collect();
    v.reverse();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(peer: u32) -> RightsInput {
        RightsInput { service_root: true, service_uid: 0, peer_uid: peer, polkit: true, ..Default::default() }
    }

    #[test]
    fn root_service_rights() {
        // root 什么都能做
        assert_eq!(rights_for(&input(0)), Rights { operate: true, view_config: true, manage: ManageRight::Direct });
        // 普通用户：只能看状态；改配置要 polkit
        assert_eq!(rights_for(&input(1000)), Rights { operate: false, view_config: false, manage: ManageRight::Polkit });
        // 屏幕前的用户能断开、停 AI、开协助
        let g = RightsInput { graphical: true, ..input(1000) };
        assert_eq!(rights_for(&g), Rights { operate: true, view_config: false, manage: ManageRight::Polkit });
        // 管理员组、绑定的系统用户能看配置
        assert!(rights_for(&RightsInput { admin_group: true, ..input(1000) }).view_config);
        assert!(rights_for(&RightsInput { bound_user: true, ..input(1000) }).view_config);
        // 没有 polkit：只有 root 能改
        assert_eq!(rights_for(&RightsInput { polkit: false, graphical: true, admin_group: true, ..input(1000) }).manage, ManageRight::Denied);
    }

    #[test]
    fn user_service_rights() {
        let mine = RightsInput { service_root: false, service_uid: 1000, peer_uid: 1000, ..Default::default() };
        assert_eq!(rights_for(&mine), Rights { operate: true, view_config: true, manage: ManageRight::Direct });
        let other = RightsInput { peer_uid: 1001, graphical: true, admin_group: true, polkit: true, ..mine };
        assert_eq!(rights_for(&other), Rights { operate: false, view_config: false, manage: ManageRight::Denied });
        let root = RightsInput { peer_uid: 0, ..mine };
        assert_eq!(rights_for(&root).manage, ManageRight::Direct);
    }

    #[test]
    fn policy_round_trip() {
        let mut p = HostPolicy::default_for("alice");
        p.terminal = false;
        p.ai_cap = galaxy_proto::AiMode::ConfirmDangerous;
        p.notify = false;
        assert_eq!(policy_from(&policy_view(&p)), p);
    }

    struct TestOs;
    impl WorkerOs for TestOs {
        fn platform(&self) -> galaxy_proto::Platform {
            galaxy_proto::Platform::Linux
        }
        fn os_version(&self) -> String {
            "test".into()
        }
        fn default_data_dir(&self, _admin: bool) -> PathBuf {
            std::env::temp_dir()
        }
        fn default_system_user(&self) -> String {
            SysUser::current().name
        }
        fn superuser_name(&self) -> &'static str {
            "root"
        }
        fn install_service(&self, _dir: &Path) -> i32 {
            0
        }
        fn uninstall_service(&self) -> i32 {
            0
        }
    }

    /// 真的走一遍套接字：握手、状态、改配置写进 host.json、错误请求、协助应答、退出删套接字。
    #[tokio::test]
    async fn socket_end_to_end() {
        let dir = std::env::temp_dir().join(format!("lg-ctl-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("run").join("control.sock");
        let ctl = start_at(Arc::new(TestOs), &dir, sock.clone()).await.expect("listen");
        // 同一个套接字再开一份：不抢
        assert!(start_at(Arc::new(TestOs), &dir, sock.clone()).await.is_none());
        let s = UnixStream::connect(&sock).await.unwrap();
        let (rd, mut wr) = s.into_split();
        let mut rd = BufReader::new(rd);
        let mut next = 0u64;
        let mut call = |req: Request| {
            next += 1;
            encode(&RequestLine { id: next, req })
        };
        async fn reply(rd: &mut BufReader<tokio::net::unix::OwnedReadHalf>, id: u64) -> Response {
            loop {
                let mut l = String::new();
                let n = tokio::time::timeout(Duration::from_secs(10), rd.read_line(&mut l)).await.unwrap().unwrap();
                assert!(n > 0, "连接被关");
                match serde_json::from_str::<ServerLine>(l.trim()).unwrap() {
                    ServerLine::Response(r) if r.id == id => return r.resp,
                    _ => {}
                }
            }
        }
        wr.write_all(call(Request::Hello { lang: "en".into() }).as_bytes()).await.unwrap();
        assert!(matches!(reply(&mut rd, 1).await, Response::Hello { protocol: PROTOCOL_VERSION, .. }));
        wr.write_all(call(Request::Status).as_bytes()).await.unwrap();
        let Response::Status { status } = reply(&mut rd, 2).await else { panic!() };
        assert_eq!(status.rights.manage, ManageRight::Direct);
        assert_eq!(status.account_count, 0);
        assert!(status.sessions.is_empty() && status.assist.is_none());
        wr.write_all(call(Request::SetLockOnEnd { enabled: true }).as_bytes()).await.unwrap();
        assert_eq!(reply(&mut rd, 3).await, Response::ok());
        assert!(HostState::load(&dir.join("host.json")).lock_on_end);
        wr.write_all(call(Request::SetHeadlessDisplay { value: "abc".into() }).as_bytes()).await.unwrap();
        assert!(matches!(reply(&mut rd, 4).await, Response::Error { code: ErrorCode::Invalid, .. }));
        wr.write_all(call(Request::SetHeadlessDisplay { value: "2560x1440".into() }).as_bytes()).await.unwrap();
        assert_eq!(reply(&mut rd, 5).await, Response::ok());
        wr.write_all(call(Request::Settings).as_bytes()).await.unwrap();
        let Response::Settings { settings } = reply(&mut rd, 6).await else { panic!() };
        assert!(settings.lock_on_end && settings.auto_update);
        assert_eq!(settings.headless_display, Some(DisplayMode { width: 2560, height: 1440, refresh: 60 }));
        wr.write_all(call(Request::SetAssistCode { code: Some("short".into()) }).as_bytes()).await.unwrap();
        assert!(matches!(reply(&mut rd, 7).await, Response::Error { code: ErrorCode::Invalid, .. }));
        wr.write_all(call(Request::AssistAnswer { request_id: 9, allow: true }).as_bytes()).await.unwrap();
        assert!(matches!(reply(&mut rd, 8).await, Response::Error { code: ErrorCode::NotFound, .. }));
        wr.write_all(call(Request::Logout { user_id: 1 }).as_bytes()).await.unwrap();
        assert!(matches!(reply(&mut rd, 9).await, Response::Error { code: ErrorCode::NotFound, .. }));
        wr.write_all(call(Request::Login { server: "host:port".into(), username: "u".into(), password: "p".into(), totp: None, replace_server: false }).as_bytes()).await.unwrap();
        assert!(matches!(reply(&mut rd, 10).await, Response::Error { code: ErrorCode::Invalid, .. }));
        wr.write_all(b"{\"id\":77,\"req\":{\"op\":\"no_such\"}}\n").await.unwrap();
        assert!(matches!(reply(&mut rd, 77).await, Response::Error { code: ErrorCode::BadRequest, .. }));
        // 命令行改了 host.json，界面读到的是新的
        let mut st = HostState::load(&dir.join("host.json"));
        st.auto_update = false;
        st.save(&dir.join("host.json")).unwrap();
        wr.write_all(call(Request::Settings).as_bytes()).await.unwrap();
        let Response::Settings { settings } = reply(&mut rd, 11).await else { panic!() };
        assert!(!settings.auto_update && settings.lock_on_end);
        ctl.close();
        assert!(!sock.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn events_tail() {
        let d = std::env::temp_dir().join(format!("lg-events-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let mut s = String::new();
        for i in 0..5 {
            s.push_str(&serde_json::to_string(&HostLogEvent { at: i, session: "s".into(), user: "u".into(), kind: "connect".into(), detail: format!("d{i}"), ai: i == 4 }).unwrap());
            s.push('\n');
        }
        s.push_str("not json\n");
        std::fs::write(d.join("events.log"), s).unwrap();
        let v = read_events(&d, 3);
        assert_eq!(v.iter().map(|e| e.at).collect::<Vec<_>>(), vec![2, 3, 4]);
        assert!(v[2].ai);
        assert!(read_events(&d.join("none"), 3).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
