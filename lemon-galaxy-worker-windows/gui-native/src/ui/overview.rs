//! 概览：设备名、系统、设备 ID（可复制）、状态；提示条（没账号、要重新登录、这份比已装的新、前台运行）；
//! 远程访问（允许远程、服务器、公网 IP、账号数、服务版本）；正在控制本机的会话（逐个断开、全部断开、停止 AI）。

use super::*;
use crate::setup::{installed_version, running_installed};
use crate::win::layout::{Panel, Right};

struct SessionRow {
    session: Session,
    name: Ctl,
    sub: Ctl,
    button: Ctl,
}

pub fn build(ctx: &Rc<Ctx>, parent: HWND) -> PageView {
    let panel = Panel::new(parent, true);
    let p = panel.hwnd;
    let title = label_font(p, tr!("概览", "Overview"), Font::Title);

    // 抬头
    let host = label_font(p, "", Font::Hero);
    let state = label_tone(p, "", Tone::Dim);
    let os = label_tone(p, &crate::setup::os_name(), Tone::Dim);
    let id_label = label_tone(p, tr!("设备 ID", "Device ID"), Tone::Dim);
    let id_info = info(p, tr!("主控端设备列表里认这台电脑的编号。", "How controllers identify this computer in their device list."));
    let id_value = value(p, "");
    id_value.set_font(Font::Mono);
    let c = ctx.clone();
    let id_copy = copy_button(ctx, p, move || c.status.borrow().as_ref().map(|s| s.device_id.clone()).unwrap_or_default());

    // 提示条
    let no_account = label(p, tr!("还没有登录账号。登录后，这台电脑会出现在账号的设备列表里。", "No account is signed in. Sign in so this computer shows up in your device list."));
    let no_account_btn = button(p, tr!("登录账号", "Sign In"));
    let relogin = label(p, tr!("有账号被管理员重置，需要重新登录后才能被它控制。", "An account was reset by an administrator and must sign in again."));
    let relogin_btn = button(p, tr!("查看账号", "View Accounts"));
    let newer = label(p, "");
    let newer_btn = button(p, "");
    newer_btn.set_shield(true);
    let foreground = label(p, "");

    // 远程访问
    let remote_h = label_font(p, tr!("远程访问", "Remote Access"), Font::Bold);
    let allow = checkbox(p, tr!("允许远程控制", "Allow remote control"));
    let allow_info = info(p, tr!("关闭后，所有账号都连不进来，正在进行的连接会立即断开。屏幕前的用户可以直接关闭；重新允许需要管理员授权。", "When off, no account can connect and active sessions end immediately. The person at this computer can turn it off; turning it back on needs administrator authorization."));
    let server_l = label(p, tr!("服务器", "Server"));
    let server_info = info(p, tr!("完整地址（含证书指纹）在「账号与权限」页，可以复制。", "The full address with the certificate fingerprint is on the Accounts page."));
    let server_v = value(p, "");
    let ip_l = label(p, tr!("公网 IP", "Public IP"));
    let ip_v = value(p, "");
    let acc_l = label(p, tr!("已登录账号", "Signed-in accounts"));
    let acc_v = value(p, "");
    let ver_l = label(p, tr!("被控服务版本", "Service version"));
    let ver_v = value(p, "");

    // 当前连接
    let sess_h = label_font(p, tr!("当前连接", "Active Sessions"), Font::Bold);
    let stop_ai = button(p, tr!("停止 AI 操作", "Stop AI"));
    let disc_all = button(p, tr!("全部断开", "Disconnect All"));
    let none = label_tone(p, tr!("现在没有人连接这台电脑", "Nobody is connected to this computer"), Tone::Dim);
    let rows: Rc<RefCell<Vec<SessionRow>>> = Rc::new(RefCell::new(vec![]));

    // 动作
    let c = ctx.clone();
    on_click(no_account_btn, move || super::login::open(&c, None));
    let c = ctx.clone();
    on_click(relogin_btn, move || c.goto(Page::Accounts));
    let c = ctx.clone();
    on_click(newer_btn, move || {
        let c2 = c.clone();
        c.run_elevated("install", vec!["--install".into()], move |ok| {
            if ok {
                super::setup_page::launch_installed_and_exit(&c2);
            }
        });
    });
    req_toggle(ctx, allow, |on| Request::SetRemoteEnabled { enabled: on });
    let c = ctx.clone();
    on_click(stop_ai, move || {
        let c2 = c.clone();
        c.act(Request::StopAi, move |r| {
            if r.is_some() {
                c2.toast(tr!("已停止所有 AI 操作", "All AI activity stopped"));
            }
        });
    });
    let c = ctx.clone();
    on_click(disc_all, move || c.act(Request::DisconnectAll, |_| {}));

    // 「这份比已装的新」：只在启动时读一次已装的版本（装好后会换成装好的那份运行）
    let newer_state: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));

    let c = ctx.clone();
    let rows2 = rows.clone();
    let ns = newer_state.clone();
    panel.set_layout(move |f| {
        let st = c.status.borrow();
        let Some(s) = st.as_ref() else { return };
        f.title(title, None, &[]);
        f.gap(18);

        // 抬头：标志、名字、状态、系统、设备 ID
        let icon = px(48);
        f.app_icon(f.x, f.y, icon);
        let (x0, w0) = (f.x, f.w);
        f.x += icon + px(16);
        f.w -= icon + px(16);
        let top = f.y;
        f.text(host);
        f.gap(2);
        let dot = px(8);
        let (sw, sh) = measure(state.get_font(), &state.text(), f.w - dot - px(8), true);
        f.dot(f.x, f.y + (line_h() - dot) / 2, dot, overall_color(overall(&c)));
        f.place(state, f.x + dot + px(8), f.y, sw, sh);
        f.y += sh;
        f.gap(2);
        f.text(os);
        f.gap(8);
        let (lw, lh) = pref(id_label);
        let (vw, vh) = pref(id_value);
        let (bw, bh) = pref(id_copy);
        let rh = lh.max(vh).max(bh);
        let mut x = f.x;
        f.place(id_label, x, f.y + (rh - lh) / 2, lw, lh);
        x += lw + px(4);
        f.place(id_info, x, f.y + (rh - px(16)) / 2, px(16), px(16));
        x += px(16 + 10);
        let vw = vw.min(f.x + f.w - x - bw - px(8));
        f.place(id_value, x, f.y + (rh - vh) / 2, vw, vh);
        x += vw + px(8);
        f.place(id_copy, x, f.y + (rh - bh) / 2, bw, bh);
        f.y += rh;
        f.y = f.y.max(top + icon);
        f.x = x0;
        f.w = w0;

        // 提示条
        let r = &s.rights;
        let mut banners = vec![];
        if s.account_count == 0 && r.can_manage() {
            banners.push((Tone::Accent, no_account, Some(no_account_btn)));
        }
        if s.needs_relogin > 0 {
            banners.push((Tone::Warning, relogin, Some(relogin_btn)));
        }
        if ns.get() == Some(true) {
            banners.push((Tone::Accent, newer, Some(newer_btn)));
        }
        if s.service.mode == ServiceMode::Foreground {
            banners.push((Tone::Accent, foreground, None));
        }
        for (tone, text, btn) in banners {
            f.gap(12);
            f.banner(tone, text, btn);
        }

        f.gap(22);
        f.title(remote_h, None, &[]);
        f.gap(8);
        f.group(|g| {
            g.row(allow, Some(allow_info), None, &[]);
            g.row(server_l, Some(server_info), None, &[Right::C(server_v)]);
            g.row(ip_l, None, None, &[Right::C(ip_v)]);
            g.row(acc_l, None, None, &[Right::C(acc_v)]);
            g.row(ver_l, None, None, &[Right::C(ver_v)]);
        });

        f.gap(22);
        let any_ai = s.sessions.iter().any(|x| x.ai_active);
        let mut head_right = vec![];
        if any_ai {
            head_right.push(stop_ai);
        }
        if s.sessions.len() > 1 {
            head_right.push(disc_all);
        }
        f.title(sess_h, None, &head_right);
        f.gap(8);
        let rows = rows2.borrow();
        f.group(|g| {
            if rows.is_empty() {
                g.note(none);
            }
            for row in rows.iter() {
                let mut right = vec![];
                if row.session.ai_active {
                    right.push(Right::Badge("AI", Tone::Warning));
                }
                right.push(Right::C(row.button));
                g.row(row.name, None, Some(row.sub), &right);
            }
        });
    });

    let c = ctx.clone();
    let pw = Rc::downgrade(&panel);
    ctx.on_status(move |s| {
        host.set_text(&s.hostname);
        state.set_text(overall_text(overall(&c)));
        id_value.set_text(&group_id(&s.device_id));
        let r = &s.rights;
        allow.set_checked(s.remote_enabled);
        // 禁止远程：屏幕前的用户也能关；打开要管理员
        allow.enable(r.can_manage() || (r.operate && s.remote_enabled));
        allow.set_shield(!s.remote_enabled && r.manage == ManageRight::Elevate);
        no_account_btn.set_shield(r.manage == ManageRight::Elevate);
        server_v.set_text(if s.server.is_empty() { tr!("未设置", "Not set") } else { server_host(&s.server) });
        ip_v.set_text(if s.public_ip.is_empty() { "—" } else { &s.public_ip });
        acc_v.set_text(&s.account_count.to_string());
        ver_v.set_text(&s.version);
        stop_ai.enable(r.operate);
        disc_all.enable(r.operate);
        foreground.set_text(&crate::i18n::fmt(tr!("被控服务正以 {0} 的身份在前台运行（命令行 run），关掉那个窗口服务就停了。", "The service runs in the foreground as {0} (command line run); closing that window stops it."), &[&s.service.user]));
        if newer_state.get().is_none() {
            let inst = installed_version();
            let me = crate::setup::app_version();
            let show = !running_installed() && inst.as_deref() != Some(me);
            newer_state.set(Some(show));
            match inst {
                None => {
                    newer.set_text(tr!("这份程序还没有装到本机：装好后开始菜单里有入口，登录后托盘自动出现。", "This copy is not installed: install it for a Start menu entry and a tray icon at sign-in."));
                    newer_btn.set_text(tr!("安装", "Install"));
                }
                Some(v) => {
                    newer.set_text(&crate::i18n::fmt(tr!("这份是 {0}，本机装的是 {1}。", "This copy is {0}; {1} is installed."), &[me, &v]));
                    newer_btn.set_text(tr!("更新", "Update"));
                }
            }
        }
        sync_sessions(&c, p, &rows, s);
        if let Some(p) = pw.upgrade() {
            p.relayout_soon();
        }
    });
    PageView { panel, refresh: None }
}

