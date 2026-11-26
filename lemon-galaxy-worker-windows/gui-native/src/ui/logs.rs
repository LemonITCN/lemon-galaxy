//! 事件日志：本机远程操作记录，最新的在上面；可只看警告。列表用系统的 ListView（报表视图）。

use super::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use crate::win::layout::Panel;
use windows::core::PWSTR;
use windows::Win32::Foundation::{COLORREF, LPARAM, LRESULT};
use windows::Win32::UI::Controls::*;

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

const COLS: [i32; 3] = [140, 100, 150];

pub fn build(ctx: &Rc<Ctx>, parent: HWND) -> PageView {
    let panel = Panel::new(parent, true);
    let p = panel.hwnd;
    let title = label_font(p, tr!("事件日志", "Event Log"), Font::Title);
    let title_info = info(
        p,
        tr!(
            "本机记录每次连接、每个文件操作、每条命令；AI 发起的操作单独标记，被拒绝的连接与被拦下的 AI 操作标为警告。日志只存在这台电脑上。",
            "Every connection, file operation and command is recorded on this computer. AI actions are marked, and rejected connections and blocked AI actions are shown as warnings. The log never leaves this computer."
        ),
    );
    let only = checkbox(p, tr!("只看警告", "Warnings only"));
    let refresh_btn = button(p, tr!("刷新", "Refresh"));
    let denied = placeholder(p, tr!("需要管理员权限", "Administrator access required"), tr!("只有管理员组成员或账号绑定的系统用户能查看事件日志。", "Only members of the Administrators group or the bound system user can view the event log."));
    let lv = create("SysListView32", "", LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS | LVS_NOSORTHEADER | WS_TABSTOP.0, WS_EX_CLIENTEDGE.0, p, None);
    lv.send(LVM_SETEXTENDEDLISTVIEWSTYLE, 0, (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_LABELTIP) as isize);
    unsafe {
        // Vista 起是资源管理器风格的列表；XP 上没有这个主题，照旧
        let _ = windows::Win32::UI::Controls::SetWindowTheme(lv.0, windows::core::w!("Explorer"), windows::core::PCWSTR::null());
    }
    let heads = [tr!("时间", "Time"), tr!("类型", "Type"), tr!("用户", "User"), tr!("详情", "Details")];
    for (i, h) in heads.iter().enumerate() {
        let mut t = wide(h);
        let col = LVCOLUMNW { mask: LVCF_TEXT | LVCF_WIDTH | LVCF_SUBITEM, cx: px(*COLS.get(i).unwrap_or(&300)), pszText: PWSTR(t.as_mut_ptr()), iSubItem: i as i32, ..Default::default() };
        lv.send(LVM_INSERTCOLUMNW, i, &col as *const _ as isize);
    }
    // 警告标红
    on_notify(lv, move |nm| unsafe {
        if (*nm).code != NM_CUSTOMDRAW {
            return None;
        }
        let cd = &mut *(nm as *mut NMLVCUSTOMDRAW);
        if cd.nmcd.dwDrawStage == CDDS_PREPAINT {
            return Some(LRESULT(CDRF_NOTIFYITEMDRAW as isize));
        }
        if cd.nmcd.dwDrawStage == CDDS_ITEMPREPAINT {
            if cd.nmcd.lItemlParam.0 == 1 {
                cd.clrText = COLORREF(tone_color(Tone::Error));
            }
            return Some(LRESULT(CDRF_DODEFAULT as isize));
        }
        None
    });

    let events: Rc<RefCell<Vec<LogEntry>>> = Rc::new(RefCell::new(vec![]));

    let c = ctx.clone();
    panel.set_layout(move |f| {
        if !c.rights().view_config {
            denied.place(f);
            return;
        }
        f.title(title, Some(title_info), &[only, refresh_btn]);
        f.gap(14);
        let w = f.w;
        f.fill(lv, px(200));
        // 详情一栏占满剩下的宽度
        let rest = w - COLS.iter().map(|c| px(*c)).sum::<i32>() - unsafe { GetSystemMetrics(SM_CXVSCROLL) } - px(6);
        lv.send(LVM_SETCOLUMNWIDTH, 3, rest.max(px(160)) as isize);
    });

    let fill = {
        let events = events.clone();
        Rc::new(move || {
            lv.send(WM_SETREDRAW, 0, 0);
            lv.send(LVM_DELETEALLITEMS, 0, 0);
            let all = events.borrow();
            let warn_only = only.checked();
            let mut row = 0usize;
            for e in all.iter().rev().filter(|e| !warn_only || warning(&e.kind)) {
                let kind = if e.ai { format!("{} · AI", kind_text(&e.kind)) } else { kind_text(&e.kind) };
                let detail = e.detail.replace(['\r', '\n'], " ");
                let texts = [fmt_time(e.at, true), kind, e.user.clone(), detail];
                let mut t0 = wide(&texts[0]);
                let it = LVITEMW { mask: LVIF_TEXT | LVIF_PARAM, iItem: row as i32, pszText: PWSTR(t0.as_mut_ptr()), lParam: LPARAM(warning(&e.kind) as isize), ..Default::default() };
                let idx = lv.send(LVM_INSERTITEMW, 0, &it as *const _ as isize);
                if idx < 0 {
                    continue;
                }
                for (sub, text) in texts.iter().enumerate().skip(1) {
                    let mut t = wide(text);
                    let it = LVITEMW { iSubItem: sub as i32, pszText: PWSTR(t.as_mut_ptr()), ..Default::default() };
                    lv.send(LVM_SETITEMTEXTW, idx as usize, &it as *const _ as isize);
                }
                row += 1;
            }
            lv.send(WM_SETREDRAW, 1, 0);
            lv.redraw();
        })
    };
    let f2 = fill.clone();
    on_click(only, move || f2());

    let c = ctx.clone();
    let ev = events.clone();
    let f2 = fill.clone();
    let busy = Rc::new(Cell::new(false));
    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        if !c.rights().view_config || busy.replace(true) {
            return;
        }
        let (c2, ev, f2, busy) = (c.clone(), ev.clone(), f2.clone(), busy.clone());
        c.client.call(Request::Events { limit: 500 }, move |r| {
            busy.set(false);
            let Response::Events { events } = r else {
                c2.report(&r);
                return;
            };
            *ev.borrow_mut() = events;
            f2();
        });
    });
    let r2 = refresh.clone();
    on_click(refresh_btn, move || r2());
    let pw = Rc::downgrade(&panel);
    let last = Rc::new(Cell::new(None::<bool>));
    ctx.on_status(move |s| {
        if last.replace(Some(s.rights.view_config)) != Some(s.rights.view_config) {
            if let Some(p) = pw.upgrade() {
                p.relayout_soon();
            }
        }
    });
    PageView { panel, refresh: Some(refresh) }
}
