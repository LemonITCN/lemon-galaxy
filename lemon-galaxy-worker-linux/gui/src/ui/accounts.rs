//! 账号与权限：列表（用户名、显示名、状态、系统用户）、登录、退出、逐个账号的本机权限。

use super::*;
use crate::control::*;
use crate::tr;
use galaxy_proto::AiMode;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub fn build(ctx: &Rc<Ctx>) -> (gtk::Widget, Rc<dyn Fn()>) {
    let stack = gtk::Stack::new();
    let (ph, _, _, _) = placeholder(
        "changes-prevent-symbolic",
        tr!("需要管理员权限", "Administrator access required"),
        tr!("只有管理员（sudo、wheel、admin 组）或账号绑定的系统用户能查看账号。", "Only administrators (sudo, wheel or admin group) or the bound system user can view accounts."),
    );
    stack.add_named(&ph, "denied");

    let (scroll, content) = page();
    let sec = section(
        tr!("账号", "Accounts"),
        Some(tr!(
            "可以登录多个账号（必须在同一台服务器上），这台电脑会出现在每个账号的设备列表里。每个账号在本机单独设置权限，所有限制都由本机执行，主控端改不了。",
            "You can sign in several accounts (on the same server); this computer shows up in each account's device list. Each account has its own permissions here, enforced by this computer; controllers cannot change them."
        )),
    );
    let add = gtk::Button::with_label(tr!("登录账号…", "Sign In…"));
    add.style_context().add_class("suggested-action");
    sec.header.pack_end(&add, false, false, 0);
    let c = ctx.clone();
    add.connect_clicked(move |_| super::login::open(&c));
    content.pack_start(&sec.root, false, false, 0);
    stack.add_named(&scroll, "list");

    let list = sec.list.clone();
    let ctx2 = ctx.clone();
    let stack2 = stack.clone();
    let add2 = add.clone();
    let loading = Rc::new(Cell::new(false));
    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        let Some(r) = ctx2.rights() else { return };
        if !r.view_config {
            stack2.set_visible_child_name("denied");
            return;
        }
        stack2.set_visible_child_name("list");
        add2.set_sensitive(r.can_manage());
        if loading.replace(true) {
            return;
        }
        let (c, list, loading) = (ctx2.clone(), list.clone(), loading.clone());
        spawn(async move {
            let r = c.client.call(Request::Accounts).await;
            loading.set(false);
            let Response::Accounts { accounts } = r else {
                c.report(&r);
                return;
            };
            let manage = c.rights().is_some_and(|r| r.can_manage());
            clear(&list);
            if accounts.is_empty() {
                let r = gtk::ListBoxRow::new();
                r.set_activatable(false);
                let l = label(tr!("还没有登录账号", "No account signed in"), &["dim-label", "lg-row"]);
                l.set_margin_top(6);
                l.set_margin_bottom(6);
                r.add(&l);
                list.add(&r);
            }
            for a in &accounts {
                list.add(&account_row(&c, a, manage));
            }
            list.show_all();
        });
    });
    // 账号数、需要重新登录的数、权限、配置版本变了就重新读
    let sig = Rc::new(Cell::new(None::<(u32, u32, bool, bool, u64)>));
    let r2 = refresh.clone();
    ctx.on_status(move |st| {
        let s = (st.account_count, st.needs_relogin, st.rights.view_config, st.rights.can_manage(), st.config_rev);
        if sig.replace(Some(s)) != Some(s) {
            r2();
        }
    });
    (stack.upcast(), refresh)
}

