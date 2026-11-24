//! 托盘（StatusNotifierItem，ksni 纯 Rust 实现）：状态图标（空闲 / 被控中 / 离线）、打开主界面、允许远程、
//! 临时协助、断开所有远程、退出托盘。托盘线程只把点击转成命令交回 GTK 主线程。

use crate::control::Status;
use crate::tr;
use gtk::{gdk_pixbuf, gio, glib};
use ksni::blocking::TrayMethods;
use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{Icon, MenuItem, ToolTip};

pub enum TrayCmd {
    Open,
    Page(&'static str),
    SetRemote(bool),
    DisconnectAll,
    Quit,
    /// 托盘宿主出现 / 消失（面板重启等）
    Host(bool),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Look {
    Idle,
    Active,
    Offline,
}

#[derive(Clone, Default, PartialEq)]
pub struct TrayState {
    pub connected: bool,
    pub online: bool,
    pub signed_in: bool,
    pub remote_enabled: bool,
    /// (谁, 是否 AI)
    pub sessions: Vec<(String, bool)>,
    pub can_enable: bool,
    pub can_disable: bool,
    pub operate: bool,
}

impl TrayState {
    pub fn from_status(st: Option<&Status>) -> Self {
        match st {
            None => Self::default(),
            Some(s) => Self {
                connected: true,
                online: s.online,
                signed_in: s.account_count > 0,
                remote_enabled: s.remote_enabled,
                sessions: s.sessions.iter().map(|x| (if x.from_user.is_empty() { tr!("未登录用户", "Anonymous").to_string() } else { x.from_user.clone() }, x.ai_active)).collect(),
                can_enable: s.rights.can_manage(),
                can_disable: s.rights.can_manage() || s.rights.operate,
                operate: s.rights.operate,
            },
        }
    }

    fn look(&self) -> Look {
        if !self.sessions.is_empty() {
            Look::Active
        } else if self.connected && self.remote_enabled && (self.online || !self.signed_in) {
            Look::Idle
        } else {
            Look::Offline
        }
    }

    fn text(&self) -> String {
        if !self.connected {
            return tr!("被控服务没有运行", "The LemonGalaxy service is not running").into();
        }
        if !self.remote_enabled {
            return tr!("已禁止远程", "Remote access disabled").into();
        }
        match self.sessions.len() {
            0 => {}
            1 => {
                let (who, ai) = &self.sessions[0];
                return if crate::i18n::zh() {
                    format!("{who} 正在控制{}", if *ai { "（AI）" } else { "" })
                } else {
                    format!("{who} is controlling this computer{}", if *ai { " (AI)" } else { "" })
                };
            }
            n => return if crate::i18n::zh() { format!("{n} 个连接正在控制") } else { format!("{n} sessions are controlling this computer") },
        }
        if !self.signed_in {
            tr!("还没有登录账号", "No account signed in").into()
        } else if self.online {
            tr!("在线，等待连接", "Online, waiting for connections").into()
        } else {
            tr!("正在连接服务器…", "Connecting to the server…").into()
        }
    }
}

/// 三种状态的图标：主题里装了就用名字（跟随主题上色），没装就用内嵌的图
#[derive(Clone, Default)]
pub struct Icons {
    use_names: bool,
    idle: Vec<Icon>,
    active: Vec<Icon>,
    offline: Vec<Icon>,
}

const IDLE_SVG: &str = include_str!("../../packaging/icons/hicolor/scalable/status/lemon-galaxy-worker-idle.svg");
const ACTIVE_SVG: &str = include_str!("../../packaging/icons/hicolor/scalable/status/lemon-galaxy-worker-active.svg");
const OFFLINE_SVG: &str = include_str!("../../packaging/icons/hicolor/scalable/status/lemon-galaxy-worker-offline-symbolic.svg");

fn render(svg: &str, sizes: &[i32]) -> Vec<Icon> {
    let mut v = vec![];
    for &s in sizes {
        let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from(svg.as_bytes()));
        let Ok(pb) = gdk_pixbuf::Pixbuf::from_stream_at_scale(&stream, s, s, true, None::<&gio::Cancellable>) else { continue };
        let pb = if pb.has_alpha() { pb } else { pb.add_alpha(false, 0, 0, 0).unwrap_or(pb) };
        let (w, h, stride, n) = (pb.width(), pb.height(), pb.rowstride() as usize, pb.n_channels() as usize);
        let bytes = pb.read_pixel_bytes();
        let mut data = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            for x in 0..w as usize {
                let p = &bytes[y * stride + x * n..];
                // RGBA → ARGB（网络字节序）
                data.extend_from_slice(&[p[3], p[0], p[1], p[2]]);
            }
        }
        v.push(Icon { width: w, height: h, data });
    }
    v
}

