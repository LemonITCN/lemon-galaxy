//! 临时协助：开始 / 结束，显示设备码与验证码，固定验证码；请求来了弹「允许 / 拒绝」（见 `ask`）。

use super::*;
use crate::control::*;
use crate::tr;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

fn code_block(ctx: &Rc<Ctx>, title: &str) -> (gtk::Box, gtk::Label) {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
    b.set_hexpand(true);
    let t = label(title, &["dim-label"]);
    t.set_xalign(0.5);
    b.pack_start(&t, false, false, 0);
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    h.set_halign(gtk::Align::Center);
    let v = label("", &["lg-code"]);
    v.set_selectable(true);
    h.pack_start(&v, false, false, 0);
    let v2 = v.clone();
    h.pack_start(&copy_button(ctx, move || v2.text().replace(' ', "")), false, false, 0);
    b.pack_start(&h, false, false, 0);
    (b, v)
}

pub fn build(ctx: &Rc<Ctx>) -> (gtk::Widget, Rc<dyn Fn()>) {
    let (scroll, content) = page();
    let intro = wrap_label(
        tr!(
            "需要别人帮忙时开启：把设备码和验证码告诉协助你的人，对方在自己的 LemonGalaxy 里输入，你点「允许」后才会连入。不需要登录账号，关掉这个窗口或结束协助后对方就连不进来了。",
            "Turn this on when someone needs to help you. Tell them the device code and verification code; they enter both in their LemonGalaxy app, and they can only connect after you click Allow. No account is needed. Closing this window or ending assistance stops it."
        ),
        &["dim-label"],
    );
    content.pack_start(&intro, false, false, 0);

    let stack = gtk::Stack::new();
    stack.set_vhomogeneous(false);
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);

    // 未开启
    let idle = gtk::Box::new(gtk::Orientation::Vertical, 16);
    let sec = section("", None);
    let server_row = row(
        tr!("服务器", "Server"),
        None,
        Some(tr!(
            "还没有登录账号时，临时协助经这台服务器配对。地址在管理后台「服务器信息」页复制，格式为 主机:端口#sha256-指纹。",
            "Without a signed-in account, assistance is paired through this server. Copy the address from the Server Info page of the admin console (host:port#sha256-fingerprint)."
        )),
    );
    let server = gtk::Entry::new();
    server.set_width_chars(30);
    server.set_placeholder_text(Some(tr!("主机:端口#sha256-指纹", "host:port#sha256-fingerprint")));
    server_row.suffix.pack_start(&server, false, false, 0);
    sec.list.add(&server_row.row);
    idle.pack_start(&sec.root, false, false, 0);
    let start = gtk::Button::with_label(tr!("开启临时协助", "Start Assistance"));
    start.style_context().add_class("suggested-action");
    start.set_halign(gtk::Align::Start);
    let sp = gtk::Spinner::new();
    let sb = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    sb.pack_start(&start, false, false, 0);
    sb.pack_start(&sp, false, false, 0);
    idle.pack_start(&sb, false, false, 0);
    let denied = wrap_label(tr!("只有 root 或当前坐在这台电脑前的用户能开启临时协助。", "Only root or the user signed in at this computer can start assistance."), &["dim-label"]);
    denied.set_no_show_all(true);
    idle.pack_start(&denied, false, false, 0);
    stack.add_named(&idle, "idle");

    // 进行中（本窗口开的）
    let active = gtk::Box::new(gtk::Orientation::Vertical, 18);
    let frame = gtk::Frame::new(None);
    let codes = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    codes.set_margin_top(22);
    codes.set_margin_bottom(22);
    codes.set_margin_start(16);
    codes.set_margin_end(16);
    codes.set_homogeneous(true);
    let (b1, device_code) = code_block(ctx, tr!("设备码", "Device code"));
    let (b2, code) = code_block(ctx, tr!("验证码", "Verification code"));
    codes.pack_start(&b1, true, true, 0);
    codes.pack_start(&b2, true, true, 0);
    frame.add(&codes);
    active.pack_start(&frame, false, false, 0);
    let waiting = wrap_label(tr!("等待对方连接。有人请求时会弹窗让你确认，2 分钟内不处理按拒绝。", "Waiting for the helper. You will be asked to confirm each request; unanswered requests are declined after 2 minutes."), &["dim-label"]);
    active.pack_start(&waiting, false, false, 0);
    let stop = gtk::Button::with_label(tr!("结束协助", "End Assistance"));
    stop.style_context().add_class("destructive-action");
    stop.set_halign(gtk::Align::Start);
    active.pack_start(&stop, false, false, 0);
    stack.add_named(&active, "active");

    // 进行中（别的窗口开的）
    let other = gtk::Box::new(gtk::Orientation::Vertical, 12);
    other.pack_start(&wrap_label(tr!("临时协助已经在另一个窗口里开启，设备码和验证码只在那里显示。", "Assistance was started in another window; the codes are shown there."), &[]), false, false, 0);
    let stop2 = gtk::Button::with_label(tr!("结束协助", "End Assistance"));
    stop2.style_context().add_class("destructive-action");
    stop2.set_halign(gtk::Align::Start);
    other.pack_start(&stop2, false, false, 0);
    stack.add_named(&other, "other");
    content.pack_start(&stack, false, false, 0);

    // 固定验证码
    let fixed_sec = section(tr!("验证码", "Verification Code"), None);
    let fr = row(
        tr!("使用固定验证码", "Use a fixed code"),
        None,
        Some(tr!(
            "默认每次开启都随机生成验证码。固定后每次都用同一个，适合常帮你的人。8 到 16 位字母或数字。",
            "By default a new random code is generated each time. A fixed code stays the same, which suits people who help you often. 8 to 16 letters or digits."
        )),
    );
    let fixed = Toggle::new();
    fr.suffix.pack_start(&fixed.switch, false, false, 0);
    fixed_sec.list.add(&fr.row);
    let er = row(tr!("固定验证码", "Fixed code"), None, None);
    let fixed_entry = gtk::Entry::new();
    fixed_entry.set_max_length(16);
    fixed_entry.set_width_chars(18);
    fixed_entry.style_context().add_class("lg-mono");
    let save = gtk::Button::with_label(tr!("保存", "Save"));
    er.suffix.pack_start(&fixed_entry, false, false, 0);
    er.suffix.pack_start(&save, false, false, 0);
    fixed_sec.list.add(&er.row);
    // 先把里面都显示出来再设 no_show_all（GTK 的 show_all 遇到 no_show_all 的控件直接跳过）
    fixed_sec.root.show_all();
    fixed_sec.root.set_no_show_all(true);
    content.pack_start(&fixed_sec.root, false, false, 0);

    // 动作
    let c = ctx.clone();
    let (sp2, server2) = (sp.clone(), server.clone());
    start.connect_clicked(move |b| {
        let (c, b, sp, server) = (c.clone(), b.clone(), sp2.clone(), server2.clone());
        b.set_sensitive(false);
        sp.start();
        spawn(async move {
            let s = server.text().trim().to_string();
            let _ = c.call(Request::AssistStart { server: (!s.is_empty()).then_some(s) }).await;
            sp.stop();
            b.set_sensitive(true);
        });
    });
    for b in [&stop, &stop2] {
        let c = ctx.clone();
        b.connect_clicked(move |_| {
            let c = c.clone();
            spawn(async move {
                let _ = c.call(Request::AssistStop).await;
            });
        });
    }
    let c = ctx.clone();
    let (fe, er_row) = (fixed_entry.clone(), er.row.clone());
    fixed.on_toggle(move |on| {
        let (c, fe, er_row) = (c.clone(), fe.clone(), er_row.clone());
        async move {
            if on {
                // 打开时先显示输入框，填好点保存才生效
                er_row.set_visible(true);
                fe.grab_focus();
                return true;
            }
            let ok = c.call(Request::SetAssistCode { code: None }).await.is_some();
            if ok {
                fe.set_text("");
                er_row.set_visible(false);
            }
            ok
        }
    });
    let c = ctx.clone();
    let fe = fixed_entry.clone();
    save.connect_clicked(move |_| {
        let (c, fe) = (c.clone(), fe.clone());
        spawn(async move {
            let code = fe.text().trim().to_string();
            if c.call(Request::SetAssistCode { code: Some(code) }).await.is_some() {
                c.toast(tr!("已保存固定验证码", "Fixed code saved"));
            }
        });
    });

    // 状态
    let (stack2, start2, denied2) = (stack.clone(), start.clone(), denied.clone());
    let server_sec = sec.root.clone();
    server_sec.show_all();
    server_sec.set_no_show_all(true);
    let server3 = server.clone();
    ctx.on_status(move |st| {
        match &st.assist {
            Some(a) if a.mine => {
                device_code.set_text(&a.device_code);
                code.set_text(&a.code);
                stack2.set_visible_child_name("active");
            }
            Some(_) => {
                stop2.set_sensitive(st.rights.operate);
                stack2.set_visible_child_name("other");
            }
            None => stack2.set_visible_child_name("idle"),
        }
        start2.set_sensitive(st.rights.operate);
        denied2.set_visible(!st.rights.operate);
        // 已登录服务器时不用填
        server_sec.set_visible(st.server.is_empty());
        if !st.server.is_empty() {
            server3.set_text("");
        }
    });

    let c = ctx.clone();
    let fixed_root = fixed_sec.root.clone();
    let er_row = er.row.clone();
    let fixed2 = fixed.clone();
    let fixed_entry2 = fixed_entry.clone();
    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        let Some(r) = c.rights() else { return };
        fixed_root.set_visible(r.view_config);
        if !r.view_config {
            return;
        }
        fixed_root.set_sensitive(r.can_manage());
        let (c, er_row, fixed, fe) = (c.clone(), er_row.clone(), fixed2.clone(), fixed_entry2.clone());
        spawn(async move {
            if let Response::Settings { settings } = c.client.call(Request::Settings).await {
                fixed.set(settings.assist_code.is_some());
                fe.set_text(settings.assist_code.as_deref().unwrap_or(""));
                er_row.set_visible(settings.assist_code.is_some());
            }
        });
    });
    // 配置被改过（界面或命令行）、权限变了：重新读
    let sig = Rc::new(std::cell::Cell::new(None::<(u64, bool, bool)>));
    let r2 = refresh.clone();
    ctx.on_status(move |st| {
        let s = (st.config_rev, st.rights.view_config, st.rights.can_manage());
        if sig.replace(Some(s)) != Some(s) {
            r2();
        }
    });
    (scroll.upcast(), refresh)
}

