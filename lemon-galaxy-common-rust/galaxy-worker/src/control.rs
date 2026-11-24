//! 本机控制接口的消息（docs/linux-desktop.md 第 7 节）。服务端实现在 `control_server.rs`。
//!
//! 图形界面 `lemon-galaxy-worker-gui` 用 `#[path]` 直接引入本文件，所以这里只能依赖 serde、serde_json、
//! galaxy-proto 与标准库，不能引用本 crate 的其他模块。
//!
//! 一行一个 JSON：
//!
//! ```text
//! → {"id":1,"req":{"op":"hello","lang":"zh-CN"}}
//! ← {"id":1,"resp":{"type":"hello","protocol":1,"version":"0.1.0"}}
//! ← {"event":{"type":"status","status":{…}}}        服务主动推送，没有 id
//! ```

use galaxy_proto::{AiMode, ConnMode, PathKind};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 协议版本：只加字段、加操作时不变；改了已有字段的含义才加一。
pub const PROTOCOL_VERSION: u32 = 1;

/// polkit 动作：非 root 用户改配置时要它授权。
pub const MANAGE_ACTION: &str = "cn.lemonit.galaxy.worker.manage";

/// 一行最长（字节）。超过的连接直接断开。
pub const MAX_LINE: usize = 1 << 20;

/// 系统服务（root）的套接字。
pub fn system_socket_path() -> PathBuf {
    if cfg!(target_os = "linux") {
        PathBuf::from("/run/lemon-galaxy-worker/control.sock")
    } else {
        PathBuf::from("/var/run/lemon-galaxy-worker/control.sock")
    }
}

/// 用户级服务的套接字：`$XDG_RUNTIME_DIR/lemon-galaxy-worker/control.sock`，没有 XDG_RUNTIME_DIR 时放数据目录。
pub fn user_socket_path(data_dir: &std::path::Path) -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        Some(d) => PathBuf::from(d).join("lemon-galaxy-worker").join("control.sock"),
        None => data_dir.join("control.sock"),
    }
}

/// 界面按顺序找服务：环境变量 `LEMON_GALAXY_CONTROL_SOCKET` > 系统服务 > 本用户的服务。
pub fn socket_candidates() -> Vec<PathBuf> {
    let mut v = vec![];
    if let Some(p) = std::env::var_os("LEMON_GALAXY_CONTROL_SOCKET").filter(|v| !v.is_empty()) {
        v.push(PathBuf::from(p));
        return v;
    }
    v.push(system_socket_path());
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let data = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home.join(".local/share")).join("lemon-galaxy-worker");
    v.push(user_socket_path(&data));
    let fallback = data.join("control.sock");
    if !v.contains(&fallback) {
        v.push(fallback);
    }
    v
}

// ---------------- 外层 ----------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RequestLine {
    pub id: u64,
    pub req: Request,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ResponseLine {
    pub id: u64,
    pub resp: Response,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EventLine {
    pub event: Event,
}

/// 服务发来的一行：回复或事件。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ServerLine {
    Response(ResponseLine),
    Event(EventLine),
}

pub fn encode<T: Serialize>(v: &T) -> String {
    let mut s = serde_json::to_string(v).unwrap_or_default();
    s.push('\n');
    s
}

