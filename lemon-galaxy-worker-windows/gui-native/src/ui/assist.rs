//! 临时协助：开始 / 结束，设备码与验证码（可复制），固定验证码；有人请求时弹置顶的小窗口「允许 / 拒绝」（`Asks`），
//! 主窗口藏在托盘里也看得到；对方取消或超时自动关掉。

use super::*;
use crate::win::dialog::{Dialog, IDCANCEL_, IDOK_};
use crate::win::layout::{Align, Decor, Panel, Right};
use std::collections::HashMap;
use windows::Win32::Foundation::RECT;

pub fn build(ctx: &Rc<Ctx>, parent: HWND) -> PageView {
    let panel = Panel::new(parent, true);
    let p = panel.hwnd;
    let title = label_font(p, tr!("临时协助", "Assistance"), Font::Title);
    let title_info = info(
        p,
        tr!(
            "需要别人帮忙时开启：把设备码和验证码告诉协助你的人，对方在自己的 LemonGalaxy 里输入，你点「允许」后才会连入。不需要登录账号；结束协助或退出这个界面后对方就连不进来了。",
            "Turn this on when someone needs to help you. Tell them the device code and verification code; they enter both in their LemonGalaxy app, and they can only connect after you click Allow. No account is needed. Ending assistance or quitting this app stops it."
        ),
    );

    // 未开启
    let server_l = label(p, tr!("服务器", "Server"));
    let server_info = info(p, tr!("还没有登录账号时，临时协助经这台服务器配对。地址在管理后台「服务器信息」页复制，格式为 主机:端口#sha256-指纹。", "Without a signed-in account, assistance is paired through this server. Copy the address from the Server Info page of the admin console (host:port#sha256-fingerprint)."));
    let server = edit(p, "", false);
    server.set_font(Font::Mono);
    server.set_cue(tr!("主机:端口#sha256-指纹", "host:port#sha256-fingerprint"));
    set_width(server, px(300));
    let start = button(p, tr!("开启临时协助", "Start Assistance"));
    let starting = label_tone(p, tr!("正在连接服务器…", "Connecting to the server…"), Tone::Dim);
    let starting_bar = marquee(p);
    let denied = label(p, tr!("只有当前坐在这台电脑前的用户或管理员能开启临时协助。", "Only the person at this computer or an administrator can start assistance."));

    // 进行中（本窗口开的）
    let dev_cap = center_label(p, tr!("设备码", "Device code"), Font::Normal, Tone::Dim);
    let code_cap = center_label(p, tr!("验证码", "Verification code"), Font::Normal, Tone::Dim);
    let dev_v = value(p, "");
    dev_v.set_font(Font::Code);
    let code_v = value(p, "");
    code_v.set_font(Font::Code);
    let c = ctx.clone();
    let dev_copy = copy_button(ctx, p, move || c.status.borrow().as_ref().and_then(|s| s.assist.as_ref().map(|a| a.device_code.replace(' ', ""))).unwrap_or_default());
    let c = ctx.clone();
    let code_copy = copy_button(ctx, p, move || c.status.borrow().as_ref().and_then(|s| s.assist.as_ref().map(|a| a.code.clone())).unwrap_or_default());
    let waiting_bar = marquee(p);
    set_width(waiting_bar, px(60));
    let waiting = label_tone(p, tr!("等待对方连接", "Waiting for the helper"), Tone::Dim);
    let waiting_info = info(p, tr!("有人请求时会弹窗让你确认，2 分钟内不处理按拒绝。", "You will be asked to confirm each request; unanswered requests are declined after 2 minutes."));
    let stop = button(p, tr!("结束临时协助", "End Assistance"));

    // 进行中（别的窗口开的）
    let other = label(p, tr!("临时协助已经在另一个窗口里开启，设备码和验证码只在那里显示。", "Assistance was started in another window; the codes are shown there."));
    let stop2 = button(p, tr!("结束临时协助", "End Assistance"));

    // 固定验证码
    let fixed_h = label_font(p, tr!("验证码", "Verification Code"), Font::Bold);
    let fixed = checkbox(p, tr!("使用固定验证码", "Use a fixed code"));
    let fixed_info = info(p, tr!("默认每次开启都随机生成验证码。固定后每次都用同一个，适合常帮你的人。8 到 16 位字母或数字。", "By default a new random code is generated each time. A fixed code stays the same, which suits people who help you often. 8 to 16 letters or digits."));
    let fixed_l = label(p, tr!("固定验证码", "Fixed code"));
    let fixed_e = edit(p, "", false);
    fixed_e.set_font(Font::Mono);
    fixed_e.set_limit(16);
    set_width(fixed_e, px(200));
    let fixed_save = button(p, tr!("保存", "Save"));

    // 读到的设置：None = 还没读 / 没权限
    let code_setting: Rc<RefCell<Option<Option<String>>>> = Rc::new(RefCell::new(None));
    let editing = Rc::new(Cell::new(false));

    let c = ctx.clone();
    let cs = code_setting.clone();
    let ed = editing.clone();
    panel.set_layout(move |f| {
        let st = c.status.borrow();
        let Some(s) = st.as_ref() else { return };
        f.title(title, Some(title_info), &[]);
        f.gap(18);
        match &s.assist {
            Some(a) if a.mine => {
                // 两个码并排，中间一条竖线
                let top = f.y;
                let pad = px(20);
                let col = (f.w - 1) / 2;
                let mut y_end = top;
                for (i, (cap, v, b)) in [(dev_cap, dev_v, dev_copy), (code_cap, code_v, code_copy)].into_iter().enumerate() {
                    let x = f.x + i as i32 * (col + 1);
                    let (_, ch) = measure(cap.get_font(), &cap.text(), col, false);
                    f.place(cap, x, top + pad, col, ch);
                    let (vw, vh) = pref(v);
                    let (bw, bh) = pref(b);
                    let total = vw + px(10) + bw;
                    let vx = x + (col - total).max(0) / 2;
                    let ry = top + pad + ch + px(8);
                    let rh = vh.max(bh);
                    f.place(v, vx, ry + (rh - vh) / 2, vw.min(col - bw - px(10)), vh);
                    f.place(b, vx + vw.min(col - bw - px(10)) + px(10), ry + (rh - bh) / 2, bw, bh);
                    y_end = y_end.max(ry + rh + pad);
                }
                f.decor(Decor::Fill(RECT { left: f.x + col, top: top + px(12), right: f.x + col + 1, bottom: y_end - px(12) }, line_color()));
                f.decor(Decor::Frame(RECT { left: f.x, top, right: f.x + f.w, bottom: y_end }));
                f.y = y_end;
                f.gap(14);
                let (sw, sh) = pref(stop);
                let row_h = sh;
                let y = f.y;
                let (mw, mh) = pref(waiting_bar);
                f.place(waiting_bar, f.x, y + (row_h - mh) / 2, mw, mh);
                let (ww, wh) = pref(waiting);
                f.place(waiting, f.x + mw + px(10), y + (row_h - wh) / 2, ww, wh);
                f.place(waiting_info, f.x + mw + px(10) + ww + px(6), y + (row_h - px(16)) / 2, px(16), px(16));
                f.place(stop, f.x + f.w - sw, y, sw, sh);
                f.y += row_h;
            }
            Some(_) => {
                f.banner(Tone::Accent, other, Some(stop2));
            }
            None => {
                if s.server.is_empty() {
                    f.group(|g| g.row(server_l, Some(server_info), None, &[Right::C(server)]));
                    f.gap(14);
                }
                if c.busy.get() == Some("assist") {
                    f.line(&[start, starting_bar, starting], Align::Left);
                } else {
                    f.line(&[start], Align::Left);
                }
                if !s.rights.operate {
                    f.gap(12);
                    f.banner(Tone::Normal, denied, None);
                }
            }
        }
        // 固定验证码（能看设置的人才显示）
        if s.rights.view_config {
            if let Some(cur) = cs.borrow().as_ref() {
                f.gap(24);
                f.title(fixed_h, None, &[]);
                f.gap(8);
                let on = cur.is_some() || ed.get();
                f.group(|g| {
                    g.row(fixed, Some(fixed_info), None, &[]);
                    if on {
                        g.row(fixed_l, None, None, &[Right::C(fixed_e), Right::C(fixed_save)]);
                    }
                });
            }
        }
    });

    // 动作
    let c = ctx.clone();
    on_click(start, move || {
        let s = server.text().trim().to_string();
        c.busy.set(Some("assist"));
        c.changed();
        let c2 = c.clone();
        c.act(Request::AssistStart { server: (!s.is_empty()).then_some(s) }, move |_| {
            c2.busy.set(None);
            c2.changed();
        });
    });
    for b in [stop, stop2] {
        let c = ctx.clone();
        on_click(b, move || c.act(Request::AssistStop, |_| {}));
    }
    let c = ctx.clone();
    let ed = editing.clone();
    let cs = code_setting.clone();
    let pw = Rc::downgrade(&panel);
    toggle(fixed, move |on, done| {
        if on {
            // 打开时先显示输入框，填好点保存才生效
            ed.set(true);
            done(true);
            if let Some(p) = pw.upgrade() {
                p.relayout();
            }
            fixed_e.focus();
            return;
        }
        let (ed, cs, pw) = (ed.clone(), cs.clone(), pw.clone());
        c.act(Request::SetAssistCode { code: None }, move |r| {
            let ok = r.is_some();
            if ok {
                ed.set(false);
                fixed_e.set_text("");
                *cs.borrow_mut() = Some(None);
                if let Some(p) = pw.upgrade() {
                    p.relayout();
                }
            }
            done(ok);
        });
    });
    let c = ctx.clone();
    on_click(fixed_save, move || {
        let code = fixed_e.text().trim().to_string();
        let c2 = c.clone();
        c.act(Request::SetAssistCode { code: Some(code) }, move |r| {
            if r.is_some() {
                c2.toast(tr!("已保存固定验证码", "Fixed code saved"));
            }
        });
    });

    // 读设置（固定验证码）
    let c = ctx.clone();
    let cs = code_setting.clone();
    let ed = editing.clone();
    let pw = Rc::downgrade(&panel);
    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        if !c.rights().view_config {
            *cs.borrow_mut() = None;
            return;
        }
        let (cs, ed, pw) = (cs.clone(), ed.clone(), pw.clone());
        c.client.call(Request::Settings, move |r| {
            if let Response::Settings { settings } = r {
                fixed.set_checked(settings.assist_code.is_some());
                fixed_e.set_text(settings.assist_code.as_deref().unwrap_or(""));
                ed.set(false);
                *cs.borrow_mut() = Some(settings.assist_code);
                if let Some(p) = pw.upgrade() {
                    p.relayout();
                }
            }
        });
    });

    // 状态：码、权限；配置被改过（界面或命令行）、权限变了就重新读设置
    let sig = Rc::new(Cell::new(None::<(u64, bool, bool)>));
    let r2 = refresh.clone();
    let pw = Rc::downgrade(&panel);
    ctx.on_status(move |s| {
        if let Some(a) = &s.assist {
            dev_v.set_text(&group_by(&a.device_code.replace(' ', ""), 3));
            code_v.set_text(&a.code);
        }
        start.enable(s.rights.operate);
        stop2.enable(s.rights.operate);
        let m = s.rights.can_manage();
        fixed.enable(m);
        fixed.set_shield(s.rights.manage == ManageRight::Elevate);
        fixed_e.enable(m);
        fixed_save.enable(m);
        fixed_save.set_shield(s.rights.manage == ManageRight::Elevate);
        if !s.server.is_empty() {
            server.set_text("");
        }
        let sg = (s.config_rev, s.rights.view_config, m);
        if sig.replace(Some(sg)) != Some(sg) {
            r2();
        }
        if let Some(p) = pw.upgrade() {
            p.relayout_soon();
        }
    });
    // 开协助时的忙碌状态
    let pw = Rc::downgrade(&panel);
    ctx.on_change(move || {
        if let Some(p) = pw.upgrade() {
            p.relayout_soon();
        }
    });
    PageView { panel, refresh: Some(refresh) }
}

