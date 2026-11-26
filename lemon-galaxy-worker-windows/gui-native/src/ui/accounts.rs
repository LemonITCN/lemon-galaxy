//! 账号与权限：列表（用户名、显示名、状态、系统用户、AI 上限）、登录、重新登录、退出、逐个账号的本机权限；服务器地址（可复制）。

use super::*;
use crate::win::dialog::{Dialog, IDCANCEL_, IDOK_};
use crate::win::layout::{Panel, Right};
use galaxy_proto::AiMode;

struct AccountRow {
    account: Account,
    name: Ctl,
    sub: Ctl,
    relogin: Ctl,
    perm: Ctl,
    out: Ctl,
}

#[derive(Clone, Copy, PartialEq)]
enum Load {
    Loading,
    Ready,
}

pub fn build(ctx: &Rc<Ctx>, parent: HWND) -> PageView {
    let panel = Panel::new(parent, true);
    let p = panel.hwnd;
    let title = label_font(p, tr!("账号与权限", "Accounts"), Font::Title);
    let title_info = info(
        p,
        tr!(
            "可以登录多个账号（必须在同一台服务器上），这台电脑会出现在每个账号的设备列表里。每个账号在本机单独设置权限，所有限制都由本机执行，主控端改不了。",
            "You can sign in several accounts (on the same server); this computer shows up in each account's device list. Each account has its own permissions here, enforced by this computer; controllers cannot change them."
        ),
    );
    let add = button(p, tr!("登录账号", "Sign In"));
    let denied = placeholder(p, tr!("需要管理员权限", "Administrator access required"), tr!("只有管理员组成员或账号绑定的系统用户能查看账号。", "Only members of the Administrators group or the bound system user can view accounts."));
    let loading = label_tone(p, tr!("正在读取…", "Loading…"), Tone::Dim);
    let empty = label_tone(p, tr!("还没有登录账号", "No account signed in"), Tone::Dim);
    let server_h = label_font(p, tr!("服务器", "Server"), Font::Bold);
    let server_v = value(p, "");
    let server_info = info(p, tr!("一台电脑上的账号必须在同一台服务器上。被控端固定了服务器证书指纹，证书被替换时拒绝连接。", "All accounts on this computer use the same server. Its certificate fingerprint is pinned, so a replaced certificate is rejected."));
    let c = ctx.clone();
    let server_copy = copy_button(ctx, p, move || c.status.borrow().as_ref().map(|s| s.server.clone()).unwrap_or_default());

    let rows: Rc<RefCell<Vec<AccountRow>>> = Rc::new(RefCell::new(vec![]));
    let load = Rc::new(Cell::new(Load::Loading));

    let c = ctx.clone();
    on_click(add, move || super::login::open(&c, None));

    let c = ctx.clone();
    let rows2 = rows.clone();
    let load2 = load.clone();
    panel.set_layout(move |f| {
        let st = c.status.borrow();
        let Some(s) = st.as_ref() else { return };
        if !s.rights.view_config {
            denied.place(f);
            return;
        }
        f.title(title, Some(title_info), &[add]);
        f.gap(16);
        let rows = rows2.borrow();
        f.group(|g| {
            if load2.get() == Load::Loading && rows.is_empty() {
                g.note(loading);
            } else if rows.is_empty() {
                g.note(empty);
            }
            for r in rows.iter() {
                let mut right = vec![];
                if !r.account.active {
                    right.push(Right::C(r.relogin));
                }
                right.push(Right::C(r.perm));
                right.push(Right::C(r.out));
                g.row(r.name, None, Some(r.sub), &right);
            }
        });
        if !s.server.is_empty() {
            f.gap(22);
            f.title(server_h, None, &[]);
            f.gap(8);
            f.group(|g| g.row(server_v, Some(server_info), None, &[Right::C(server_copy)]));
        }
    });

    // 读账号列表
    let c = ctx.clone();
    let rows2 = rows.clone();
    let load2 = load.clone();
    let pw = Rc::downgrade(&panel);
    let busy = Rc::new(Cell::new(false));
    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        if !c.rights().view_config || busy.replace(true) {
            return;
        }
        let (c2, rows, load, pw, busy) = (c.clone(), rows2.clone(), load2.clone(), pw.clone(), busy.clone());
        c.client.call(Request::Accounts, move |r| {
            busy.set(false);
            let Response::Accounts { accounts } = r else {
                c2.report(&r);
                return;
            };
            load.set(Load::Ready);
            if let Some(p) = pw.upgrade() {
                rebuild_rows(&c2, p.hwnd, &rows, accounts);
                p.relayout();
            }
        });
    });

    // 账号数、需要重新登录的数、权限、配置版本变了就重新读
    let sig = Rc::new(Cell::new(None::<(u32, u32, bool, bool, u64)>));
    let r2 = refresh.clone();
    let pw = Rc::downgrade(&panel);
    ctx.on_status(move |s| {
        server_v.set_text(server_host(&s.server));
        add.enable(s.rights.can_manage());
        add.set_shield(s.rights.manage == ManageRight::Elevate);
        let sg = (s.account_count, s.needs_relogin, s.rights.view_config, s.rights.can_manage(), s.config_rev);
        if sig.replace(Some(sg)) != Some(sg) {
            r2();
        }
        if let Some(p) = pw.upgrade() {
            p.relayout_soon();
        }
    });
    PageView { panel, refresh: Some(refresh) }
}

