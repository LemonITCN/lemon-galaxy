//! 程序：单实例、主窗口、托盘、连入通知、协助请求。

use crate::client::{Client, Signal};
use crate::control::*;
use crate::tr;
use crate::tray::{Icons, Tray, TrayCmd, TrayState};
use crate::ui::assist::Asks;
use crate::ui::window::MainWindow;
use crate::ui::{spawn, Ctx, APP_ID, ICON};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct Gui {
    ctx: Rc<Ctx>,
    win: Rc<MainWindow>,
    asks: Rc<Asks>,
    tray: RefCell<Option<Tray>>,
    /// 托盘宿主在（关窗口时藏到托盘；不在就最小化）
    tray_host: Cell<bool>,
    /// 有托盘时窗口藏起来程序也不退出
    _hold: RefCell<Option<gio::ApplicationHoldGuard>>,
}

pub fn run() -> i32 {
    let tray_only = std::env::args().any(|a| a == "--tray");
    let app = gtk::Application::new(Some(APP_ID), gio::ApplicationFlags::empty());
    app.connect_startup(|_| crate::ui::install_css());
    if app.register(None::<&gio::Cancellable>).is_err() {
        eprintln!("cannot register application");
        return 1;
    }
    if app.is_remote() {
        // 已经在运行：再开一次就把窗口切到前面；登录自启动的那份不打扰
        if !tray_only {
            app.activate();
        }
        // 直接退出：不走 GApplication 的析构（远端实例析构时 GLib 会警告没注销）
        std::process::exit(0);
    }
    let gui: Rc<RefCell<Option<Rc<Gui>>>> = Rc::new(RefCell::new(None));
    let first = Rc::new(Cell::new(true));
    let g2 = gui.clone();
    app.connect_activate(move |app| {
        if let Some(g) = g2.borrow().as_ref() {
            g.win.window.present();
            return;
        }
        let g = Gui::new(app);
        let show = !(first.replace(false) && tray_only);
        if show {
            g.win.window.show();
            g.win.window.present();
        }
        *g2.borrow_mut() = Some(g);
    });
    let code = app.run_with_args::<&str>(&[]);
    if let Some(g) = gui.borrow().as_ref() {
        if let Some(t) = g.tray.borrow().as_ref() {
            t.shutdown();
        }
    }
    code.value()
}

impl Gui {
    fn new(app: &gtk::Application) -> Rc<Self> {
        let client = Client::start();
        let ctx = Ctx::new(client.clone(), app.clone());
        let win = MainWindow::new(&ctx);
        let me = Rc::new(Self { ctx: ctx.clone(), win: win.clone(), asks: Rc::new(Asks::default()), tray: RefCell::new(None), tray_host: Cell::new(false), _hold: RefCell::new(None) });

        // 菜单
        let about = gio::SimpleAction::new("about", None);
        let w = win.clone();
        about.connect_activate(move |_, _| w.show_page("about"));
        app.add_action(&about);
        let act = gio::SimpleAction::new("activate-window", None);
        let w = win.clone();
        act.connect_activate(move |_, _| {
            w.window.show();
            w.window.present();
        });
        app.add_action(&act);
        let disc = gio::SimpleAction::new("disconnect-session", Some(glib::VariantTy::STRING));
        let c = ctx.clone();
        disc.connect_activate(move |_, v| {
            let Some(id) = v.and_then(|v| v.get::<String>()) else { return };
            let c = c.clone();
            spawn(async move {
                let _ = c.client.call(Request::Disconnect { session_id: id }).await;
            });
        });
        app.add_action(&disc);
        let quit = gio::SimpleAction::new("quit", None);
        let a = app.clone();
        quit.connect_activate(move |_, _| a.quit());
        app.add_action(&quit);

        // 关窗口：有托盘就藏起来，没有就最小化
        let m = Rc::downgrade(&me);
        win.window.connect_delete_event(move |w, _| {
            match m.upgrade() {
                Some(m) if m.tray_host.get() && m.tray.borrow().is_some() => w.hide(),
                _ => w.iconify(),
            }
            glib::Propagation::Stop
        });

        // 托盘
        let (tx, rx) = async_channel::unbounded();
        if let Some(t) = Tray::start(tx, Icons::load()) {
            *me.tray.borrow_mut() = Some(t);
            me.tray_host.set(true);
            *me._hold.borrow_mut() = Some(app.hold());
        }
        let m = Rc::downgrade(&me);
        spawn(async move {
            while let Ok(cmd) = rx.recv().await {
                let Some(m) = m.upgrade() else { break };
                m.tray_cmd(cmd);
            }
        });

        let m = Rc::downgrade(&me);
        ctx.on_status(move |st| {
            if let Some(m) = m.upgrade() {
                m.update_tray(Some(st));
            }
        });

        let m = Rc::downgrade(&me);
        client.subscribe(move |s| {
            if let Some(m) = m.upgrade() {
                m.on_signal(s);
            }
        });
        me
    }

