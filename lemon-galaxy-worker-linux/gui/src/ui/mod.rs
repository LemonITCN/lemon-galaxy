//! 界面公共部分：共享上下文与常用控件（设置程序式的分组列表、带说明弹出层的「i」按钮、异步开关）。

pub mod about;
pub mod accounts;
pub mod assist;
pub mod login;
pub mod logs;
pub mod overview;
pub mod settings;
pub mod window;

use crate::client::Client;
use crate::control::*;
use crate::tr;
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::Rc;

pub const APP_ID: &str = "cn.lemonit.galaxy.worker";
pub const ICON: &str = "lemon-galaxy-worker";

/// 所有页面共用
pub struct Ctx {
    pub client: Client,
    pub app: gtk::Application,
    pub status: RefCell<Option<Status>>,
    pub window: RefCell<Option<gtk::ApplicationWindow>>,
    status_listeners: RefCell<Vec<Rc<dyn Fn(&Status)>>>,
    toast: RefCell<Option<Rc<dyn Fn(&str, bool)>>>,
    goto: RefCell<Option<Rc<dyn Fn(&str)>>>,
}

impl Ctx {
    pub fn new(client: Client, app: gtk::Application) -> Rc<Self> {
        Rc::new(Self {
            client,
            app,
            status: RefCell::new(None),
            window: RefCell::new(None),
            status_listeners: RefCell::new(vec![]),
            toast: RefCell::new(None),
            goto: RefCell::new(None),
        })
    }

    pub fn on_status(&self, f: impl Fn(&Status) + 'static) {
        self.status_listeners.borrow_mut().push(Rc::new(f));
    }

    pub fn set_status(&self, st: Status) {
        *self.status.borrow_mut() = Some(st.clone());
        let ls: Vec<_> = self.status_listeners.borrow().clone();
        for l in ls {
            l(&st);
        }
    }

    pub fn rights(&self) -> Option<Rights> {
        self.status.borrow().as_ref().map(|s| s.rights.clone())
    }

    pub fn set_toast(&self, f: impl Fn(&str, bool) + 'static) {
        *self.toast.borrow_mut() = Some(Rc::new(f));
    }

    pub fn set_goto(&self, f: impl Fn(&str) + 'static) {
        *self.goto.borrow_mut() = Some(Rc::new(f));
    }

    /// 页内提示条
    pub fn toast(&self, msg: &str) {
        let f = self.toast.borrow().clone();
        if let Some(f) = f {
            f(msg, false);
        }
    }

    pub fn error(&self, msg: &str) {
        let f = self.toast.borrow().clone();
        if let Some(f) = f {
            f(msg, true);
        }
    }