fn rebuild_rows(ctx: &Rc<Ctx>, p: HWND, rows: &Rc<RefCell<Vec<AccountRow>>>, accounts: Vec<Account>) {
    for r in rows.borrow_mut().drain(..) {
        for c in [r.name, r.sub, r.relogin, r.perm, r.out] {
            c.destroy();
        }
    }
    let rights = ctx.rights();
    let manage = rights.can_manage();
    let shield = rights.manage == ManageRight::Elevate;
    let mut v = vec![];
    for a in accounts {
        let title = if a.display_name.is_empty() || a.display_name == a.username { a.username.clone() } else { format!("{}（{}）", a.display_name, a.username) };
        let sub_text = if a.active {
            crate::i18n::fmt(tr!("以系统用户 {0} 运行 · AI 上限：{1}", "Runs as {0} · AI limit: {1}"), &[&a.policy.system_user, ai_mode_text(a.policy.ai_cap)])
        } else {
            tr!("账号被管理员重置，需要重新登录", "Reset by an administrator; sign in again").to_string()
        };
        let name = label(p, &title);
        let sub = label_tone(p, &sub_text, if a.active { Tone::Dim } else { Tone::Warning });
        let relogin = button(p, tr!("重新登录", "Sign In Again"));
        relogin.enable(manage);
        relogin.set_shield(shield);
        let perm = button(p, tr!("权限", "Permissions"));
        let out = button(p, tr!("退出", "Sign Out"));
        out.enable(manage);
        let (c, user) = (ctx.clone(), a.username.clone());
        on_click(relogin, move || super::login::open(&c, Some(&user)));
        let (c, acct) = (ctx.clone(), a.clone());
        on_click(perm, move || open_policy(&c, &acct));
        let (c, acct) = (ctx.clone(), a.clone());
        on_click(out, move || {
            let title = crate::i18n::fmt(tr!("退出 {0}？", "Sign out {0}?"), &[&acct.username]);
            let ok = c.confirm(
                &title,
                tr!("这个账号的连接会立即断开，这台电脑会从它的设备列表里移除。之后要再次登录才能被它控制。", "Its sessions end immediately and this computer is removed from its device list. Sign in again to allow it to connect."),
                tr!("退出", "Sign Out"),
                true,
            );
            if !ok {
                return;
            }
            let (c2, name) = (c.clone(), acct.username.clone());
            c.act(Request::Logout { user_id: acct.user_id }, move |r| {
                if r.is_some() {
                    c2.toast(&crate::i18n::fmt(tr!("已退出 {0}", "Signed out {0}"), &[&name]));
                }
            });
        });
        v.push(AccountRow { account: a, name, sub, relogin, perm, out });
    }
    *rows.borrow_mut() = v;
}

