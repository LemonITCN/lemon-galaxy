//! 登录向导：服务器地址、用户名、密码 → 两步验证码 → 首次登录设新密码 → 选系统用户（root 二次确认）。

use super::*;
use crate::control::*;
use crate::tr;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Creds,
    Totp,
    NewPassword,
    SystemUser,
}

struct Wizard {
    ctx: Rc<Ctx>,
    win: gtk::Window,
    stack: gtk::Stack,
    next: gtk::Button,
    spinner: gtk::Spinner,
    error: gtk::Label,
    step: Cell<Step>,
    server: gtk::Entry,
    user: gtk::Entry,
    pass: gtk::Entry,
    totp: gtk::Entry,
    totp_hint: gtk::Label,
    new1: gtk::Entry,
    new2: gtk::Entry,
    newpw_hint: gtk::Label,
    sysuser: gtk::ComboBoxText,
    sysuser_hint: gtk::Label,
    replace_server: Cell<bool>,
    superuser: RefCell<String>,
    busy: Cell<bool>,
}

fn field(grid: &gtk::Grid, row: i32, title: &str, w: &impl IsA<gtk::Widget>, info: Option<&str>) {
    let l = gtk::Label::new(Some(title));
    l.set_xalign(1.0);
    l.style_context().add_class("dim-label");
    grid.attach(&l, 0, row, 1, 1);
    w.set_hexpand(true);
    grid.attach(w, 1, row, 1, 1);
    if let Some(i) = info {
        grid.attach(&info_button(i), 2, row, 1, 1);
    }
}

fn grid() -> gtk::Grid {
    let g = gtk::Grid::new();
    g.set_row_spacing(10);
    g.set_column_spacing(12);
    g
}

fn entry(secret: bool) -> gtk::Entry {
    let e = gtk::Entry::new();
    e.set_activates_default(true);
    if secret {
        e.set_visibility(false);
        e.set_input_purpose(gtk::InputPurpose::Password);
    }
    e
}

