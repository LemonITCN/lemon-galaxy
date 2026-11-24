//! 概览：设备名、设备 ID、服务状态、版本；允许 / 禁止远程；正在控制本机的会话（逐个断开、停止 AI）。

use super::*;
use crate::control::*;
use crate::tr;
use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

fn infobar(kind: gtk::MessageType, text: &str, button: &str) -> (gtk::InfoBar, gtk::Label) {
    let bar = gtk::InfoBar::new();
    bar.set_message_type(kind);
    let l = wrap_label(text, &[]);
    bar.content_area().add(&l);
    bar.add_button(button, gtk::ResponseType::Accept);
    bar.show_all();
    bar.set_no_show_all(true);
    bar.hide();
    (bar, l)
}

pub fn build(ctx: &Rc<Ctx>) -> gtk::Widget {
    let (scroll, content) = page();

    // 抬头：图标、设备名、状态、设备 ID
    let hero = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    let icon = gtk::Image::from_icon_name(Some(ICON), gtk::IconSize::Dialog);
    icon.set_pixel_size(64);
    icon.set_valign(gtk::Align::Start);
    hero.pack_start(&icon, false, false, 0);
    let hv = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let name = label("", &["lg-hero-title"]);
    hv.pack_start(&name, false, false, 0);
    let sb = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let color = Rc::new(Cell::new((0.6, 0.6, 0.6)));
    let d = dot(color.clone());
    sb.pack_start(&d, false, false, 0);
    let state = label("", &[]);
    sb.pack_start(&state, false, false, 0);
    hv.pack_start(&sb, false, false, 0);
    let ib = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    let idl = label(tr!("设备 ID", "Device ID"), &["dim-label"]);
    ib.pack_start(&idl, false, false, 0);
    let id = label("", &["lg-mono", "dim-label"]);
    id.set_selectable(true);
    // 可选中的文字会抢焦点并整段高亮，切页时很难看
    id.set_can_focus(false);
    id.set_margin_start(6);
    ib.pack_start(&id, false, false, 0);
    let c2 = ctx.clone();
    ib.pack_start(&copy_button(ctx, move || c2.status.borrow().as_ref().map(|s| s.device_id.clone()).unwrap_or_default()), false, false, 0);
    hv.pack_start(&ib, false, false, 0);
    hero.pack_start(&hv, true, true, 0);
    content.pack_start(&hero, false, false, 0);

    // 提示条
    let (no_account, _) = infobar(gtk::MessageType::Info, tr!("还没有登录账号。登录后，这台电脑会出现在账号的设备列表里。", "No account is signed in. Sign in so this computer shows up in your device list."), tr!("登录账号…", "Sign In…"));
    let c = ctx.clone();
    no_account.connect_response(move |_, _| super::login::open(&c));
    content.pack_start(&no_account, false, false, 0);
    let (relogin, _) = infobar(gtk::MessageType::Warning, tr!("有账号被管理员重置，需要重新登录后才能被它控制。", "An account was reset by an administrator and must sign in again."), tr!("查看账号", "View Accounts"));
    let c = ctx.clone();
    relogin.connect_response(move |_, _| c.goto("accounts"));
    content.pack_start(&relogin, false, false, 0);

    // 远程访问
    let sec = section(tr!("远程访问", "Remote Access"), None);
    let allow = row(
        tr!("允许远程控制", "Allow remote control"),
        None,
        Some(tr!(
            "关闭后，所有账号都连不进来，正在进行的连接会立即断开。屏幕前的用户可以直接关闭；重新允许需要管理员授权。",
            "When off, no account can connect and active sessions end immediately. The person at this computer can turn it off; turning it back on needs administrator authorization."
        )),
    );
    let toggle = Toggle::new();
    allow.suffix.pack_start(&toggle.switch, false, false, 0);
    sec.list.add(&allow.row);
    let c = ctx.clone();
    toggle.on_toggle(move |on| {
        let c = c.clone();
        async move { c.call(Request::SetRemoteEnabled { enabled: on }).await.is_some() }
    });
    let server_row = row(tr!("服务器", "Server"), None, None);
    let server = label("", &["dim-label"]);
    server.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    server.set_max_width_chars(32);
    server.set_selectable(true);
    server.set_can_focus(false);
    server_row.suffix.pack_start(&server, false, false, 0);
    sec.list.add(&server_row.row);
    let ip_row = row(tr!("公网 IP", "Public IP"), None, None);
    let ip = label("", &["dim-label"]);
    ip.set_selectable(true);
    ip.set_can_focus(false);
    ip_row.suffix.pack_start(&ip, false, false, 0);
    sec.list.add(&ip_row.row);
    let acc_row = row(tr!("已登录账号", "Signed-in accounts"), None, None);
    let accn = label("", &["dim-label"]);
    acc_row.suffix.pack_start(&accn, false, false, 0);
    sec.list.add(&acc_row.row);
    let ver_row = row(tr!("版本", "Version"), None, None);
    let ver = label("", &["dim-label"]);
    ver_row.suffix.pack_start(&ver, false, false, 0);
    sec.list.add(&ver_row.row);
    content.pack_start(&sec.root, false, false, 0);

    // 当前连接
    let sess = section(tr!("当前连接", "Active Sessions"), None);
    let stop_ai = gtk::Button::with_label(tr!("停止 AI 操作", "Stop AI"));
    let disc_all = gtk::Button::with_label(tr!("全部断开", "Disconnect All"));
    disc_all.style_context().add_class("destructive-action");
    sess.header.pack_end(&disc_all, false, false, 0);
    sess.header.pack_end(&stop_ai, false, false, 0);
    let c = ctx.clone();
    stop_ai.connect_clicked(move |_| {
        let c = c.clone();
        spawn(async move {
            if c.call(Request::StopAi).await.is_some() {
                c.toast(tr!("已停止所有 AI 操作", "All AI activity stopped"));
            }
        });
    });
    let c = ctx.clone();
    disc_all.connect_clicked(move |_| {
        let c = c.clone();
        spawn(async move {
            let _ = c.call(Request::DisconnectAll).await;
        });
    });
    content.pack_start(&sess.root, false, false, 0);

    let list = sess.list.clone();
    let c = ctx.clone();
    let last_sessions = Rc::new(std::cell::RefCell::new(Vec::<Session>::new()));
    let last_operate = Rc::new(Cell::new(None::<bool>));
    ctx.on_status(move |st| {
        name.set_text(&st.hostname);
        id.set_text(&group_id(&st.device_id));
        let (rgb, text) = state_of(st);
        color.set(rgb);
        d.queue_draw();
        state.set_text(&text);
        no_account.set_visible(st.account_count == 0 && st.rights.can_manage());
        relogin.set_visible(st.needs_relogin > 0);
        toggle.set(st.remote_enabled);
        // 禁止远程：屏幕前的用户也能关；打开要管理员
        toggle.switch.set_sensitive(st.rights.can_manage() || (st.rights.operate && st.remote_enabled));
        server.set_text(if st.server.is_empty() { tr!("未设置", "Not set") } else { st.server.as_str() });
        ip.set_text(if st.public_ip.is_empty() { "—" } else { st.public_ip.as_str() });
        accn.set_text(&st.account_count.to_string());
        ver.set_text(&st.version);
        let any_ai = st.sessions.iter().any(|s| s.ai_active);
        stop_ai.set_visible(any_ai);
        stop_ai.set_sensitive(st.rights.operate);
        disc_all.set_visible(st.sessions.len() > 1);
        disc_all.set_sensitive(st.rights.operate);
        // 会话列表没变就不重建（状态推得很勤）
        if *last_sessions.borrow() == st.sessions && last_operate.get() == Some(st.rights.operate) {
            return;
        }
        *last_sessions.borrow_mut() = st.sessions.clone();
        last_operate.set(Some(st.rights.operate));
        clear(&list);
        if st.sessions.is_empty() {
            let r = gtk::ListBoxRow::new();
            r.set_activatable(false);
            let l = label(tr!("现在没有人连接这台电脑", "Nobody is connected to this computer"), &["dim-label", "lg-row"]);
            l.set_margin_top(6);
            l.set_margin_bottom(6);
            r.add(&l);
            list.add(&r);
        }
        for s in &st.sessions {
            list.add(&session_row(&c, s, st.rights.operate));
        }
        list.show_all();
    });
    scroll.upcast()
}