/// 会话列表没变就不重建（状态推得很勤）
fn sync_sessions(ctx: &Rc<Ctx>, p: HWND, rows: &Rc<RefCell<Vec<SessionRow>>>, s: &Status) {
    let same = {
        let cur = rows.borrow();
        cur.len() == s.sessions.len() && cur.iter().zip(&s.sessions).all(|(a, b)| a.session == *b)
    };
    let operate = s.rights.operate;
    if same {
        for r in rows.borrow().iter() {
            r.button.enable(operate);
        }
        return;
    }
    for r in rows.borrow_mut().drain(..) {
        r.name.destroy();
        r.sub.destroy();
        r.button.destroy();
    }
    let mut v = vec![];
    for x in &s.sessions {
        let mode = if x.assist { tr!("临时协助", "Assistance") } else { tr!("远程控制", "Remote control") };
        let sub = format!("{mode} · {} {} · {} · {}", tr!("设备", "device"), short_device(&x.from_device), path_text(x.path), since(x.started_at));
        let name = label(p, &who(&x.from_user));
        let subl = label_tone(p, &sub, Tone::Dim);
        let b = button(p, tr!("断开", "Disconnect"));
        b.enable(operate);
        let (c, id) = (ctx.clone(), x.id.clone());
        on_click(b, move || {
            b.enable(false);
            c.act(Request::Disconnect { session_id: id.clone() }, move |r| {
                if r.is_none() {
                    b.enable(true);
                }
            });
        });
        v.push(SessionRow { session: x.clone(), name, sub: subl, button: b });
    }
    *rows.borrow_mut() = v;
}
