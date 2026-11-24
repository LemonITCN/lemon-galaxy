//! 主窗口：标题栏 + 侧边栏 + 各页；服务没运行时整页换成「安装并启动服务」。

use super::*;
use crate::tr;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

pub struct MainWindow {
    pub window: gtk::ApplicationWindow,
    top: gtk::Stack,
    pages: gtk::Stack,
    refresh: HashMap<&'static str, Rc<dyn Fn()>>,
    ever_connected: Cell<bool>,
}

impl MainWindow {
    pub fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let window = gtk::ApplicationWindow::new(&ctx.app);
        window.set_title(tr!("LemonGalaxy 被控端", "LemonGalaxy Worker"));
        window.set_icon_name(Some(ICON));
        window.set_default_size(940, 680);
        window.set_size_request(820, 480);
        *ctx.window.borrow_mut() = Some(window.clone());

        let header = gtk::HeaderBar::new();
        header.set_show_close_button(true);
        header.set_title(Some(tr!("LemonGalaxy 被控端", "LemonGalaxy Worker")));
        let menu = gio::Menu::new();
        menu.append(Some(tr!("关于 LemonGalaxy", "About LemonGalaxy")), Some("app.about"));
        menu.append(Some(tr!("退出", "Quit")), Some("app.quit"));
        let mb = gtk::MenuButton::new();
        mb.set_image(Some(&gtk::Image::from_icon_name(Some("open-menu-symbolic"), gtk::IconSize::Button)));
        mb.set_menu_model(Some(&menu));
        header.pack_end(&mb);
        window.set_titlebar(Some(&header));
        header.show_all();

        let overlay = gtk::Overlay::new();
        let top = gtk::Stack::new();
        top.set_transition_type(gtk::StackTransitionType::Crossfade);
        overlay.add(&top);

        // 连接中
        let (connecting, _, _, act) = placeholder("network-transmit-receive-symbolic", tr!("正在连接被控服务…", "Connecting to the LemonGalaxy service…"), "");
        let sp = gtk::Spinner::new();
        sp.start();
        act.pack_start(&sp, false, false, 0);
        top.add_named(&connecting, "connecting");

        // 服务没运行
        top.add_named(&service_down(ctx), "down");

        // 主体
        let main = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let pages = gtk::Stack::new();
        pages.set_transition_type(gtk::StackTransitionType::Crossfade);
        pages.set_hexpand(true);
        let sidebar = gtk::StackSidebar::new();
        sidebar.set_stack(&pages);
        sidebar.set_size_request(180, -1);
        main.pack_start(&sidebar, false, false, 0);
        main.pack_start(&gtk::Separator::new(gtk::Orientation::Vertical), false, false, 0);
        main.pack_start(&pages, true, true, 0);
        top.add_named(&main, "main");

        let mut refresh: HashMap<&'static str, Rc<dyn Fn()>> = HashMap::new();
        pages.add_titled(&overview::build(ctx), "overview", tr!("概览", "Overview"));
        let (w, r) = accounts::build(ctx);
        pages.add_titled(&w, "accounts", tr!("账号与权限", "Accounts"));
        refresh.insert("accounts", r);
        let (w, r) = assist::build(ctx);
        pages.add_titled(&w, "assist", tr!("临时协助", "Assistance"));
        refresh.insert("assist", r);
        let (w, r) = settings::build(ctx);
        pages.add_titled(&w, "settings", tr!("设置", "Settings"));
        refresh.insert("settings", r);
        let (w, r) = logs::build(ctx);
        pages.add_titled(&w, "logs", tr!("事件日志", "Event Log"));
        refresh.insert("logs", r);
        pages.add_titled(&about::build(ctx), "about", tr!("关于", "About"));

        // 页内提示条
        let rev = gtk::Revealer::new();
        rev.set_halign(gtk::Align::Center);
        rev.set_valign(gtk::Align::Start);
        rev.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        let frame = gtk::Frame::new(None);
        frame.style_context().add_class("app-notification");
        let tb = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let ti = gtk::Image::from_icon_name(Some("dialog-warning-symbolic"), gtk::IconSize::Button);
        ti.set_no_show_all(true);
        tb.pack_start(&ti, false, false, 0);
        let tl = gtk::Label::new(None);
        tl.set_line_wrap(true);
        tl.set_max_width_chars(60);
        tb.pack_start(&tl, true, true, 0);
        let tc = gtk::Button::from_icon_name(Some("window-close-symbolic"), gtk::IconSize::Button);
        tc.set_relief(gtk::ReliefStyle::None);
        tb.pack_start(&tc, false, false, 0);
        frame.add(&tb);
        rev.add(&frame);
        overlay.add_overlay(&rev);
        let r2 = rev.clone();
        tc.connect_clicked(move |_| r2.set_reveal_child(false));
        let serial = Rc::new(Cell::new(0u64));
        let r2 = rev.clone();
        ctx.set_toast(move |msg, error| {
            tl.set_text(msg);
            ti.set_visible(error);
            r2.set_reveal_child(true);
            let n = serial.get() + 1;
            serial.set(n);
            let (r3, s2) = (r2.clone(), serial.clone());
            glib::timeout_add_seconds_local(if error { 8 } else { 4 }, move || {
                if s2.get() == n {
                    r3.set_reveal_child(false);
                }
                glib::ControlFlow::Break
            });
        });

