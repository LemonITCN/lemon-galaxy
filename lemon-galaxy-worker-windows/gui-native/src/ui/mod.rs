//! 界面公共部分：共享上下文（`Ctx`：连接、状态、通知、提示条、跳页、提权）与格式化。

pub mod about;
pub mod accounts;
pub mod assist;
pub mod login;
pub mod logs;
pub mod overview;
pub mod settings;
pub mod setup_page;
pub mod window;

use crate::client::Client;
use crate::control::*;
use crate::settings::AppSettings;
use crate::setup::ServiceState;
use crate::tr;
use crate::win::layout::Panel;
use crate::win::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use windows::Win32::Foundation::{FILETIME, HWND, SYSTEMTIME};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Overview,
    Accounts,
    Assist,
    Settings,
    Events,
    About,
}

impl Page {
    pub const ALL: [Page; 6] = [Page::Overview, Page::Accounts, Page::Assist, Page::Settings, Page::Events, Page::About];

    pub fn title(self) -> &'static str {
        match self {
            Page::Overview => tr!("概览", "Overview"),
            Page::Accounts => tr!("账号与权限", "Accounts"),
            Page::Assist => tr!("临时协助", "Assistance"),
            Page::Settings => tr!("设置", "Settings"),
            Page::Events => tr!("事件日志", "Event Log"),
            Page::About => tr!("关于", "About"),
        }
    }
}

/// 一个页面：容器 + 切到它时要做的刷新
pub struct PageView {
    pub panel: Rc<Panel>,
    pub refresh: Option<Rc<dyn Fn()>>,
}

pub fn app_name() -> &'static str {
    tr!("LemonGalaxy 被控端", "LemonGalaxy Worker")
}

/// 所有页面共用
pub struct Ctx {
    pub client: Client,
    pub status: RefCell<Option<Status>>,
    pub service: Cell<ServiceState>,
    pub ever_connected: Cell<bool>,
    /// 启动后 1.2 秒内还连不上才显示「服务没运行」，免得一闪
    pub grace_over: Cell<bool>,
    /// 界面正在做的长动作：install / start / uninstall / uac / assist
    pub busy: Cell<Option<&'static str>>,
    pub settings: RefCell<AppSettings>,
    listeners: RefCell<Vec<Rc<dyn Fn(&Status)>>>,
    changes: RefCell<Vec<Rc<dyn Fn()>>>,
    app_changes: RefCell<Vec<Rc<dyn Fn()>>>,
    toast: RefCell<Option<Rc<dyn Fn(&str, bool)>>>,
    goto: RefCell<Option<Rc<dyn Fn(Page)>>>,
    quit: RefCell<Option<Rc<dyn Fn()>>>,
    rebuild: RefCell<Option<Rc<dyn Fn()>>>,
    main: Cell<isize>,
}

impl Ctx {
    pub fn new(client: Client, settings: AppSettings) -> Rc<Self> {
        Rc::new(Self {
            client,
            status: RefCell::new(None),
            service: Cell::new(ServiceState::Unknown),
            ever_connected: Cell::new(false),
            grace_over: Cell::new(false),
            busy: Cell::new(None),
            settings: RefCell::new(settings),
            listeners: RefCell::new(vec![]),
            changes: RefCell::new(vec![]),
            app_changes: RefCell::new(vec![]),
            toast: RefCell::new(None),
            goto: RefCell::new(None),
            quit: RefCell::new(None),
            rebuild: RefCell::new(None),
            main: Cell::new(0),
        })
    }

    /// 窗口重建（换语言）前：去掉旧窗口登记的回调
    pub fn reset_window_hooks(&self) {
        self.listeners.borrow_mut().clear();
        self.changes.borrow_mut().clear();
        *self.toast.borrow_mut() = None;
        *self.goto.borrow_mut() = None;
    }

    /// 页面：状态变了（连着服务时）
    pub fn on_status(&self, f: impl Fn(&Status) + 'static) {
        self.listeners.borrow_mut().push(Rc::new(f));
    }

    /// 窗口：状态、服务、连接、忙碌任何一样变了
    pub fn on_change(&self, f: impl Fn() + 'static) {
        self.changes.borrow_mut().push(Rc::new(f));
    }

    /// 程序（托盘）：同上，窗口重建时不清掉
    pub fn on_app_change(&self, f: impl Fn() + 'static) {
        self.app_changes.borrow_mut().push(Rc::new(f));
    }

    pub fn changed(&self) {
        let a: Vec<_> = self.app_changes.borrow().clone();
        for f in a {
            f();
        }
        let c: Vec<_> = self.changes.borrow().clone();
        for f in c {
            f();
        }
    }