    pub fn goto(&self, page: &str) {
        let f = self.goto.borrow().clone();
        if let Some(f) = f {
            f(page);
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

    pub fn parent(&self) -> Option<gtk::Window> {
        self.window.borrow().as_ref().map(|w| w.clone().upcast())
    }

    /// 发请求并在出错时提示；成功返回回复
    pub async fn call(&self, req: Request) -> Option<Response> {
        let r = self.client.call(req).await;
        if self.report(&r) {
            Some(r)
        } else {
            None
        }
    }
}

pub fn spawn(f: impl Future<Output = ()> + 'static) {
    glib::MainContext::default().spawn_local(f);
}

pub fn install_css() {
    let css = r#"
.lg-page { padding: 28px 32px 32px 32px; }
.lg-section-title { font-weight: bold; }
.lg-row { padding: 6px 12px; min-height: 34px; }
.lg-info { padding: 2px 4px; min-height: 0; min-width: 0; }
.lg-info-glyph { font-family: serif; font-weight: bold; font-style: italic; font-size: 9pt; min-width: 14px; min-height: 14px; border-radius: 9px; border: 1px solid alpha(currentColor, 0.55); color: alpha(currentColor, 0.75); }
.lg-hero-title { font-size: 17pt; font-weight: bold; }
.lg-code { font-family: monospace; font-size: 22pt; font-weight: bold; }
.lg-mono { font-family: monospace; }
.lg-badge { border-radius: 9px; padding: 0 8px; font-size: smaller; font-weight: bold; background-color: alpha(currentColor, 0.12); }
.lg-badge-ai { color: #c64600; background-color: alpha(#e66100, 0.14); }
.lg-placeholder-title { font-size: 15pt; font-weight: bold; }
.lg-small { font-size: smaller; }
"#;
    let p = gtk::CssProvider::new();
    if p.load_from_data(css.as_bytes()).is_ok() {
        if let Some(screen) = gdk::Screen::default() {
            gtk::StyleContext::add_provider_for_screen(&screen, &p, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
    }
}

pub fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    for c in classes {
        l.style_context().add_class(c);
    }
    l
}

pub fn wrap_label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = label(text, classes);
    l.set_line_wrap(true);
    l.set_line_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_max_width_chars(60);
    l
}

/// 「i」按钮：点开是说明文字
pub fn info_button(text: &str) -> gtk::MenuButton {
    let b = gtk::MenuButton::new();
    // 自己画的圆圈「i」：各图标主题的 information 图标长得不一样（有的是灯泡）
    let glyph = gtk::Label::new(Some("i"));
    glyph.style_context().add_class("lg-info-glyph");
    b.add(&glyph);
    b.style_context().add_class("lg-info");
    b.set_relief(gtk::ReliefStyle::None);
    b.set_valign(gtk::Align::Center);
    b.set_tooltip_text(Some(tr!("说明", "About this")));
    let pop = gtk::Popover::new(Some(&b));
    let l = wrap_label(text, &[]);
    l.set_max_width_chars(46);
    l.set_margin_top(12);
    l.set_margin_bottom(12);
    l.set_margin_start(14);
    l.set_margin_end(14);
    pop.add(&l);
    l.show();
    b.set_popover(Some(&pop));
    b
}

/// 分组：标题 + 带边框的列表
pub struct Section {
    pub root: gtk::Box,
    pub header: gtk::Box,
    pub list: gtk::ListBox,
}

pub fn section(title: &str, info: Option<&str>) -> Section {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    if !title.is_empty() {
        let t = label(title, &["lg-section-title"]);
        header.pack_start(&t, false, false, 0);
        if let Some(i) = info {
            header.pack_start(&info_button(i), false, false, 0);
        }
        root.pack_start(&header, false, false, 0);
    }
    let frame = gtk::Frame::new(None);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.set_activate_on_single_click(true);
    frame.add(&list);
    root.pack_start(&frame, false, false, 0);
    Section { root, header, list }
}

/// 一行：左边标题（可带副标题与「i」），右边控件
pub struct Row {
    pub row: gtk::ListBoxRow,
    /// 标题左边（图标等）；用的时候调 `use_prefix`
    pub prefix: gtk::Box,
    pub subtitle: gtk::Label,
    pub suffix: gtk::Box,
}

pub fn row(title: &str, subtitle: Option<&str>, info: Option<&str>) -> Row {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    h.style_context().add_class("lg-row");
    let prefix = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    prefix.set_valign(gtk::Align::Center);
    prefix.set_no_show_all(true);
    h.pack_start(&prefix, false, false, 0);
    let v = gtk::Box::new(gtk::Orientation::Vertical, 2);
    v.set_valign(gtk::Align::Center);
    let tl = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    let t = label(title, &[]);
    t.set_line_wrap(true);
    tl.pack_start(&t, false, false, 0);
    if let Some(i) = info {
        tl.pack_start(&info_button(i), false, false, 0);
    }
    v.pack_start(&tl, false, false, 0);
    let s = wrap_label(subtitle.unwrap_or(""), &["dim-label", "lg-small"]);
    s.set_no_show_all(subtitle.is_none());
    v.pack_start(&s, false, false, 0);
    h.pack_start(&v, true, true, 0);
    let suffix = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    suffix.set_valign(gtk::Align::Center);
    h.pack_end(&suffix, false, false, 0);
    row.add(&h);
    Row { row, prefix, subtitle: s, suffix }
}

impl Row {
    pub fn use_prefix(&self, w: &impl IsA<gtk::Widget>) {
        self.prefix.pack_start(w, false, false, 0);
        self.prefix.set_no_show_all(false);
    }

    pub fn set_subtitle(&self, s: &str) {
        self.subtitle.set_text(s);
        self.subtitle.set_no_show_all(s.is_empty());
        self.subtitle.set_visible(!s.is_empty());
    }
}

/// 页面：可滚动、内容居中定宽
pub fn page() -> (gtk::ScrolledWindow, gtk::Box) {
    let sw = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    sw.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 22);
    content.style_context().add_class("lg-page");
    content.set_halign(gtk::Align::Center);
    content.set_size_request(600, -1);
    outer.pack_start(&content, false, false, 0);
    sw.add(&outer);
    (sw, content)
}

/// 整页的占位（服务没运行、没有权限等）
pub fn placeholder(icon: &str, title: &str, body: &str) -> (gtk::Box, gtk::Label, gtk::Label, gtk::Box) {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 10);
    b.set_valign(gtk::Align::Center);
    b.set_halign(gtk::Align::Center);
    b.set_margin_start(32);
    b.set_margin_end(32);
    let img = gtk::Image::from_icon_name(Some(icon), gtk::IconSize::Dialog);
    img.set_pixel_size(64);
    img.style_context().add_class("dim-label");
    b.pack_start(&img, false, false, 4);
    let t = gtk::Label::new(Some(title));
    t.style_context().add_class("lg-placeholder-title");
    t.set_line_wrap(true);
    t.set_justify(gtk::Justification::Center);
    b.pack_start(&t, false, false, 0);
    let s = gtk::Label::new(Some(body));
    s.set_line_wrap(true);
    s.set_justify(gtk::Justification::Center);
    s.set_max_width_chars(52);
    s.style_context().add_class("dim-label");
    b.pack_start(&s, false, false, 0);
    let actions = gtk::Box::new(gtk::Orientation::Vertical, 8);
    actions.set_halign(gtk::Align::Center);
    actions.set_margin_top(10);
    b.pack_start(&actions, false, false, 0);
    (b, t, s, actions)
}

/// 开关：用户拨动时发请求，成功才算数，失败弹回去；程序里同步状态用 `set`（不会发请求）。
#[derive(Clone)]
pub struct Toggle {
    pub switch: gtk::Switch,
    quiet: Rc<Cell<bool>>,
}

impl Toggle {
    pub fn new() -> Self {
        let s = gtk::Switch::new();
        s.set_valign(gtk::Align::Center);
        Self { switch: s, quiet: Rc::new(Cell::new(false)) }
    }