        window.add(&overlay);
        let me = Rc::new(Self { window, top, pages, refresh, ever_connected: Cell::new(false) });
        let m2 = me.clone();
        me.pages.connect_visible_child_name_notify(move |p| {
            if let Some(n) = p.visible_child_name() {
                m2.refresh_page(&n);
            }
        });
        let m2 = Rc::downgrade(&me);
        ctx.set_goto(move |p| {
            if let Some(m) = m2.upgrade() {
                m.show_page(p);
            }
        });
        overlay.show_all();
        rev.set_reveal_child(false);
        // 一秒内连不上才显示「服务没运行」，免得一闪
        let m2 = Rc::downgrade(&me);
        glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
            if let Some(m) = m2.upgrade() {
                if !m.ever_connected.get() && m.top.visible_child_name().as_deref() == Some("connecting") {
                    m.top.set_visible_child_name("down");
                }
            }
        });
        me
    }

    fn refresh_page(&self, name: &str) {
        if let Some(r) = self.refresh.get(name) {
            r();
        }
    }

    pub fn show_page(&self, name: &str) {
        self.pages.set_visible_child_name(name);
        self.refresh_page(name);
        self.window.present();
    }

    pub fn set_connected(&self, on: bool) {
        if on {
            self.ever_connected.set(true);
            self.top.set_visible_child_name("main");
            if let Some(n) = self.pages.visible_child_name() {
                self.refresh_page(&n);
            }
        } else {
            self.top.set_visible_child_name("down");
        }
    }
}

/// 服务没运行：说明 + 「安装并启动服务」（pkexec）；没有 pkexec 时给命令。
fn service_down(ctx: &Rc<Ctx>) -> gtk::Widget {
    let (b, _, body, actions) = placeholder(
        "computer-fail-symbolic",
        tr!("被控服务没有运行", "The LemonGalaxy service is not running"),
        tr!(
            "被控服务负责联网和远程控制，这个窗口只是它的遥控器。安装后服务会开机自动运行，没人登录时也能被远程控制。",
            "The service connects this computer and handles remote control; this window only manages it. Once installed it starts at boot, even before anyone signs in."
        ),
    );
    let kernel = crate::desktop::kernel_path();
    let pkexec = crate::desktop::pkexec();
    match (&kernel, &pkexec) {
        (None, _) => {
            body.set_text(tr!("找不到 lemon-galaxy-worker 程序。请重新安装 LemonGalaxy。", "The lemon-galaxy-worker program was not found. Please reinstall LemonGalaxy."));
        }
        (Some(k), Some(p)) => {
            let btn = gtk::Button::with_label(tr!("安装并启动服务", "Install and Start Service"));
            btn.style_context().add_class("suggested-action");
            let sp = gtk::Spinner::new();
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.set_halign(gtk::Align::Center);
            row.pack_start(&btn, false, false, 0);
            row.pack_start(&sp, false, false, 0);
            actions.pack_start(&row, false, false, 0);
            let note = gtk::Label::new(Some(tr!("需要管理员密码。", "Requires an administrator password.")));
            note.style_context().add_class("dim-label");
            note.style_context().add_class("lg-small");
            actions.pack_start(&note, false, false, 0);
            let (k, p, c) = (k.clone(), p.clone(), ctx.clone());
            btn.connect_clicked(move |btn| {
                btn.set_sensitive(false);
                sp.start();
                let (k, p) = (k.clone(), p.clone());
                let k2 = k.clone();
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let _ = tx.send_blocking(crate::desktop::install_service(&p, &k));
                });
                let (btn, sp, c) = (btn.clone(), sp.clone(), c.clone());
                spawn(async move {
                    let (ok, out) = rx.recv().await.unwrap_or((false, String::new()));
                    sp.stop();
                    btn.set_sensitive(true);
                    if ok {
                        c.toast(tr!("服务已安装并启动", "Service installed and started"));
                    } else if out == "cancelled" {
                        c.error(tr!("已取消授权", "Authorization was cancelled"));
                    } else if out.contains("authentication agent") {
                        // 桌面没有 polkit 授权代理，pkexec 弹不出密码框
                        let cmd = format!("sudo {} install-service", k2.display());
                        c.error(&if crate::i18n::zh() { format!("没有找到授权代理，弹不出管理员密码框。请在终端里运行：{cmd}") } else { format!("No polkit agent is running, so no password prompt can be shown. Run this in a terminal: {cmd}") });
                    } else {
                        c.error(&format!("{}{out}", tr!("安装服务失败：", "Could not install the service: ")));
                    }
                });
            });
        }
        (Some(k), None) => {
            let hint = gtk::Label::new(Some(tr!("在终端里运行下面的命令安装：", "Run this command in a terminal to install it:")));
            hint.style_context().add_class("dim-label");
            actions.pack_start(&hint, false, false, 0);
            let cmd = gtk::Label::new(Some(&format!("sudo {} install-service", k.display())));
            cmd.set_selectable(true);
            cmd.style_context().add_class("lg-mono");
            actions.pack_start(&cmd, false, false, 0);
        }
    }
    b.upcast()
}