/// 协助请求的小窗口（按请求号；对方取消或超时时关掉）
#[derive(Default)]
pub struct Asks {
    open: RefCell<HashMap<u64, Rc<Dialog>>>,
}

impl Asks {
    pub fn ask(self: &Rc<Self>, ctx: &Rc<Ctx>, request_id: u64, from_user: &str, from_device: &str) {
        if self.open.borrow().contains_key(&request_id) {
            return;
        }
        let d = Dialog::new(None, tr!("协助请求", "Assistance Request"), 380, false, true);
        let p = d.panel.hwnd;
        let who_ = if from_user.is_empty() { tr!("对方", "Someone").to_string() } else { from_user.to_string() };
        let head = label_font(p, tr!("有人请求控制这台电脑", "Someone wants to control this computer"), Font::Bold);
        let body = label(p, &crate::i18n::fmt(tr!("{0} 从设备 {1} 发起了协助请求。", "{0} on device {1} asked to help."), &[&who_, &short_device(from_device)]));
        let more = label_tone(
            p,
            tr!(
                "允许后对方可以查看屏幕、操作键鼠，并按屏幕前用户的权限使用终端和文件。随时可以在概览或托盘里断开。",
                "If you allow it, they can see the screen, use the keyboard and mouse, and use the terminal and files with the permissions of the person at this computer. You can disconnect any time from Overview or the tray."
            ),
            Tone::Dim,
        );
        let allow = button_id(p, tr!("允许", "Allow"), IDOK_, false);
        let deny = button_id(p, tr!("拒绝", "Deny"), IDCANCEL_, true);
        d.set_layout(move |f| {
            let s = px(32);
            f.sys_icon(f.x, f.y, s, 32515 /* IDI_WARNING */);
            let (x0, w0) = (f.x, f.w);
            f.x += s + px(14);
            f.w -= s + px(14);
            f.text(head);
            f.gap(6);
            f.text(body);
            f.gap(8);
            f.text(more);
            f.x = x0;
            f.w = w0;
            f.footer(&[allow, deny], &[]);
        });
        // 回车默认是「拒绝」，免得误按
        d.set_default(IDCANCEL_);
        let answer = {
            let me = self.clone();
            let c = ctx.clone();
            Rc::new(move |allow: bool| {
                let d = me.open.borrow_mut().remove(&request_id);
                if let Some(d) = d {
                    c.client.send(Request::AssistAnswer { request_id, allow });
                    d.close();
                }
            })
        };
        let a = answer.clone();
        on_click(allow, move || a(true));
        let a = answer.clone();
        on_click(deny, move || a(false));
        let a = answer.clone();
        d.on_ok(move || a(true));
        // 关掉窗口当拒绝
        let a = answer.clone();
        d.on_cancel(move || a(false));
        self.open.borrow_mut().insert(request_id, d.clone());
        d.show();
        deny.focus();
        d.flash();
    }

    /// 对方取消或超时：直接关（不用再回）
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