fn state_of(st: &Status) -> ((f64, f64, f64), String) {
    const GREEN: (f64, f64, f64) = (0.18, 0.76, 0.49);
    const AMBER: (f64, f64, f64) = (0.90, 0.65, 0.04);
    const ORANGE: (f64, f64, f64) = (0.90, 0.38, 0.0);
    const GRAY: (f64, f64, f64) = (0.60, 0.60, 0.59);
    const RED: (f64, f64, f64) = (0.88, 0.11, 0.14);
    if !st.remote_enabled {
        return (RED, tr!("已禁止远程", "Remote access disabled").into());
    }
    if st.sessions.iter().any(|s| s.ai_active) {
        return (ORANGE, tr!("AI 正在操作", "AI is operating this computer").into());
    }
    if !st.sessions.is_empty() {
        return (ORANGE, tr!("正在被远程控制", "Being controlled remotely").into());
    }
    if st.account_count == 0 {
        return (GRAY, tr!("服务运行中，还没有登录账号", "Service running, no account signed in").into());
    }
    if st.online {
        (GREEN, tr!("在线，等待连接", "Online, waiting for connections").into())
    } else {
        (AMBER, tr!("正在连接服务器…", "Connecting to the server…").into())
    }
}

fn session_row(ctx: &Rc<Ctx>, s: &Session, operate: bool) -> gtk::ListBoxRow {
    let user = if s.from_user.is_empty() { tr!("未登录用户", "Anonymous").to_string() } else { s.from_user.clone() };
    let r = row(&user, None, None);
    let icon = gtk::Image::from_icon_name(Some(if s.ai_active { "applications-science-symbolic" } else { "avatar-default-symbolic" }), gtk::IconSize::LargeToolbar);
    icon.set_pixel_size(24);
    r.use_prefix(&icon);
    let dev: String = s.from_device.chars().take(8).collect();
    let mode = if s.assist { tr!("临时协助", "Assistance") } else { tr!("远程控制", "Remote control") };
    r.set_subtitle(&format!("{mode} · {} {dev} · {} · {}", tr!("设备", "device"), path_text(s.path), since(s.started_at)));
    if s.ai_active {
        let b = label("AI", &["lg-badge", "lg-badge-ai"]);
        b.set_valign(gtk::Align::Center);
        r.suffix.pack_start(&b, false, false, 0);
    }
    let btn = gtk::Button::with_label(tr!("断开", "Disconnect"));
    btn.set_sensitive(operate);
    let (c, id) = (ctx.clone(), s.id.clone());
    btn.connect_clicked(move |b| {
        b.set_sensitive(false);
        let (c, id, b) = (c.clone(), id.clone(), b.clone());
        spawn(async move {
            if c.call(Request::Disconnect { session_id: id }).await.is_none() {
                b.set_sensitive(true);
            }
        });
    });
    r.suffix.pack_start(&btn, false, false, 0);
    r.row
}

fn since(ms: i64) -> String {
    let t = fmt_time(ms, false);
    if crate::i18n::zh() {
        format!("{t} 起")
    } else {
        format!("since {t}")
    }
}