    pub fn set_status(&self, st: Status) {
        *self.status.borrow_mut() = Some(st.clone());
        self.ever_connected.set(true);
        self.service.set(ServiceState::Running);
        let ls: Vec<_> = self.listeners.borrow().clone();
        for l in ls {
            l(&st);
        }
        self.changed();
    }

    /// 把最近一次状态再推一遍（窗口重建后）
    pub fn replay_status(&self) {
        let st = self.status.borrow().clone();
        if let Some(st) = st {
            self.set_status(st);
        } else {
            self.changed();
        }
    }

    pub fn clear_status(&self) {
        *self.status.borrow_mut() = None;
        self.changed();
    }

    pub fn connected(&self) -> bool {
        self.status.borrow().is_some()
    }

    pub fn rights(&self) -> Rights {
        self.status.borrow().as_ref().map(|s| s.rights.clone()).unwrap_or(Rights { operate: false, view_config: false, manage: ManageRight::Denied })
    }

    /// 改配置要经 UAC 授权（按钮旁画盾牌）
    pub fn needs_elevation(&self) -> bool {
        self.rights().manage == ManageRight::Elevate
    }

    pub fn set_main(&self, h: HWND) {
        self.main.set(key(h));
    }

    pub fn main_hwnd(&self) -> HWND {
        hwnd(self.main.get())
    }

    /// 对话框的父窗口：主窗口显示着时是它，藏在托盘里时没有
    pub fn parent(&self) -> Option<HWND> {
        let h = self.main_hwnd();
        (!h.0.is_null() && Ctl(h).visible()).then_some(h)
    }

    pub fn set_toast(&self, f: impl Fn(&str, bool) + 'static) {
        *self.toast.borrow_mut() = Some(Rc::new(f));
    }

    pub fn set_goto(&self, f: impl Fn(Page) + 'static) {
        *self.goto.borrow_mut() = Some(Rc::new(f));
    }

    pub fn set_quit(&self, f: impl Fn() + 'static) {
        *self.quit.borrow_mut() = Some(Rc::new(f));
    }

    /// 退出界面（被控服务照常运行）
    pub fn quit(&self) {
        let f = self.quit.borrow().clone();
        if let Some(f) = f {
            f();
        }
    }

    pub fn set_rebuild(&self, f: impl Fn() + 'static) {
        *self.rebuild.borrow_mut() = Some(Rc::new(f));
    }

    /// 换了语言：整个窗口重建
    pub fn rebuild(&self) {
        let f = self.rebuild.borrow().clone();
        if let Some(f) = f {
            f();
        }
    }

    pub fn toast(&self, msg: &str) {
        let f = self.toast.borrow().clone();
        if let Some(f) = f {
            f(msg, false);
        }
    }

    pub fn error(&self, msg: &str) {
        crate::settings::log_info(&format!("提示：{msg}"));
        let f = self.toast.borrow().clone();
        if let Some(f) = f {
            f(msg, true);
        }
    }

    pub fn goto(&self, p: Page) {
        let f = self.goto.borrow().clone();
        if let Some(f) = f {
            f(p);
        }
    }

    /// 回复是错误就提示，返回是否成功
    pub fn report(&self, r: &Response) -> bool {
        match r {
            Response::Error { message, .. } => {
                self.error(message);
                false
            }
            Response::Ok { message: Some(m) } => {
                self.toast(m);
                true
            }
            _ => true,
        }
    }

    /// 发请求；服务要管理员授权时弹 UAC，授权后重试一次
    pub fn call(self: &Rc<Self>, req: Request, cb: impl FnOnce(Response) + 'static) {
        let me = self.clone();
        let again = req.clone();
        self.client.call(req, move |r| {
            if !matches!(r, Response::Error { code: ErrorCode::ElevationRequired, .. }) {
                cb(r);
                return;
            }
            let me2 = me.clone();
            me.authorize(move |res| match res {
                Ok(()) => me2.client.call(again, cb),
                Err(m) => cb(Response::err(ErrorCode::AuthFailed, m)),
            });
        });
    }

    /// 发请求，出错时显示提示条；成功给回复，失败给 None
    pub fn act(self: &Rc<Self>, req: Request, cb: impl FnOnce(Option<Response>) + 'static) {
        let me = self.clone();
        self.call(req, move |r| {
            let ok = me.report(&r);
            cb(ok.then_some(r));
        });
    }