const AI_MODES: [AiMode; 5] = [AiMode::Off, AiMode::ReadOnly, AiMode::ConfirmAll, AiMode::ConfirmDangerous, AiMode::FullTrust];

/// 账号在本机的权限（模态对话框）
fn open_policy(ctx: &Rc<Ctx>, a: &Account) {
    let manage = ctx.rights().can_manage();
    let title = format!("{} — {}", tr!("权限", "Permissions"), a.username);
    let d = Dialog::new(ctx.parent(), &title, 440, ctx.parent().is_some(), false);
    let p = d.panel.hwnd;
    let pol = &a.policy;

    let run_h = label_font(p, tr!("运行身份", "Run As"), Font::Bold);
    let user_l = label(p, tr!("系统用户", "System user"));
    let user_info = info(p, tr!("这个账号远程打开的终端、执行的命令、读写的文件都以这个 Windows 账号的身份进行，只有它的权限。选 SYSTEM 风险很高，会要求再确认一次。", "Remote terminals, commands and file access for this account run as this Windows account, with only its permissions. SYSTEM is risky and asks for confirmation."));
    let users = combo(p, &[(pol.system_user.as_str(), pol.system_user.as_str())]);
    users.select_index(0);
    set_width(users, px(200));
    users.enable(false);

    let allow_h = label_font(p, tr!("允许对方", "Allow"), Font::Bold);
    let mk = |text: &str, on: bool| {
        let c = checkbox(p, text);
        c.set_checked(on);
        c.enable(manage);
        c
    };
    let view = mk(tr!("查看画面", "View screen"), pol.view);
    let control = mk(tr!("控制键盘鼠标", "Control keyboard and mouse"), pol.control);
    let files = mk(tr!("文件", "Files"), pol.files);
    let terminal = mk(tr!("终端与命令", "Terminal and commands"), pol.terminal);
    let clipboard = mk(tr!("剪贴板", "Clipboard"), pol.clipboard);
    let audio = mk(tr!("声音", "Sound"), pol.audio);
    let notify = mk(tr!("被控时提示", "Notify when controlled"), pol.notify);
    let notify_info = info(p, tr!("这个账号连入时发系统通知。托盘图标总会变色。", "Show a notification when this account connects. The tray icon always changes color."));

    let ai_h = label_font(p, "AI", Font::Bold);
    let ai_l = label(p, tr!("AI 上限", "AI limit"));
    let ai_info = info(p, tr!("这个账号通过 AI 操作本机时最多能到哪一档。即使主控端选了更高的档位，也按这里执行；选「禁止」则 AI 不能操作本机。", "The highest level AI may use on this computer for this account. Even if the controller picks a higher level, this limit applies. Off means AI cannot operate this computer."));
    let items: Vec<(&str, &str)> = AI_MODES.iter().map(|m| (m.as_str(), ai_mode_text(*m))).collect();
    let ai = combo(p, &items);
    ai.select_id(pol.ai_cap.as_str());
    set_width(ai, px(180));
    ai.enable(manage);

    let error = label_tone(p, "", Tone::Error);
    let save = button_id(p, tr!("保存", "Save"), IDOK_, true);
    let cancel = button_id(p, tr!("取消", "Cancel"), IDCANCEL_, false);
    save.enable(manage);
    save.set_shield(ctx.needs_elevation());

    d.set_layout(move |f| {
        f.title(run_h, None, &[]);
        f.gap(8);
        f.group(|g| g.row(user_l, Some(user_info), None, &[Right::C(users)]));
        f.gap(18);
        f.title(allow_h, None, &[]);
        f.gap(8);
        f.group(|g| {
            for c in [view, control, files, terminal, clipboard, audio] {
                g.row(c, None, None, &[]);
            }
            g.row(notify, Some(notify_info), None, &[]);
        });
        f.gap(18);
        f.title(ai_h, None, &[]);
        f.gap(8);
        f.group(|g| g.row(ai_l, Some(ai_info), None, &[Right::C(ai)]));
        if !error.text().is_empty() {
            f.gap(12);
            f.text(error);
        }
        f.footer(&[save, cancel], &[]);
    });

    // 可选的系统用户
    let cur = pol.system_user.clone();
    let dw = Rc::downgrade(&d);
    ctx.client.call(Request::SystemUsers, move |r| {
        let Some(_) = dw.upgrade() else { return };
        if let Response::SystemUsers { users: list, can_choose, .. } = r {
            let mut all = list;
            if !all.iter().any(|u| u.eq_ignore_ascii_case(&cur)) {
                all.insert(0, cur.clone());
            }
            let items: Vec<(&str, &str)> = all.iter().map(|u| (u.as_str(), u.as_str())).collect();
            users.set_items(&items);
            if !users.select_id(&cur) {
                users.select_index(0);
            }
            users.enable(manage && can_choose);
        }
    });

    let busy = Rc::new(Cell::new(false));
    let acct = a.clone();
    let c = ctx.clone();
    let dw = Rc::downgrade(&d);
    let do_save = move || {
        if busy.get() || !manage {
            return;
        }
        let policy = Policy {
            system_user: users.selected_id().unwrap_or_else(|| acct.policy.system_user.clone()),
            view: view.checked(),
            control: control.checked(),
            files: files.checked(),
            terminal: terminal.checked(),
            clipboard: clipboard.checked(),
            audio: audio.checked(),
            ai_cap: ai.selected_id().and_then(|s| AiMode::parse(&s)).unwrap_or(acct.policy.ai_cap),
            notify: notify.checked(),
        };
        busy.set(true);
        save.enable(false);
        save_policy(c.clone(), dw.clone(), acct.user_id, policy, false, busy.clone(), save, error);
    };
    let ds = Rc::new(do_save);
    let f1 = ds.clone();
    on_click(save, move || f1());
    let f2 = ds.clone();
    d.on_ok(move || f2());
    let dd = d.clone();
    on_click(cancel, move || dd.close());
    d.show();
    if manage {
        view.focus();
    } else {
        cancel.focus();
    }
}