// ---------------- 请求 ----------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// 连上后先发：界面语言（服务回的错误信息用这个语言）
    Hello { lang: String },
    Status,
    Accounts,
    /// 可选的系统用户（权限里的「以哪个系统用户运行」）
    SystemUsers,
    /// 登录第一步；需要两步验证码时带上 `totp` 再发一次。更换服务器要 `replace_server`。
    Login {
        server: String,
        username: String,
        password: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        totp: Option<String>,
        #[serde(default)]
        replace_server: bool,
    },
    /// 首次登录设新密码（接在 `LoginStep::NeedNewPassword` 后）
    LoginSetPassword { new_password: String },
    /// 选好系统用户，完成登录（接在 `LoginStep::ChooseSystemUser` 后）
    LoginFinish {
        system_user: String,
        #[serde(default)]
        confirm_root: bool,
    },
    LoginCancel,
    Logout { user_id: u64 },
    SetPolicy {
        user_id: u64,
        policy: Policy,
        #[serde(default)]
        confirm_root: bool,
    },
    SetRemoteEnabled { enabled: bool },
    Disconnect { session_id: String },
    DisconnectAll,
    StopAi,
    /// 开始临时协助；`server` 为空时用已登录的服务器
    AssistStart {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        server: Option<String>,
    },
    AssistStop,
    /// 回应 `Event::AssistRequest`
    AssistAnswer { request_id: u64, allow: bool },
    Settings,
    /// 固定的临时协助验证码（8–16 位字母数字）；None 为每次随机
    SetAssistCode {
        #[serde(default)]
        code: Option<String>,
    },
    SetAutoUpdate { enabled: bool },
    SetLockOnEnd { enabled: bool },
    /// 远程连入时自动解锁（锁屏时系统禁止远程桌面的 Wayland 桌面用）
    SetUnlockOnConnect { enabled: bool },
    /// `off`、空（默认 1920x1080@60）或 `宽x高[@刷新率]`
    SetHeadlessDisplay { value: String },
    /// 开机自动启动服务（systemd）
    SetAutostart { enabled: bool },
    /// 登录界面使用 X11（GDM 的 WaylandEnable=false）
    SetLoginScreenX11 { enabled: bool },
    /// 操作日志最近 `limit` 条
    Events { limit: u32 },
    CheckUpdate,
    ApplyUpdate,
}

// ---------------- 回复 ----------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Ok {
        /// 需要告诉用户的话（如「重启后生效」）
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    Error { code: ErrorCode, message: String },
    Hello { protocol: u32, version: String },
    Status { status: Status },
    Accounts { accounts: Vec<Account> },
    SystemUsers {
        users: Vec<String>,
        /// 用户级服务只能用服务自己的用户
        can_choose: bool,
        default_user: String,
        superuser: String,
    },
    Login { step: LoginStep },
    Settings { settings: Settings },
    Events { events: Vec<LogEntry> },
    Assist { assist: AssistInfo },
    UpdateCheck {
        current: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        latest: Option<UpdateAvailable>,
    },
    Updated {
        /// 新版本号；已是最新时为 None
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
        /// 服务马上自己重启
        restarting: bool,
    },
}

impl Response {
    pub fn ok() -> Self {
        Response::Ok { message: None }
    }
    pub fn err(code: ErrorCode, message: impl Into<String>) -> Self {
        Response::Error { code, message: message.into() }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// 没有这个权限（表见契约第 7 节）
    Denied,
    /// 要管理员授权，但没通过（取消、密码错、没有授权代理）
    AuthFailed,
    /// 本机没有 polkit：只有 root 能改配置
    PolkitUnavailable,
    /// 参数不对
    Invalid,
    NotFound,
    /// 还没有登录任何账号
    NotSignedIn,
    /// 这台机器上不适用（没有 systemd、不是 GDM 等）
    NotApplicable,
    /// 另一个窗口正在做（如临时协助已开）
    Busy,
    /// 要先确认以 root 身份运行
    ConfirmRoot,
    /// 两步验证码不正确
    InvalidTotp,
    /// 用户名或密码不正确
    InvalidCredentials,
    /// 网络、服务器或系统错误
    Failed,
    /// 看不懂的请求
    BadRequest,
}

// ---------------- 数据 ----------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Status {
    pub version: String,
    pub device_id: String,
    pub hostname: String,
    pub data_dir: String,
    pub service: ServiceInfo,
    /// 已登录的账号数（需要重新登录的也算）
    pub account_count: u32,
    pub needs_relogin: u32,
    /// 已连上服务器
    pub online: bool,
    pub server: String,
    pub public_ip: String,
    pub remote_enabled: bool,
    pub sessions: Vec<Session>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assist: Option<AssistInfo>,
    /// 这个连接能做什么
    pub rights: Rights,
    /// 连接方的系统用户名
    pub you: String,
    /// 配置（host.json）的版本：变了说明账号或设置被改过（界面或命令行），界面据此重新读
    #[serde(default)]
    pub config_rev: u64,
}