    pub fn set(&self, on: bool) {
        if self.switch.is_active() == on && self.switch.state() == on {
            return;
        }
        self.quiet.set(true);
        self.switch.set_active(on);
        self.switch.set_state(on);
        self.quiet.set(false);
    }

    /// `f(on)` 返回是否成功
    pub fn on_toggle<F, Fut>(&self, f: F)
    where
        F: Fn(bool) -> Fut + 'static,
        Fut: Future<Output = bool> + 'static,
    {
        let quiet = self.quiet.clone();
        let f = Rc::new(f);
        let me = self.clone();
        self.switch.connect_state_set(move |_, on| {
            if quiet.get() {
                return glib::Propagation::Proceed;
            }
            let (f, me) = (f.clone(), me.clone());
            spawn(async move {
                let ok = f(on).await;
                me.quiet.set(true);
                if ok {
                    me.switch.set_state(on);
                } else {
                    me.switch.set_active(!on);
                    me.switch.set_state(!on);
                }
                me.quiet.set(false);
            });
            glib::Propagation::Stop
        });
    }
}

/// 确认对话框
pub async fn confirm(parent: Option<gtk::Window>, title: &str, body: &str, ok: &str, destructive: bool) -> bool {
    let kind = if destructive { gtk::MessageType::Warning } else { gtk::MessageType::Question };
    let d = gtk::MessageDialog::new(parent.as_ref(), gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT, kind, gtk::ButtonsType::None, title);
    d.set_secondary_text(Some(body));
    d.add_button(tr!("取消", "Cancel"), gtk::ResponseType::Cancel);
    let b = d.add_button(ok, gtk::ResponseType::Accept);
    if destructive {
        b.style_context().add_class("destructive-action");
    } else {
        b.style_context().add_class("suggested-action");
    }
    d.set_default_response(gtk::ResponseType::Cancel);
    let r = d.run_future().await;
    d.close();
    r == gtk::ResponseType::Accept
}

pub fn copy_text(s: &str) {
    gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD).set_text(s);
}