impl Icons {
    pub fn load() -> Self {
        let sizes = [16, 22, 24, 32, 48];
        Self {
            // LEMON_GALAXY_TRAY_PIXMAP=1：只用内嵌图（排查托盘宿主找不到图标时用）
            use_names: std::env::var_os("LEMON_GALAXY_TRAY_PIXMAP").is_none() && crate::desktop::icon_installed("lemon-galaxy-worker-idle-symbolic"),
            idle: render(IDLE_SVG, &sizes),
            active: render(ACTIVE_SVG, &sizes),
            // 离线图标是 symbolic 的深灰色，内嵌时换成中灰，深浅面板上都看得见
            offline: render(&OFFLINE_SVG.replace("#2e3436", "#9a9996"), &sizes),
        }
    }
}

pub struct LgTray {
    tx: async_channel::Sender<TrayCmd>,
    state: TrayState,
    icons: Icons,
}

impl ksni::Tray for LgTray {
    fn id(&self) -> String {
        "lemon-galaxy-worker".into()
    }

    fn title(&self) -> String {
        "LemonGalaxy".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn status(&self) -> ksni::Status {
        if self.state.look() == Look::Active {
            ksni::Status::NeedsAttention
        } else {
            ksni::Status::Active
        }
    }

    fn icon_name(&self) -> String {
        if !self.icons.use_names {
            return String::new();
        }
        match self.state.look() {
            Look::Idle => "lemon-galaxy-worker-idle-symbolic",
            // 被控中用彩色图标（不跟主题变色），一眼能看出来
            Look::Active => "lemon-galaxy-worker-active",
            Look::Offline => "lemon-galaxy-worker-offline-symbolic",
        }
        .into()
    }

    fn attention_icon_name(&self) -> String {
        if self.icons.use_names {
            "lemon-galaxy-worker-active".into()
        } else {
            String::new()
        }
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        match self.state.look() {
            Look::Idle => self.icons.idle.clone(),
            Look::Active => self.icons.active.clone(),
            Look::Offline => self.icons.offline.clone(),
        }
    }

    fn attention_icon_pixmap(&self) -> Vec<Icon> {
        self.icons.active.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip { title: "LemonGalaxy".into(), description: self.state.text(), ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send_blocking(TrayCmd::Open);
    }

    fn watcher_online(&self) {
        let _ = self.tx.send_blocking(TrayCmd::Host(true));
    }

    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        let _ = self.tx.send_blocking(TrayCmd::Host(false));
        // 继续等宿主回来（面板重启）
        true
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let s = &self.state;
        let mut m: Vec<MenuItem<Self>> = vec![
            StandardItem { label: s.text(), enabled: false, ..Default::default() }.into(),
            MenuItem::Separator,
            StandardItem {
                label: tr!("打开 LemonGalaxy", "Open LemonGalaxy").into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send_blocking(TrayCmd::Open);
                }),
                ..Default::default()
            }
            .into(),
        ];
        if s.connected {
            m.push(
                CheckmarkItem {
                    label: tr!("允许远程控制", "Allow Remote Control").into(),
                    checked: s.remote_enabled,
                    enabled: if s.remote_enabled { s.can_disable } else { s.can_enable },
                    activate: Box::new(|t: &mut Self| {
                        let on = !t.state.remote_enabled;
                        let _ = t.tx.send_blocking(TrayCmd::SetRemote(on));
                    }),
                    ..Default::default()
                }
                .into(),
            );
            m.push(
                StandardItem {
                    label: tr!("临时协助…", "Remote Assistance…").into(),
                    activate: Box::new(|t: &mut Self| {
                        let _ = t.tx.send_blocking(TrayCmd::Page("assist"));
                    }),
                    ..Default::default()
                }
                .into(),
            );
            m.push(
                StandardItem {
                    label: tr!("断开所有远程", "Disconnect Everyone").into(),
                    enabled: s.operate && !s.sessions.is_empty(),
                    activate: Box::new(|t: &mut Self| {
                        let _ = t.tx.send_blocking(TrayCmd::DisconnectAll);
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }
        m.push(MenuItem::Separator);
        m.push(
            StandardItem {
                label: tr!("退出托盘", "Quit Tray").into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send_blocking(TrayCmd::Quit);
                }),
                ..Default::default()
            }
            .into(),
        );
        m
    }
}

pub struct Tray {
    handle: ksni::blocking::Handle<LgTray>,
    last: std::cell::RefCell<TrayState>,
}

impl Tray {
    /// 连不上会话总线时返回 None；宿主（StatusNotifierWatcher）在不在经 `TrayCmd::Host` 通知
    pub fn start(tx: async_channel::Sender<TrayCmd>, icons: Icons) -> Option<Self> {
        let t = LgTray { tx, state: TrayState::default(), icons };
        // 托盘宿主可能比我们晚启动（登录时自启动）：先注册着，宿主来了自动显示
        match t.assume_sni_available(true).spawn() {
            Ok(handle) => Some(Self { handle, last: Default::default() }),
            Err(e) => {
                eprintln!("tray: {e}");
                None
            }
        }
    }

    pub fn update(&self, st: TrayState) {
        if *self.last.borrow() == st {
            return;
        }
        *self.last.borrow_mut() = st.clone();
        self.handle.update(move |t| t.state = st);
    }

    pub fn shutdown(&self) {
        self.handle.shutdown().wait();
    }
}