impl Status {
    pub fn controlled(&self) -> bool {
        !self.sessions.is_empty()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ServiceInfo {
    pub mode: ServiceMode,
    /// 服务以哪个系统用户运行
    pub user: String,
    pub pid: u32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServiceMode {
    /// systemd 系统服务（root）
    System,
    /// systemd 用户服务
    User,
    /// 前台 `run`
    Foreground,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Rights {
    /// 断开远程、停止 AI、临时协助
    pub operate: bool,
    /// 看账号、设置、操作日志
    pub view_config: bool,
    /// 改设置、登录退出、改权限、开机自启、更新
    pub manage: ManageRight,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManageRight {
    /// 直接能改（root，或用户级服务的同一用户）
    Direct,
    /// 每次经 polkit 授权（会弹管理员密码框，几分钟内不再问）
    Polkit,
    Denied,
}

impl Rights {
    pub fn can_manage(&self) -> bool {
        self.manage != ManageRight::Denied
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Session {
    pub id: String,
    /// 谁（显示名（用户名））
    pub from_user: String,
    /// 主控设备 ID
    pub from_device: String,
    pub mode: ConnMode,
    pub path: PathKind,
    pub started_at: i64,
    pub ai_active: bool,
    /// 来自临时协助
    #[serde(default)]
    pub assist: bool,
    /// 这个账号开着「被控时提示」（界面据此决定发不发连入通知）
    #[serde(default = "yes")]
    pub notify: bool,
}

fn yes() -> bool {
    true
}

impl Session {
    /// 走中转
    pub fn relayed(&self) -> bool {
        matches!(self.path, PathKind::RelayUdp | PathKind::RelayTcp)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Account {
    pub user_id: u64,
    pub username: String,
    pub display_name: String,
    /// false = 需要重新登录（服务器上改了密码、被注销等）
    pub active: bool,
    pub added_at: i64,
    pub policy: Policy,
}

/// 账号在本机的权限（与 galaxy_host::HostPolicy 一一对应）。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Policy {
    pub system_user: String,
    pub view: bool,
    pub control: bool,
    pub files: bool,
    pub terminal: bool,
    pub clipboard: bool,
    pub audio: bool,
    pub ai_cap: AiMode,
    /// 被控时显示提示
    pub notify: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LoginStep {
    /// 要两步验证码：带 `totp` 再发一次 Login
    NeedTotp,
    /// 首次登录要设新密码：发 LoginSetPassword
    NeedNewPassword { username: String, display_name: String },
    /// 换服务器会退出本机所有账号：确认后带 `replace_server` 再发一次
    ConfirmServerChange { current_server: String, accounts: u32 },
    /// 选系统用户：发 LoginFinish
    ChooseSystemUser { username: String, display_name: String, default_user: String, users: Vec<String>, can_choose: bool, superuser: String },
    Done { username: String, display_name: String, system_user: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    pub auto_update: bool,
    pub lock_on_end: bool,
    /// 远程连入时自动解锁屏幕（老服务不带这个字段时为 false）
    #[serde(default)]
    pub unlock_on_connect: bool,
    /// 虚拟桌面 / 无头兜底屏；None = 关
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headless_display: Option<DisplayMode>,
    /// 固定的临时协助验证码；None = 每次随机
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assist_code: Option<String>,
    pub autostart: Feature,
    pub login_screen_x11: Feature,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub refresh: u32,
}

/// 系统相关的开关：不适用时 `available` 为 false，`detail` 说明原因。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct Feature {
    pub available: bool,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AssistInfo {
    /// 是这个连接开的（设备码与验证码只给开的人看）
    pub mine: bool,
    pub device_code: String,
    pub code: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LogEntry {
    pub at: i64,
    pub session: String,
    pub user: String,
    pub kind: String,
    pub detail: String,
    pub ai: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UpdateAvailable {
    pub version: String,
    pub size: u64,
    #[serde(default)]
    pub notes: String,
}

// ---------------- 事件 ----------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// 状态变了（在线、会话、远程开关、协助……）；连上后也会先推一次
    Status { status: Status },
    SessionStarted { session: Session },
    SessionEnded { session: Session },
    /// 有人请求临时协助：只推给开协助的连接，回 AssistAnswer
    AssistRequest { request_id: u64, from_user: String, from_device: String },
    /// 这个协助请求已经不用回了（对方取消或超时）
    AssistRequestClosed { request_id: u64 },
    /// 临时协助结束了（只推给开协助的连接）
    AssistEnded { reason: String },
    AiActivity { session_id: String, from_user: String, description: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> Status {
        Status {
            version: "0.1.0".into(),
            device_id: "ab".repeat(16),
            hostname: "pc".into(),
            data_dir: "/var/lib/lemon-galaxy-worker".into(),
            service: ServiceInfo { mode: ServiceMode::System, user: "root".into(), pid: 42 },
            account_count: 1,
            needs_relogin: 0,
            online: true,
            server: "example.com:7443#sha256-x".into(),
            public_ip: "1.2.3.4".into(),
            remote_enabled: true,
            sessions: vec![Session { id: "s1".into(), from_user: "Alice（alice）".into(), from_device: "cd".into(), mode: ConnMode::Control, path: PathKind::RelayUdp, started_at: 1, ai_active: true, assist: false, notify: true }],
            assist: Some(AssistInfo { mine: true, device_code: "123 456 789".into(), code: "ABCDEFGH".into() }),
            rights: Rights { operate: true, view_config: false, manage: ManageRight::Polkit },
            you: "alice".into(),
            config_rev: 1,
        }
    }

    fn round<T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(v: &T) -> String {
        let s = encode(v);
        assert!(s.ends_with('\n') && !s[..s.len() - 1].contains('\n'), "一行一个 JSON");
        let back: T = serde_json::from_str(s.trim()).unwrap();
        assert_eq!(&back, v);
        s
    }

    #[test]
    fn requests_round_trip() {
        let policy = Policy { system_user: "alice".into(), view: true, control: true, files: false, terminal: true, clipboard: true, audio: false, ai_cap: AiMode::ConfirmAll, notify: true };
        let all = vec![
            Request::Hello { lang: "zh-CN".into() },
            Request::Status,
            Request::Accounts,
            Request::SystemUsers,
            Request::Login { server: "h:7443#sha256-x".into(), username: "u".into(), password: "p".into(), totp: Some("123456".into()), replace_server: true },
            Request::LoginSetPassword { new_password: "12345678".into() },
            Request::LoginFinish { system_user: "root".into(), confirm_root: true },
            Request::LoginCancel,
            Request::Logout { user_id: 7 },
            Request::SetPolicy { user_id: 7, policy, confirm_root: false },
            Request::SetRemoteEnabled { enabled: false },
            Request::Disconnect { session_id: "s1".into() },
            Request::DisconnectAll,
            Request::StopAi,
            Request::AssistStart { server: None },
            Request::AssistStop,
            Request::AssistAnswer { request_id: 3, allow: true },
            Request::Settings,
            Request::SetAssistCode { code: Some("ABCD2345".into()) },
            Request::SetAutoUpdate { enabled: true },
            Request::SetLockOnEnd { enabled: true },
            Request::SetUnlockOnConnect { enabled: true },
            Request::SetHeadlessDisplay { value: "2560x1440@60".into() },
            Request::SetAutostart { enabled: false },
            Request::SetLoginScreenX11 { enabled: true },
            Request::Events { limit: 100 },
            Request::CheckUpdate,
            Request::ApplyUpdate,
        ];
        for (i, r) in all.into_iter().enumerate() {
            round(&RequestLine { id: i as u64, req: r });
        }
        // 线上格式
        assert_eq!(encode(&RequestLine { id: 1, req: Request::Status }), "{\"id\":1,\"req\":{\"op\":\"status\"}}\n");
        // 可选字段可以不写
        let r: RequestLine = serde_json::from_str(r#"{"id":2,"req":{"op":"assist_start"}}"#).unwrap();
        assert_eq!(r.req, Request::AssistStart { server: None });
        let r: RequestLine = serde_json::from_str(r#"{"id":3,"req":{"op":"login","server":"s","username":"u","password":"p"}}"#).unwrap();
        assert!(matches!(r.req, Request::Login { totp: None, replace_server: false, .. }));
    }

    #[test]
    fn responses_and_events_round_trip() {
        let resps = vec![
            Response::ok(),
            Response::Ok { message: Some("重启后生效".into()) },
            Response::err(ErrorCode::PolkitUnavailable, "x"),
            Response::Hello { protocol: PROTOCOL_VERSION, version: "0.1.0".into() },
            Response::Status { status: status() },
            Response::Accounts { accounts: vec![] },
            Response::SystemUsers { users: vec!["alice".into(), "root".into()], can_choose: true, default_user: "alice".into(), superuser: "root".into() },
            Response::Login { step: LoginStep::NeedTotp },
            Response::Login { step: LoginStep::NeedNewPassword { username: "u".into(), display_name: "U".into() } },
            Response::Login { step: LoginStep::ConfirmServerChange { current_server: "a".into(), accounts: 2 } },
            Response::Login { step: LoginStep::ChooseSystemUser { username: "u".into(), display_name: "U".into(), default_user: "alice".into(), users: vec!["alice".into()], can_choose: true, superuser: "root".into() } },
            Response::Login { step: LoginStep::Done { username: "u".into(), display_name: "U".into(), system_user: "alice".into() } },
            Response::Settings {
                settings: Settings {
                    auto_update: true,
                    lock_on_end: false,
                    unlock_on_connect: false,
                    headless_display: Some(DisplayMode { width: 1920, height: 1080, refresh: 60 }),
                    assist_code: None,
                    autostart: Feature { available: true, enabled: true, detail: String::new() },
                    login_screen_x11: Feature { available: false, enabled: false, detail: "not gdm".into() },
                },
            },
            Response::Events { events: vec![LogEntry { at: 1, session: "s".into(), user: "u".into(), kind: "connect".into(), detail: "d".into(), ai: false }] },
            Response::Assist { assist: AssistInfo { mine: true, device_code: "1".into(), code: "2".into() } },
            Response::UpdateCheck { current: "0.1.0".into(), latest: Some(UpdateAvailable { version: "0.2.0".into(), size: 10, notes: String::new() }) },
            Response::Updated { version: None, restarting: false },
        ];
        for (i, r) in resps.into_iter().enumerate() {
            let line = round(&ResponseLine { id: i as u64, resp: r.clone() });
            // 界面按 ServerLine 读：回复要认成回复
            assert_eq!(serde_json::from_str::<ServerLine>(line.trim()).unwrap(), ServerLine::Response(ResponseLine { id: i as u64, resp: r }));
        }
        let s = status();
        let events = vec![
            Event::Status { status: s.clone() },
            Event::SessionStarted { session: s.sessions[0].clone() },
            Event::SessionEnded { session: s.sessions[0].clone() },
            Event::AssistRequest { request_id: 1, from_user: "bob".into(), from_device: "ef".into() },
            Event::AssistRequestClosed { request_id: 1 },
            Event::AssistEnded { reason: "stopped".into() },
            Event::AiActivity { session_id: "s1".into(), from_user: "bob".into(), description: "点击".into() },
        ];
        for e in events {
            let line = round(&EventLine { event: e.clone() });
            assert_eq!(serde_json::from_str::<ServerLine>(line.trim()).unwrap(), ServerLine::Event(EventLine { event: e }));
        }
        assert_eq!(encode(&ResponseLine { id: 5, resp: Response::err(ErrorCode::Denied, "no") }), "{\"id\":5,\"resp\":{\"type\":\"error\",\"code\":\"denied\",\"message\":\"no\"}}\n");
    }

    #[test]
    fn socket_paths() {
        assert!(system_socket_path().ends_with("lemon-galaxy-worker/control.sock"));
        assert!(user_socket_path(std::path::Path::new("/d")).ends_with("control.sock"));
        assert!(!socket_candidates().is_empty());
    }
}
