//! 事件日志：本机远程操作记录（events.log），最新的在上面。

use super::*;
use crate::control::*;
use crate::tr;
use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

fn kind_text(k: &str) -> String {
    let t = match k {
        "connect" => tr!("连接", "Connect"),
        "disconnect" => tr!("断开", "Disconnect"),
        "reject" => tr!("拒绝连接", "Rejected"),
        "auth_failed" => tr!("认证失败", "Auth failed"),
        "terminal" => tr!("终端", "Terminal"),
        "file" => tr!("文件", "File"),
        "exec" => tr!("命令", "Command"),
        "display" => tr!("画面", "Screen"),
        "forward" => tr!("端口映射", "Port forward"),
        "clipboard" => tr!("剪贴板", "Clipboard"),
        "privacy" => tr!("隐私屏", "Privacy screen"),
        "ai" => "AI",
        "ai_denied" => tr!("AI 被拒", "AI denied"),
        "denied" => tr!("拒绝", "Denied"),
        "assist" => tr!("协助", "Assistance"),
        "policy" => tr!("设置", "Settings"),
        other => return other.to_string(),
    };
    t.to_string()
}

fn warning(k: &str) -> bool {
    matches!(k, "reject" | "auth_failed" | "ai_denied" | "denied")
}

pub fn build(ctx: &Rc<Ctx>) -> (gtk::Widget, Rc<dyn Fn()>) {
    let stack = gtk::Stack::new();
    let (ph, _, _, _) = placeholder(
        "changes-prevent-symbolic",
        tr!("需要管理员权限", "Administrator access required"),
        tr!("只有管理员（sudo、wheel、admin 组）或账号绑定的系统用户能查看事件日志。", "Only administrators (sudo, wheel or admin group) or the bound system user can view the event log."),
    );
    stack.add_named(&ph, "denied");
    let (scroll, content) = page();
    let sec = section(
        tr!("事件日志", "Event Log"),
        Some(tr!(
            "本机记录每次连接、每个文件操作、每条命令；AI 发起的操作单独标记，被拒绝的连接与被拦下的 AI 操作标为警告。日志只存在这台电脑上。",
            "Every connection, file operation and command is recorded on this computer. AI actions are marked, and rejected connections and blocked AI actions are shown as warnings. The log never leaves this computer."
        )),
    );
    let only_warn = gtk::CheckButton::with_label(tr!("只看警告", "Warnings only"));
    let refresh_btn = gtk::Button::from_icon_name(Some("view-refresh-symbolic"), gtk::IconSize::Button);
    refresh_btn.set_tooltip_text(Some(tr!("刷新", "Refresh")));
    sec.header.pack_end(&refresh_btn, false, false, 0);
    sec.header.pack_end(&only_warn, false, false, 8);
    content.pack_start(&sec.root, false, false, 0);
    stack.add_named(&scroll, "main");

    let list = sec.list.clone();
    let c = ctx.clone();
    let stack2 = stack.clone();
    let loading = Rc::new(Cell::new(false));
    let ow = only_warn.clone();
    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        let Some(r) = c.rights() else { return };
        if !r.view_config {
            stack2.set_visible_child_name("denied");
            return;
        }
        stack2.set_visible_child_name("main");
        if loading.replace(true) {
            return;
        }
        let (c, list, loading, only) = (c.clone(), list.clone(), loading.clone(), ow.is_active());
        spawn(async move {
            let r = c.client.call(Request::Events { limit: 500 }).await;
            loading.set(false);
            let Response::Events { events } = r else {
                c.report(&r);
                return;
            };
            clear(&list);
            let shown: Vec<&LogEntry> = events.iter().rev().filter(|e| !only || warning(&e.kind)).collect();
            if shown.is_empty() {
                let r = gtk::ListBoxRow::new();
                r.set_activatable(false);
                let l = label(tr!("没有记录", "No events"), &["dim-label", "lg-row"]);
                l.set_margin_top(6);
                l.set_margin_bottom(6);
                r.add(&l);
                list.add(&r);
            }
            for e in shown {
                list.add(&event_row(e));
            }
            list.show_all();
        });
    });
    let r2 = refresh.clone();
    refresh_btn.connect_clicked(move |_| r2());
    let r2 = refresh.clone();
    only_warn.connect_toggled(move |_| r2());
    (stack.upcast(), refresh)
}

fn event_row(e: &LogEntry) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    let g = gtk::Grid::new();
    g.style_context().add_class("lg-row");
    g.set_column_spacing(12);
    g.set_row_spacing(2);
    let time = label(&fmt_time(e.at, true), &["lg-mono", "dim-label", "lg-small"]);
    time.set_valign(gtk::Align::Start);
    g.attach(&time, 0, 0, 1, 1);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let kind = label(&kind_text(&e.kind), &["lg-badge"]);
    if warning(&e.kind) {
        kind.style_context().add_class("error");
    }
    head.pack_start(&kind, false, false, 0);
    if e.ai {
        head.pack_start(&label("AI", &["lg-badge", "lg-badge-ai"]), false, false, 0);
    }
    if !e.user.is_empty() {
        head.pack_start(&label(&e.user, &[]), false, false, 0);
    }
    g.attach(&head, 1, 0, 1, 1);
    if !e.detail.is_empty() {
        let d = wrap_label(&e.detail, &["dim-label", "lg-small"]);
        d.set_selectable(true);
        d.set_hexpand(true);
        g.attach(&d, 1, 1, 1, 1);
    }
    row.add(&g);
    row
}