pub fn open(ctx: &Rc<Ctx>) {
    let win = gtk::Window::new(gtk::WindowType::Toplevel);
    win.set_modal(true);
    win.set_transient_for(ctx.parent().as_ref());
    win.set_destroy_with_parent(true);
    win.set_default_size(480, -1);
    win.set_resizable(false);
    win.set_type_hint(gtk::gdk::WindowTypeHint::Dialog);
    win.set_position(gtk::WindowPosition::CenterOnParent);
    let header = gtk::HeaderBar::new();
    header.set_title(Some(tr!("登录账号", "Sign In")));
    let cancel = gtk::Button::with_label(tr!("取消", "Cancel"));
    header.pack_start(&cancel);
    let next = gtk::Button::with_label(tr!("登录", "Sign In"));
    next.style_context().add_class("suggested-action");
    next.set_can_default(true);
    header.pack_end(&next);
    let spinner = gtk::Spinner::new();
    header.pack_end(&spinner);
    win.set_titlebar(Some(&header));

    let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    body.set_margin_top(22);
    body.set_margin_bottom(22);
    body.set_margin_start(24);
    body.set_margin_end(24);
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::SlideLeft);
    stack.set_vhomogeneous(false);
    stack.set_interpolate_size(true);

    // 第一步：服务器、用户名、密码
    let g = grid();
    let server = entry(false);
    server.set_placeholder_text(Some(tr!("主机:端口#sha256-指纹", "host:port#sha256-fingerprint")));
    if let Some(s) = ctx.status.borrow().as_ref() {
        server.set_text(&s.server);
    }
    field(
        &g,
        0,
        tr!("服务器", "Server"),
        &server,
        Some(tr!(
            "在管理后台「服务器信息」页复制完整地址，格式为 主机:端口#sha256-指纹。被控端会固定这个指纹，服务器证书被替换时拒绝连接。一台电脑上的账号必须在同一台服务器上。",
            "Copy the full address from the Server Info page of the admin console (host:port#sha256-fingerprint). The fingerprint is pinned, so a replaced server certificate is rejected. All accounts on this computer must use the same server."
        )),
    );
    let user = entry(false);
    field(&g, 1, tr!("用户名", "Username"), &user, None);
    let pass = entry(true);
    field(&g, 2, tr!("密码", "Password"), &pass, None);
    stack.add_named(&g, "creds");

    // 两步验证
    let tb = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let totp_hint = wrap_label("", &[]);
    tb.pack_start(&totp_hint, false, false, 0);
    let g = grid();
    let totp = entry(false);
    totp.set_input_purpose(gtk::InputPurpose::Digits);
    totp.set_max_length(8);
    field(&g, 0, tr!("验证码", "Code"), &totp, None);
    tb.pack_start(&g, false, false, 0);
    stack.add_named(&tb, "totp");

    // 首次登录设新密码
    let nb = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let newpw_hint = wrap_label("", &[]);
    nb.pack_start(&newpw_hint, false, false, 0);
    let g = grid();
    let new1 = entry(true);
    field(
        &g,
        0,
        tr!("新密码", "New password"),
        &new1,
        Some(tr!(
            "至少 8 位。新密码只在本机参与 OPAQUE 计算，服务器和管理员都无法得知。忘记密码只能请管理员重置，重置后要在每台设备上重新登录。",
            "At least 8 characters. The password only takes part in OPAQUE on this computer, so neither the server nor administrators can learn it. If you forget it, an administrator must reset it and you sign in again on every device."
        )),
    );
    let new2 = entry(true);
    field(&g, 1, tr!("再次输入", "Confirm"), &new2, None);
    nb.pack_start(&g, false, false, 0);
    stack.add_named(&nb, "newpw");

    // 选系统用户
    let ub = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let sysuser_hint = wrap_label("", &[]);
    ub.pack_start(&sysuser_hint, false, false, 0);
    let g = grid();
    let sysuser = gtk::ComboBoxText::new();
    field(
        &g,
        0,
        tr!("系统用户", "System user"),
        &sysuser,
        Some(tr!(
            "这个账号远程打开的终端、执行的命令、读写的文件都以这个系统用户的身份进行，只有这个用户的权限。以后可以在「账号与权限」里修改。",
            "Remote terminals, commands and file access for this account run as this system user, with only that user's permissions. You can change it later under Accounts."
        )),
    );
    ub.pack_start(&g, false, false, 0);
    stack.add_named(&ub, "sysuser");

    body.pack_start(&stack, false, false, 0);
    let error = wrap_label("", &["lg-small"]);
    error.style_context().add_class("error");
    error.set_no_show_all(true);
    body.pack_start(&error, false, false, 0);
    win.add(&body);

    let w = Rc::new(Wizard {
        ctx: ctx.clone(),
        win: win.clone(),
        stack,
        next: next.clone(),
        spinner,
        error,
        step: Cell::new(Step::Creds),
        server,
        user,
        pass,
        totp,
        totp_hint,
        new1,
        new2,
        newpw_hint,
        sysuser,
        sysuser_hint,
        replace_server: Cell::new(false),
        superuser: RefCell::new("root".into()),
        busy: Cell::new(false),
    });
    let w2 = w.clone();
    next.connect_clicked(move |_| w2.clone().submit());
    let w2 = w.clone();
    cancel.connect_clicked(move |_| w2.close());
    let w2 = w.clone();
    win.connect_delete_event(move |_, _| {
        let c = w2.ctx.clone();
        spawn(async move {
            let _ = c.client.call(Request::LoginCancel).await;
        });
        gtk::glib::Propagation::Proceed
    });
    win.set_default(Some(&next));
    win.show_all();
    if w.server.text().is_empty() {
        w.server.grab_focus();
    } else {
        w.user.grab_focus();
    }
}

impl Wizard {
    fn close(&self) {
        let c = self.ctx.clone();
        spawn(async move {
            let _ = c.client.call(Request::LoginCancel).await;
        });
        self.win.close();
    }

    fn set_busy(&self, b: bool) {
        self.busy.set(b);
        self.next.set_sensitive(!b);
        if b {
            self.spinner.start();
        } else {
            self.spinner.stop();
        }
    }

    fn show_error(&self, m: &str) {
        self.error.set_text(m);
        self.error.set_visible(!m.is_empty());
    }

    fn go(&self, s: Step) {
        self.step.set(s);
        self.show_error("");
        let (name, button) = match s {
            Step::Creds => ("creds", tr!("登录", "Sign In")),
            Step::Totp => ("totp", tr!("验证", "Verify")),
            Step::NewPassword => ("newpw", tr!("设置并登录", "Set and Sign In")),
            Step::SystemUser => ("sysuser", tr!("完成", "Done")),
        };
        self.stack.set_visible_child_name(name);
        self.next.set_label(button);
        match s {
            Step::Creds => self.pass.grab_focus(),
            Step::Totp => self.totp.grab_focus(),
            Step::NewPassword => self.new1.grab_focus(),
            Step::SystemUser => self.sysuser.grab_focus(),
        }
    }