#[allow(clippy::too_many_arguments)]
fn save_policy(c: Rc<Ctx>, dw: std::rc::Weak<Dialog>, user_id: u64, policy: Policy, confirm_root: bool, busy: Rc<Cell<bool>>, save: Ctl, error: Ctl) {
    let c2 = c.clone();
    let p2 = policy.clone();
    c.call(Request::SetPolicy { user_id, policy, confirm_root }, move |r| {
        let Some(d) = dw.upgrade() else { return };
        if d.is_closed() {
            return;
        }
        match r {
            Response::Error { code: ErrorCode::ConfirmRoot, message } if !confirm_root => {
                if crate::win::dialog::confirm(Some(d.hwnd), app_name(), tr!("以 SYSTEM 身份运行？", "Run as SYSTEM?"), &message, tr!("仍然使用 SYSTEM", "Use SYSTEM Anyway"), tr!("取消", "Cancel"), true) {
                    save_policy(c2, dw, user_id, p2, true, busy, save, error);
                    return;
                }
            }
            Response::Error { message, .. } => {
                error.set_text(&message);
                d.fit();
            }
            _ => {
                c2.toast(tr!("权限已保存", "Permissions saved"));
                d.close();
                return;
            }
        }
        busy.set(false);
        save.enable(true);
    });
}