fn account_row(ctx: &Rc<Ctx>, a: &Account, manage: bool) -> gtk::ListBoxRow {
    let title = if a.display_name.is_empty() || a.display_name == a.username { a.username.clone() } else { format!("{}（{}）", a.display_name, a.username) };
    let r = row(&title, None, None);
    let icon = gtk::Image::from_icon_name(Some("avatar-default-symbolic"), gtk::IconSize::LargeToolbar);
    icon.set_pixel_size(24);
    r.use_prefix(&icon);
    let sub = if a.active {
        if crate::i18n::zh() {
            format!("以系统用户 {} 运行 · AI 上限：{}", a.policy.system_user, ai_mode_text(a.policy.ai_cap))
        } else {
            format!("Runs as {} · AI limit: {}", a.policy.system_user, ai_mode_text(a.policy.ai_cap))
        }
    } else {
        tr!("账号被管理员重置，需要重新登录", "Reset by an administrator; sign in again").to_string()
    };
    r.set_subtitle(&sub);
    if !a.active {
        let b = gtk::Button::with_label(tr!("重新登录", "Sign In Again"));
        b.set_sensitive(manage);
        let c = ctx.clone();
        b.connect_clicked(move |_| super::login::open(&c));
        r.suffix.pack_start(&b, false, false, 0);
    }
    let perm = gtk::Button::with_label(tr!("权限…", "Permissions…"));
    let (c, acct) = (ctx.clone(), a.clone());
    perm.connect_clicked(move |_| open_policy(&c, &acct));
    r.suffix.pack_start(&perm, false, false, 0);
    let out = gtk::Button::with_label(tr!("退出", "Sign Out"));
    out.set_sensitive(manage);
    let (c, acct) = (ctx.clone(), a.clone());
    out.connect_clicked(move |_| {
        let (c, acct) = (c.clone(), acct.clone());
        spawn(async move {
            let title = if crate::i18n::zh() { format!("退出 {}？", acct.username) } else { format!("Sign out {}?", acct.username) };
            let ok = confirm(
                c.parent(),
                &title,
                tr!("这个账号的连接会立即断开，这台电脑会从它的设备列表里移除。之后要再次登录才能被它控制。", "Its sessions end immediately and this computer is removed from its device list. Sign in again to allow it to connect."),
                tr!("退出", "Sign Out"),
                true,
            )
            .await;
            if ok && c.call(Request::Logout { user_id: acct.user_id }).await.is_some() {
                c.toast(&if crate::i18n::zh() { format!("已退出 {}", acct.username) } else { format!("Signed out {}", acct.username) });
            }
        });
    });
    r.suffix.pack_start(&out, false, false, 0);
    r.row
}

const AI_MODES: [AiMode; 5] = [AiMode::Off, AiMode::ReadOnly, AiMode::ConfirmAll, AiMode::ConfirmDangerous, AiMode::FullTrust];