    fn submit(self: Rc<Self>) {
        if self.busy.get() {
            return;
        }
        let req = match self.step.get() {
            Step::Creds | Step::Totp => {
                let totp = (self.step.get() == Step::Totp).then(|| self.totp.text().to_string());
                if self.user.text().trim().is_empty() || self.pass.text().is_empty() {
                    self.show_error(tr!("请填写用户名和密码。", "Enter your username and password."));
                    return;
                }
                Request::Login { server: self.server.text().trim().to_string(), username: self.user.text().trim().to_string(), password: self.pass.text().to_string(), totp, replace_server: self.replace_server.get() }
            }
            Step::NewPassword => {
                let (a, b) = (self.new1.text().to_string(), self.new2.text().to_string());
                if a.chars().count() < 8 {
                    self.show_error(tr!("新密码至少 8 位。", "The new password must be at least 8 characters."));
                    return;
                }
                if a != b {
                    self.show_error(tr!("两次输入不一致。", "The passwords do not match."));
                    return;
                }
                Request::LoginSetPassword { new_password: a }
            }
            Step::SystemUser => {
                let Some(u) = self.sysuser.active_text() else { return };
                Request::LoginFinish { system_user: u.to_string(), confirm_root: false }
            }
        };
        spawn(async move { self.send(req).await });
    }

    async fn send(self: Rc<Self>, req: Request) {
        self.set_busy(true);
        let mut req = req;
        loop {
            let r = self.ctx.client.call(req.clone()).await;
            match r {
                Response::Login { step: LoginStep::NeedTotp } => {
                    let name = self.user.text().to_string();
                    self.totp_hint.set_text(&if crate::i18n::zh() { format!("{name} 开启了两步验证。输入验证器应用里的 6 位数字。") } else { format!("{name} uses two-factor authentication. Enter the 6-digit code from your authenticator app.") });
                    self.go(Step::Totp);
                }
                Response::Login { step: LoginStep::ConfirmServerChange { current_server, accounts } } => {
                    let body = if crate::i18n::zh() {
                        format!("这台电脑上已经登录了 {accounts} 个账号，它们在服务器 {current_server} 上。换到新服务器会退出这些账号。")
                    } else {
                        format!("{accounts} account(s) on server {current_server} are signed in on this computer. Switching servers signs them out.")
                    };
                    if confirm(Some(self.win.clone()), tr!("更换服务器？", "Switch servers?"), &body, tr!("更换并登录", "Switch and Sign In"), true).await {
                        self.replace_server.set(true);
                        if let Request::Login { replace_server, .. } = &mut req {
                            *replace_server = true;
                        }
                        continue;
                    }
                }
                Response::Login { step: LoginStep::NeedNewPassword { username, .. } } => {
                    self.newpw_hint.set_text(&if crate::i18n::zh() { format!("{username} 是首次登录，需要设置自己的密码。") } else { format!("This is the first sign-in for {username}. Set your own password.") });
                    self.go(Step::NewPassword);
                }
                Response::Login { step: LoginStep::ChooseSystemUser { username, display_name, default_user, users, can_choose, superuser } } => {
                    self.sysuser.remove_all();
                    for u in &users {
                        self.sysuser.append(Some(u), u);
                    }
                    self.sysuser.set_active_id(Some(&default_user));
                    if self.sysuser.active().is_none() {
                        self.sysuser.set_active(Some(0));
                    }
                    self.sysuser.set_sensitive(can_choose);
                    *self.superuser.borrow_mut() = superuser;
                    let who = if display_name.is_empty() { username } else { format!("{display_name}（{username}）") };
                    self.sysuser_hint.set_text(&match (crate::i18n::zh(), can_choose) {
                        (true, true) => format!("已验证 {who}。最后选一下：这个账号远程操作这台电脑时以哪个系统用户运行。"),
                        (true, false) => format!("已验证 {who}。被控服务以普通用户运行，这个账号远程操作这台电脑时也以 {default_user} 运行。"),
                        (false, true) => format!("Signed in as {who}. Last step: choose which system user this account runs as on this computer."),
                        (false, false) => format!("Signed in as {who}. The service runs as a regular user, so this account also runs as {default_user}."),
                    });
                    self.go(Step::SystemUser);
                }
                Response::Login { step: LoginStep::Done { username, .. } } => {
                    self.set_busy(false);
                    self.ctx.toast(&if crate::i18n::zh() { format!("已登录 {username}") } else { format!("Signed in as {username}") });
                    self.win.close();
                    return;
                }
                Response::Error { code: ErrorCode::ConfirmRoot, message } => {
                    if confirm(Some(self.win.clone()), tr!("以 root 身份运行？", "Run as root?"), &message, tr!("仍然使用 root", "Use root Anyway"), true).await {
                        if let Request::LoginFinish { confirm_root, .. } = &mut req {
                            *confirm_root = true;
                        }
                        continue;
                    }
                }
                Response::Error { code, message } => {
                    // 登录已失效：回到第一步
                    if code == ErrorCode::Invalid && matches!(req, Request::LoginFinish { .. } | Request::LoginSetPassword { .. }) && self.step.get() != Step::Creds && message.contains(tr!("重新登录", "start again")) {
                        self.go(Step::Creds);
                    }
                    if code == ErrorCode::InvalidCredentials {
                        self.go(Step::Creds);
                        self.pass.set_text("");
                    }
                    self.show_error(&message);
                }
                _ => {}
            }
            break;
        }
        self.set_busy(false);
    }
}