    fn update_tray(&self, st: Option<&Status>) {
        if let Some(t) = self.tray.borrow().as_ref() {
            t.update(TrayState::from_status(st));
        }
    }

    fn tray_cmd(self: &Rc<Self>, cmd: TrayCmd) {
        match cmd {
            TrayCmd::Open => {
                self.win.window.show();
                self.win.window.present();
            }
            TrayCmd::Page(p) => {
                self.win.window.show();
                self.win.show_page(p);
            }
            TrayCmd::SetRemote(on) => {
                let c = self.ctx.clone();
                spawn(async move {
                    let r = c.client.call(Request::SetRemoteEnabled { enabled: on }).await;
                    if let Response::Error { message, .. } = r {
                        c.app.send_notification(Some("error"), &notification("LemonGalaxy", &message));
                    }
                });
            }
            TrayCmd::DisconnectAll => {
                let c = self.ctx.clone();
                spawn(async move {
                    let _ = c.client.call(Request::DisconnectAll).await;
                });
            }
            TrayCmd::Quit => self.ctx.app.quit(),
            TrayCmd::Host(on) => {
                self.tray_host.set(on);
                // 托盘没了而窗口藏着：拿出来，免得找不到
                if !on && !self.win.window.is_visible() {
                    self.win.window.show();
                    self.win.window.iconify();
                }
            }
        }
    }

    fn on_signal(self: &Rc<Self>, s: &Signal) {
        match s {
            Signal::Connected => {
                let me = self.clone();
                spawn(async move {
                    let c = &me.ctx.client;
                    let _ = c.call(Request::Hello { lang: crate::i18n::lang_tag().into() }).await;
                    if let Response::Status { status } = c.call(Request::Status).await {
                        me.ctx.set_status(status);
                        me.win.set_connected(true);
                    }
                });
            }
            Signal::Disconnected => {
                *self.ctx.status.borrow_mut() = None;
                self.win.set_connected(false);
                self.update_tray(None);
                self.asks.close_all();
            }
            Signal::Event(e) => match e {
                Event::Status { status } => self.ctx.set_status(status.clone()),
                Event::SessionStarted { session } => {
                    if session.notify {
                        let who = if session.from_user.is_empty() { tr!("有人", "Someone").to_string() } else { session.from_user.clone() };
                        let dev: String = session.from_device.chars().take(8).collect();
                        let body = if crate::i18n::zh() { format!("{who} 从设备 {dev} 连入。") } else { format!("{who} connected from device {dev}.") };
                        let n = notification(tr!("有人连入了这台电脑", "Someone connected to this computer"), &body);
                        n.add_button(tr!("查看", "Show"), "app.activate-window");
                        n.add_button_with_target_value(tr!("断开", "Disconnect"), "app.disconnect-session", Some(&session.id.to_variant()));
                        self.ctx.app.send_notification(Some(&format!("session-{}", session.id)), &n);
                    }
                }
                Event::SessionEnded { session } => self.ctx.app.withdraw_notification(&format!("session-{}", session.id)),
                Event::AssistRequest { request_id, from_user, from_device } => {
                    self.asks.ask(&self.ctx, *request_id, from_user, from_device);
                    let n = notification(tr!("有人请求控制这台电脑", "Someone wants to control this computer"), tr!("在弹出的窗口里选择允许或拒绝。", "Choose Allow or Deny in the dialog."));
                    n.set_priority(gio::NotificationPriority::Urgent);
                    self.ctx.app.send_notification(Some(&format!("assist-{request_id}")), &n);
                }
                Event::AssistRequestClosed { request_id } => {
                    self.asks.closed(*request_id);
                    self.ctx.app.withdraw_notification(&format!("assist-{request_id}"));
                }
                Event::AssistEnded { .. } => self.ctx.toast(tr!("临时协助已结束", "Assistance ended")),
                Event::AiActivity { .. } => {}
            },
        }
    }
}

/// 桌面通知（右上角）。不设默认动作：有的通知服务会把它画成一个没有文字的按钮
fn notification(title: &str, body: &str) -> gio::Notification {
    let n = gio::Notification::new(title);
    n.set_body(Some(body));
    n.set_icon(&gio::ThemedIcon::new(ICON));
    n
}