/// 一个复制按钮
pub fn copy_button(ctx: &Rc<Ctx>, text: impl Fn() -> String + 'static) -> gtk::Button {
    let b = gtk::Button::from_icon_name(Some("edit-copy-symbolic"), gtk::IconSize::Button);
    b.set_relief(gtk::ReliefStyle::None);
    b.set_valign(gtk::Align::Center);
    b.set_tooltip_text(Some(tr!("复制", "Copy")));
    let ctx = ctx.clone();
    b.connect_clicked(move |_| {
        copy_text(&text());
        ctx.toast(tr!("已复制", "Copied"));
    });
    b
}

/// 设备 ID 分组显示：每 4 位一组
pub fn group_id(id: &str) -> String {
    id.chars().collect::<Vec<_>>().chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join(" ")
}

pub fn path_text(p: galaxy_proto::PathKind) -> &'static str {
    use galaxy_proto::PathKind::*;
    match p {
        Connecting => tr!("连接中", "Connecting"),
        Lan => tr!("局域网直连", "LAN"),
        Direct => tr!("直连", "Direct"),
        RelayUdp => tr!("中转", "Relay"),
        RelayTcp => tr!("中转（TCP）", "Relay (TCP)"),
        Closed => tr!("已断开", "Closed"),
    }
}

pub fn ai_mode_text(m: galaxy_proto::AiMode) -> &'static str {
    use galaxy_proto::AiMode::*;
    match m {
        Off => tr!("禁止", "Off"),
        ReadOnly => tr!("只读", "Read only"),
        ConfirmAll => tr!("每步确认", "Confirm every step"),
        ConfirmDangerous => tr!("危险操作确认", "Confirm risky actions"),
        FullTrust => tr!("完全信任", "Full trust"),
    }
}

/// 本地时间（用 glib，跟随系统时区）
pub fn fmt_time(ms: i64, with_date: bool) -> String {
    match glib::DateTime::from_unix_local(ms / 1000) {
        Ok(d) => d.format(if with_date { "%Y-%m-%d %H:%M:%S" } else { "%H:%M" }).map(|s| s.to_string()).unwrap_or_default(),
        Err(_) => String::new(),
    }
}

/// 状态小圆点
pub fn dot(color: Rc<Cell<(f64, f64, f64)>>) -> gtk::DrawingArea {
    let d = gtk::DrawingArea::new();
    d.set_size_request(10, 10);
    d.set_valign(gtk::Align::Center);
    d.connect_draw(move |w, cr| {
        let (r, g, b) = color.get();
        let s = w.allocated_width().min(w.allocated_height()) as f64;
        cr.set_source_rgb(r, g, b);
        cr.arc(w.allocated_width() as f64 / 2.0, w.allocated_height() as f64 / 2.0, s / 2.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        glib::Propagation::Proceed
    });
    d
}

/// 去掉列表里的全部行
pub fn clear(list: &impl IsA<gtk::Container>) {
    for c in list.children() {
        list.remove(&c);
    }
}