/// 协助请求对话框（按请求号，对方取消或超时时关掉）
#[derive(Default)]
pub struct Asks {
    open: RefCell<HashMap<u64, gtk::MessageDialog>>,
}

impl Asks {
    pub fn ask(self: &Rc<Self>, ctx: &Rc<Ctx>, request_id: u64, from_user: &str, from_device: &str) {
        let who = if from_user.is_empty() { tr!("对方", "Someone").to_string() } else { from_user.to_string() };
        let dev: String = from_device.chars().take(8).collect();
        let d = gtk::MessageDialog::new(ctx.parent().as_ref(), gtk::DialogFlags::empty(), gtk::MessageType::Question, gtk::ButtonsType::None, tr!("有人请求控制这台电脑", "Someone wants to control this computer"));
        d.set_secondary_text(Some(&if crate::i18n::zh() {
            format!("{who} 从设备 {dev} 发起了协助请求。允许后对方可以查看屏幕、操作键鼠，并按屏幕前用户的权限使用终端和文件。随时可以在概览或托盘里断开。")
        } else {
            format!("{who} on device {dev} asked to help. If you allow it, they can see the screen, use the keyboard and mouse, and use the terminal and files with the permissions of the person at this computer. You can disconnect any time from Overview or the tray.")
        }));
        d.add_button(tr!("拒绝", "Deny"), gtk::ResponseType::Reject);
        let allow = d.add_button(tr!("允许", "Allow"), gtk::ResponseType::Accept);
        allow.style_context().add_class("suggested-action");
        d.set_keep_above(true);
        d.set_urgency_hint(true);
        d.set_title(tr!("协助请求", "Assistance Request"));
        let (c, me) = (ctx.clone(), self.clone());
        d.connect_response(move |d, r| {
            // 已被服务关掉（对方取消、超时）的不用再回
            let pending = me.open.borrow_mut().remove(&request_id).is_some();
            if pending {
                let allow = r == gtk::ResponseType::Accept;
                let c = c.clone();
                spawn(async move {
                    let _ = c.call(Request::AssistAnswer { request_id, allow }).await;
                });
            }
            d.close();
        });
        self.open.borrow_mut().insert(request_id, d.clone());
        d.present();
    }

    pub fn closed(&self, request_id: u64) {
        let d = self.open.borrow_mut().remove(&request_id);
        if let Some(d) = d {
            d.close();
        }
    }

    pub fn close_all(&self) {
        let all: Vec<_> = self.open.borrow_mut().drain().map(|(_, d)| d).collect();
        for d in all {
            d.close();
        }
    }
}