    /// 经 UAC 授权（几分钟内这个连接能直接改配置）：向服务要一把钥匙，以管理员身份运行 `--elevate 钥匙 --pipe 管道`
    pub fn authorize(self: &Rc<Self>, then: impl FnOnce(Result<(), String>) + 'static) {
        let me = self.clone();
        self.client.call(Request::ElevationKey, move |r| {
            let key = match r {
                Response::ElevationKey { key, .. } => key,
                Response::Error { message, .. } => return then(Err(message)),
                _ => return then(Err("elevation_key".into())),
            };
            let pipe = me.client.pipe_name().unwrap_or_else(|| SYSTEM_PIPE.to_string());
            me.busy.set(Some("uac"));
            me.changed();
            let owner = key_of(me.parent());
            let args = vec!["--elevate".to_string(), key, "--pipe".into(), pipe];
            let me2 = me.clone();
            spawn_bg(
                move || crate::elevation::run_blocking(&args, owner),
                move |res| {
                    me2.busy.set(None);
                    me2.changed();
                    if res.ok {
                        then(Ok(()))
                    } else if res.cancelled {
                        then(Err(tr!("已取消授权", "Authorization was cancelled").into()))
                    } else {
                        let e = res.error.unwrap_or_default();
                        then(Err(if crate::i18n::zh() { format!("管理员授权没有通过：{e}") } else { format!("Administrator authorization failed: {e}") }))
                    }
                },
            );
        });
    }

    /// 以管理员身份做一件事（安装、启动服务、卸载），期间界面显示转圈。`then(成功)`
    pub fn run_elevated(self: &Rc<Self>, busy: &'static str, args: Vec<String>, then: impl FnOnce(bool) + 'static) {
        self.busy.set(Some(busy));
        self.changed();
        let owner = key_of(self.parent());
        let me = self.clone();
        spawn_bg(
            move || crate::elevation::run_blocking(&args, owner),
            move |r| {
                me.busy.set(None);
                if r.cancelled {
                    me.error(tr!("已取消授权", "Authorization was cancelled"));
                } else if !r.ok {
                    let e = r.error.clone().unwrap_or_default();
                    me.error(&if crate::i18n::zh() { format!("没有完成：{e}") } else { format!("Did not complete: {e}") });
                }
                me.refresh_service();
                me.changed();
                then(r.ok);
            },
        );
    }

    /// 重新查一下服务装没装、在不在运行（没连上服务时定时查）
    pub fn refresh_service(self: &Rc<Self>) {
        let me = self.clone();
        spawn_bg(crate::setup::query_service, move |s| {
            if me.service.get() != s {
                me.service.set(s);
                me.changed();
            }
        });
    }

    /// 确认框（等用户选完才返回）
    pub fn confirm(&self, title: &str, body: &str, ok: &str, destructive: bool) -> bool {
        crate::win::dialog::confirm(self.parent(), app_name(), title, body, ok, tr!("取消", "Cancel"), destructive)
    }
}

pub fn key_of(h: Option<HWND>) -> isize {
    h.map(key).unwrap_or(0)
}

// ---------------- 总体状态 ----------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Overall {
    NotRunning,
    Connecting,
    Disabled,
    Ai,
    Controlled,
    NoAccount,
    Online,
    Offline,
}

pub fn overall(ctx: &Ctx) -> Overall {
    let st = ctx.status.borrow();
    let Some(s) = st.as_ref() else {
        return if matches!(ctx.service.get(), ServiceState::Running | ServiceState::Starting) || !ctx.grace_over.get() { Overall::Connecting } else { Overall::NotRunning };
    };
    if !s.remote_enabled {
        Overall::Disabled
    } else if s.sessions.iter().any(|x| x.ai_active) {
        Overall::Ai
    } else if !s.sessions.is_empty() {
        Overall::Controlled
    } else if s.account_count == 0 {
        Overall::NoAccount
    } else if s.online {
        Overall::Online
    } else {
        Overall::Offline
    }
}

pub fn overall_text(o: Overall) -> &'static str {
    match o {
        Overall::NotRunning => tr!("被控服务没有运行", "Service not running"),
        Overall::Connecting => tr!("正在连接被控服务…", "Connecting to the service…"),
        Overall::Disabled => tr!("已禁止远程", "Remote access disabled"),
        Overall::Ai => tr!("AI 正在操作", "AI is operating"),
        Overall::Controlled => tr!("正在被远程控制", "Being controlled remotely"),
        Overall::NoAccount => tr!("服务运行中，还没有登录账号", "Running, no account signed in"),
        Overall::Online => tr!("在线，等待连接", "Online, waiting for connections"),
        Overall::Offline => tr!("正在连接服务器…", "Connecting to the server…"),
    }
}

/// 状态点的颜色
pub fn overall_color(o: Overall) -> u32 {
    match o {
        Overall::Online => rgb(0x2E, 0xB8, 0x6B),
        Overall::Controlled | Overall::Ai | Overall::Offline => rgb(0xE8, 0x96, 0x1E),
        Overall::Disabled | Overall::NotRunning => rgb(0xE0, 0x3E, 0x3E),
        _ => rgb(0x9A, 0x9A, 0x9A),
    }
}

// ---------------- 格式化 ----------------