/// 权限编辑
fn open_policy(ctx: &Rc<Ctx>, a: &Account) {
    let manage = ctx.rights().is_some_and(|r| r.can_manage());
    let win = gtk::Window::new(gtk::WindowType::Toplevel);
    win.set_modal(true);
    win.set_transient_for(ctx.parent().as_ref());
    win.set_destroy_with_parent(true);
    win.set_type_hint(gtk::gdk::WindowTypeHint::Dialog);
    win.set_position(gtk::WindowPosition::CenterOnParent);
    win.set_default_size(520, 640);
    let header = gtk::HeaderBar::new();
    header.set_title(Some(tr!("权限", "Permissions")));
    header.set_subtitle(Some(&a.username));
    let cancel = gtk::Button::with_label(tr!("取消", "Cancel"));
    header.pack_start(&cancel);
    let save = gtk::Button::with_label(tr!("保存", "Save"));
    save.style_context().add_class("suggested-action");
    save.set_sensitive(manage);
    header.pack_end(&save);
    win.set_titlebar(Some(&header));

    let sw = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    sw.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 22);
    content.set_margin_top(20);
    content.set_margin_bottom(24);
    content.set_margin_start(24);
    content.set_margin_end(24);
    sw.add(&content);
    win.add(&sw);

    let s1 = section(tr!("运行身份", "Run As"), None);
    let r = row(
        tr!("系统用户", "System user"),
        None,
        Some(tr!(
            "终端、命令、文件以这个系统用户的身份运行，只有这个用户的权限。被控服务以普通用户运行时只能用这个用户。",
            "Terminals, commands and files run as this system user, with only that user's permissions. When the service runs as a regular user, only that user is available."
        )),
    );
    let users = gtk::ComboBoxText::new();
    users.append(Some(&a.policy.system_user), &a.policy.system_user);
    users.set_active_id(Some(&a.policy.system_user));
    r.suffix.pack_start(&users, false, false, 0);
    s1.list.add(&r.row);
    content.pack_start(&s1.root, false, false, 0);

    let s2 = section(tr!("允许对方", "Allow"), None);
    let mk = |title: &str, on: bool, info: Option<&str>| {
        let r = row(title, None, info);
        let sw = gtk::Switch::new();
        sw.set_active(on);
        sw.set_valign(gtk::Align::Center);
        r.suffix.pack_start(&sw, false, false, 0);
        s2.list.add(&r.row);
        sw
    };
    let p = &a.policy;
    let view = mk(tr!("查看画面", "View screen"), p.view, None);
    let control = mk(tr!("控制键盘鼠标", "Control keyboard and mouse"), p.control, None);
    let files = mk(tr!("文件", "Files"), p.files, None);
    let terminal = mk(tr!("终端与命令", "Terminal and commands"), p.terminal, None);
    let clipboard = mk(tr!("剪贴板", "Clipboard"), p.clipboard, None);
    let audio = mk(tr!("声音", "Sound"), p.audio, None);
    let notify = mk(
        tr!("被控时提示", "Notify when controlled"),
        p.notify,
        Some(tr!("这个账号连入时在屏幕右上角发通知。托盘图标总会变色。", "Show a notification when this account connects. The tray icon always changes color.")),
    );
    content.pack_start(&s2.root, false, false, 0);

    let s3 = section(tr!("AI", "AI"), None);
    let r = row(
        tr!("AI 上限", "AI limit"),
        None,
        Some(tr!(
            "这个账号通过 AI 操作本机时最多能到哪一档。即使主控端选了更高的档位，也按这里执行；选「禁止」则 AI 不能操作本机。",
            "The highest level AI may use on this computer for this account. Even if the controller picks a higher level, this limit applies. Off means AI cannot operate this computer."
        )),
    );
    let ai = gtk::ComboBoxText::new();
    for m in AI_MODES {
        ai.append(Some(m.as_str()), ai_mode_text(m));
    }
    ai.set_active_id(Some(p.ai_cap.as_str()));
    r.suffix.pack_start(&ai, false, false, 0);
    s3.list.add(&r.row);
    content.pack_start(&s3.root, false, false, 0);

    if !manage {
        content.set_sensitive(false);
    }

    // 可选的系统用户
    let (c, users2, cur) = (ctx.clone(), users.clone(), a.policy.system_user.clone());
    spawn(async move {
        if let Response::SystemUsers { users, can_choose, .. } = c.client.call(Request::SystemUsers).await {
            users2.remove_all();
            let mut all = users;
            if !all.contains(&cur) {
                all.insert(0, cur.clone());
            }
            for u in &all {
                users2.append(Some(u), u);
            }
            users2.set_active_id(Some(&cur));
            users2.set_sensitive(can_choose);
        }
    });

    let busy = Rc::new(Cell::new(false));
    let acct = RefCell::new(a.clone());
    let (c, w2) = (ctx.clone(), win.clone());
    save.connect_clicked(move |b| {
        if busy.replace(true) {
            return;
        }
        let a = acct.borrow().clone();
        let policy = Policy {
            system_user: users.active_text().map(|s| s.to_string()).unwrap_or(a.policy.system_user.clone()),
            view: view.is_active(),
            control: control.is_active(),
            files: files.is_active(),
            terminal: terminal.is_active(),
            clipboard: clipboard.is_active(),
            audio: audio.is_active(),
            ai_cap: ai.active_id().and_then(|s| AiMode::parse(&s)).unwrap_or(a.policy.ai_cap),
            notify: notify.is_active(),
        };
        let (c, w2, b, busy) = (c.clone(), w2.clone(), b.clone(), busy.clone());
        b.set_sensitive(false);
        spawn(async move {
            let mut confirm_root = false;
            loop {
                let r = c.client.call(Request::SetPolicy { user_id: a.user_id, policy: policy.clone(), confirm_root }).await;
                match r {
                    Response::Error { code: ErrorCode::ConfirmRoot, message } if !confirm_root => {
                        if confirm(Some(w2.clone()), tr!("以 root 身份运行？", "Run as root?"), &message, tr!("仍然使用 root", "Use root Anyway"), true).await {
                            confirm_root = true;
                            continue;
                        }
                    }
                    r => {
                        if c.report(&r) {
                            c.toast(tr!("权限已保存", "Permissions saved"));
                            w2.close();
                        }
                    }
                }
                break;
            }
            busy.set(false);
            b.set_sensitive(true);
        });
    });
    let w3 = win.clone();
    cancel.connect_clicked(move |_| w3.close());
    win.show_all();
}