/// 设备 ID 分组显示：每 4 位一组
pub fn group_id(id: &str) -> String {
    id.chars().collect::<Vec<_>>().chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join(" ")
}

pub fn group_by(s: &str, n: usize) -> String {
    s.chars().collect::<Vec<_>>().chunks(n).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join(" ")
}

/// 服务器地址只显示「主机:端口」，指纹太长
pub fn server_host(s: &str) -> &str {
    match s.find('#') {
        Some(i) if i > 0 => &s[..i],
        _ => s,
    }
}

pub fn who(user: &str) -> String {
    if user.is_empty() {
        tr!("未登录用户", "Anonymous").into()
    } else {
        user.into()
    }
}

pub fn short_device(d: &str) -> String {
    d.chars().take(8).collect()
}

pub fn path_text(p: galaxy_proto::PathKind) -> &'static str {
    use galaxy_proto::PathKind::*;
    match p {
        Connecting => tr!("连接中", "Connecting"),
        Lan => tr!("局域网直连", "LAN"),
        Direct => tr!("直连", "Direct"),
        RelayUdp => tr!("中转", "Relayed"),
        RelayTcp => tr!("中转（TCP）", "Relayed (TCP)"),
        Closed => tr!("已断开", "Closed"),
    }
}

pub fn ai_mode_text(m: galaxy_proto::AiMode) -> &'static str {
    use galaxy_proto::AiMode::*;
    match m {
        Off => tr!("禁止", "Off"),
        ReadOnly => tr!("只读", "Read only"),
        ConfirmAll => tr!("每步确认", "Confirm every step"),
        ConfirmDangerous => tr!("危险操作确认", "Confirm risky steps"),
        FullTrust => tr!("完全信任", "Full trust"),
    }
}

/// 本地时间（跟随系统时区）
pub fn fmt_time(ms: i64, with_date: bool) -> String {
    let t = (ms.max(0) as u64) * 10_000 + 116_444_736_000_000_000;
    let ft = FILETIME { dwLowDateTime: t as u32, dwHighDateTime: (t >> 32) as u32 };
    let mut local = FILETIME::default();
    let mut st = SYSTEMTIME::default();
    unsafe {
        if windows::Win32::Storage::FileSystem::FileTimeToLocalFileTime(&ft, &mut local).is_err() || windows::Win32::System::Time::FileTimeToSystemTime(&local, &mut st).is_err() {
            return String::new();
        }
    }
    if with_date {
        format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond)
    } else {
        format!("{:02}:{:02}", st.wHour, st.wMinute)
    }
}

pub fn since(ms: i64) -> String {
    let t = fmt_time(ms, false);
    if crate::i18n::zh() {
        format!("{t} 起")
    } else {
        format!("since {t}")
    }
}

/// 复制并提示
pub fn copy(ctx: &Ctx, s: &str) {
    if copy_text(s) {
        ctx.toast(tr!("已复制", "Copied"));
    }
}

/// 「复制」按钮
pub fn copy_button(ctx: &Rc<Ctx>, parent: HWND, text: impl Fn() -> String + 'static) -> Ctl {
    let b = button(parent, tr!("复制", "Copy"));
    set_width(b, pref(b).0.min(px(64)).max(px(56)));
    let c = ctx.clone();
    on_click(b, move || copy(&c, &text()));
    b
}

/// 「需要管理员权限」之类的整页占位：标题 + 说明，居中
pub struct Placeholder {
    pub title: Ctl,
    pub body: Ctl,
}

pub fn placeholder(parent: HWND, title: &str, body: &str) -> Placeholder {
    Placeholder { title: center_label(parent, title, Font::Hero, Tone::Normal), body: center_label(parent, body, Font::Normal, Tone::Dim) }
}

impl Placeholder {
    pub fn place(&self, f: &mut crate::win::layout::Flow) {
        f.gap(40);
        f.center_text(self.title, px(460));
        f.gap(8);
        f.center_text(self.body, px(460));
    }
}

/// 开关（复选框）：用户点了就发请求，成功才算数，失败弹回去；程序里同步状态用 `set`（不发请求）
pub fn toggle(c: Ctl, f: impl Fn(bool, Box<dyn FnOnce(bool)>) + 'static) {
    on_click(c, move || {
        let on = c.checked();
        c.enable(false);
        f(
            on,
            Box::new(move |ok| {
                c.enable(true);
                if !ok {
                    c.set_checked(!on);
                }
            }),
        );
    });
}

/// 发请求的开关：`req(on)` 是要发的请求
pub fn req_toggle(ctx: &Rc<Ctx>, c: Ctl, req: impl Fn(bool) -> Request + 'static) {
    let ctx = ctx.clone();
    toggle(c, move |on, done| {
        ctx.act(req(on), move |r| done(r.is_some()));
    });
}
