//! 登录向导（模态对话框）：服务器地址、用户名、密码 → 两步验证码 → 首次登录设新密码 → 选系统用户（选 SYSTEM 二次确认）；
//! 换服务器确认。与 Linux 版 ui/login.rs、WPF 版 Login 同一套流程。

use super::*;
use crate::win::dialog::{Dialog, IDCANCEL_, IDOK_};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Creds,
    Totp,
    NewPassword,
    SystemUser,
}

struct Wizard {
    ctx: Rc<Ctx>,
    d: Rc<Dialog>,
    step: Cell<Step>,
    busy: Cell<bool>,
    replace_server: Cell<bool>,
    next: Ctl,
    error: Ctl,
    hint: Ctl,
    server: Ctl,
    user: Ctl,
    pass: Ctl,
    totp: Ctl,
    new1: Ctl,
    new2: Ctl,
    sysuser: Ctl,
}

pub fn open(ctx: &Rc<Ctx>, username: Option<&str>) {
    let d = Dialog::new(ctx.parent(), tr!("登录账号", "Sign In"), 420, ctx.parent().is_some(), false);
    let p = d.panel.hwnd;
    let lbl = |t: &str| {
        let c = create("STATIC", t, SS_NOPREFIX_ | 0x2 /* SS_RIGHT */, 0, p, None);
        set_tone(c, Tone::Dim);
        c
    };
    let server_l = lbl(tr!("服务器", "Server"));
    let server = edit(p, &ctx.status.borrow().as_ref().map(|s| s.server.clone()).unwrap_or_default(), false);
    server.set_cue(tr!("主机:端口#sha256-指纹", "host:port#sha256-fingerprint"));
    server.set_font(Font::Mono);
    let server_info = info(
        p,
        tr!(
            "在管理后台「服务器信息」页复制完整地址，格式为 主机:端口#sha256-指纹。被控端会固定这个指纹，服务器证书被替换时拒绝连接。一台电脑上的账号必须在同一台服务器上。",
            "Copy the full address from the Server Info page of the admin console (host:port#sha256-fingerprint). The fingerprint is pinned, so a replaced server certificate is rejected. All accounts on this computer must use the same server."
        ),
    );
    let user_l = lbl(tr!("用户名", "Username"));
    let user = edit(p, username.unwrap_or(""), false);
    let pass_l = lbl(tr!("密码", "Password"));
    let pass = edit(p, "", true);
    let hint = label(p, "");
    let totp_l = lbl(tr!("验证码", "Code"));
    let totp = edit(p, "", false);
    totp.set_limit(8);
    totp.set_font(Font::Mono);
    let new1_l = lbl(tr!("新密码", "New password"));
    let new1 = edit(p, "", true);
    let new1_info = info(
        p,
        tr!(
            "至少 8 位。新密码只在本机参与 OPAQUE 计算，服务器和管理员都无法得知。忘记密码只能请管理员重置，重置后要在每台设备上重新登录。",
            "At least 8 characters. The password only takes part in OPAQUE on this computer, so neither the server nor administrators can learn it. If you forget it, an administrator must reset it and you sign in again on every device."
        ),
    );
    let new2_l = lbl(tr!("再次输入", "Confirm"));
    let new2 = edit(p, "", true);
    let sys_l = lbl(tr!("系统用户", "System user"));
    let sysuser = combo(p, &[]);
    let sys_info = info(
        p,
        tr!(
            "这个账号远程打开的终端、执行的命令、读写的文件都以这个 Windows 账号的身份进行，只有它的权限。以后可以在「账号与权限」里修改。",
            "Remote terminals, commands and file access for this account run as this Windows account, with only its permissions. You can change it later under Accounts."
        ),
    );
    let error = label_tone(p, "", Tone::Error);
    let next = button_id(p, tr!("登录", "Sign In"), IDOK_, true);
    let cancel = button_id(p, tr!("取消", "Cancel"), IDCANCEL_, false);
    let spinner = marquee(p);
    set_width(spinner, px(90));
    next.set_shield(ctx.needs_elevation());

    let w = Rc::new(Wizard { ctx: ctx.clone(), d: d.clone(), step: Cell::new(Step::Creds), busy: Cell::new(false), replace_server: Cell::new(false), next, error, hint, server, user, pass, totp, new1, new2, sysuser });

    let lw = [server_l, user_l, pass_l, totp_l, new1_l, new2_l, sys_l].iter().map(|c| pref(*c).0).max().unwrap_or(px(70)) + px(4);
    let ww = Rc::downgrade(&w);
    d.set_layout(move |f| {
        let Some(w) = ww.upgrade() else { return };
        match w.step.get() {
            Step::Creds => {
                f.field(server_l, lw, server, Some(server_info));
                f.gap(10);
                f.field(user_l, lw, user, None);
                f.gap(10);
                f.field(pass_l, lw, pass, None);
            }
            Step::Totp => {
                f.text(hint);
                f.gap(14);
                f.field(totp_l, lw, totp, None);
            }
            Step::NewPassword => {
                f.text(hint);
                f.gap(14);
                f.field(new1_l, lw, new1, Some(new1_info));
                f.gap(10);
                f.field(new2_l, lw, new2, None);
            }
            Step::SystemUser => {
                f.text(hint);
                f.gap(14);
                f.field(sys_l, lw, sysuser, Some(sys_info));
            }
        }
        if !error.text().is_empty() {
            f.gap(12);
            f.text(error);
        }
        let left: Vec<Ctl> = if w.busy.get() { vec![spinner] } else { vec![] };
        f.footer(&[next, cancel], &left);
    });

    let w2 = w.clone();
    on_click(next, move || w2.clone().submit());
    let w2 = w.clone();
    d.on_ok(move || w2.clone().submit());
    let w2 = w.clone();
    on_click(cancel, move || w2.close());
    let w2 = Rc::downgrade(&w);
    d.on_cancel(move || {
        if let Some(w) = w2.upgrade() {
            w.close();
        }
    });
    d.show();
    if server.text().is_empty() {
        server.focus();
    } else if user.text().is_empty() {
        user.focus();
    } else {
        pass.focus();
    }
    // 对话框关掉前一直活着：回调（按钮）里持有它
    std::mem::drop(w);
}

impl Wizard {
    fn close(&self) {
        self.ctx.client.send(Request::LoginCancel);
        self.d.close();
    }

    fn set_busy(&self, b: bool) {
        self.busy.set(b);
        self.next.enable(!b);
        self.d.panel.relayout();
    }

    fn show_error(&self, m: &str) {
        self.error.set_text(m);
        self.d.fit();
    }

    fn go(&self, s: Step) {
        self.step.set(s);
        self.error.set_text("");
        self.next.set_text(match s {
            Step::Creds => tr!("登录", "Sign In"),
            Step::Totp => tr!("验证", "Verify"),
            Step::NewPassword => tr!("设置并登录", "Set and Sign In"),
            Step::SystemUser => tr!("完成", "Done"),
        });
        self.d.fit();
        let f = match s {
            Step::Creds => self.pass,
            Step::Totp => self.totp,
            Step::NewPassword => self.new1,
            Step::SystemUser => self.sysuser,
        };
        f.focus();
        f.select_all();
    }

    fn submit(self: Rc<Self>) {
        if self.busy.get() || self.d.is_closed() {
            return;
        }
        let req = match self.step.get() {
            Step::Creds | Step::Totp => {
                let totp = (self.step.get() == Step::Totp).then(|| self.totp.text().trim().to_string());
                if self.user.text().trim().is_empty() || self.pass.text().is_empty() {
                    self.show_error(tr!("请填写用户名和密码。", "Enter your username and password."));
                    return;
                }
                Request::Login { server: self.server.text().trim().to_string(), username: self.user.text().trim().to_string(), password: self.pass.text(), totp, replace_server: self.replace_server.get() }
            }
            Step::NewPassword => {
                let (a, b) = (self.new1.text(), self.new2.text());
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
                let Some(u) = self.sysuser.selected_id() else { return };
                Request::LoginFinish { system_user: u, confirm_root: false }
            }
        };
        self.set_busy(true);
        self.send(req);
    }

    fn send(self: Rc<Self>, req: Request) {
        let me = self.clone();
        let again = req.clone();
        self.ctx.call(req, move |r| {
            if me.d.is_closed() {
                return;
            }
            me.on_reply(again, r);
        });
    }

    fn on_reply(self: Rc<Self>, mut req: Request, r: Response) {
        let owner = Some(self.d.hwnd);
        match r {
            Response::Login { step: LoginStep::NeedTotp } => {
                let name = self.user.text().trim().to_string();
                self.hint.set_text(&crate::i18n::fmt(tr!("{0} 开启了两步验证。输入验证器应用里的 6 位数字。", "{0} uses two-factor authentication. Enter the 6-digit code from your authenticator app."), &[&name]));
                self.go(Step::Totp);
            }
            Response::Login { step: LoginStep::ConfirmServerChange { current_server, accounts } } => {
                let body = crate::i18n::fmt(
                    tr!("这台电脑上已经登录了 {0} 个账号，它们在服务器 {1} 上。换到新服务器会退出这些账号。", "{0} account(s) on server {1} are signed in on this computer. Switching servers signs them out."),
                    &[&accounts.to_string(), server_host(&current_server)],
                );
                if crate::win::dialog::confirm(owner, app_name(), tr!("更换服务器？", "Switch servers?"), &body, tr!("更换并登录", "Switch and Sign In"), tr!("取消", "Cancel"), true) {
                    self.replace_server.set(true);
                    if let Request::Login { replace_server, .. } = &mut req {
                        *replace_server = true;
                    }
                    self.send(req);
                    return;
                }
            }
            Response::Login { step: LoginStep::NeedNewPassword { username, .. } } => {
                self.hint.set_text(&crate::i18n::fmt(tr!("{0} 是首次登录，需要设置自己的密码。", "This is the first sign-in for {0}. Set your own password."), &[&username]));
                self.go(Step::NewPassword);
            }
            Response::Login { step: LoginStep::ChooseSystemUser { username, display_name, default_user, users, can_choose, .. } } => {
                let mut list = users;
                if !list.iter().any(|u| u.eq_ignore_ascii_case(&default_user)) {
                    list.insert(0, default_user.clone());
                }
                let items: Vec<(&str, &str)> = list.iter().map(|u| (u.as_str(), u.as_str())).collect();
                self.sysuser.set_items(&items);
                if !self.sysuser.select_id(&default_user) {
                    self.sysuser.select_index(0);
                }
                self.sysuser.enable(can_choose);
                set_width(self.sysuser, px(240));
                let who = if display_name.is_empty() { username } else { format!("{display_name}（{username}）") };
                self.hint.set_text(&if can_choose {
                    crate::i18n::fmt(tr!("已验证 {0}。最后选一下：这个账号远程操作这台电脑时以哪个 Windows 账号运行。", "Signed in as {0}. Last step: choose which Windows account this account runs as on this computer."), &[&who])
                } else {
                    crate::i18n::fmt(tr!("已验证 {0}。被控服务以普通用户运行，这个账号远程操作这台电脑时也以 {1} 运行。", "Signed in as {0}. The service runs as a regular user, so this account also runs as {1}."), &[&who, &default_user])
                });
                self.go(Step::SystemUser);
            }
            Response::Login { step: LoginStep::Done { username, .. } } => {
                self.ctx.toast(&crate::i18n::fmt(tr!("已登录 {0}", "Signed in as {0}"), &[&username]));
                self.d.close();
                return;
            }
            Response::Error { code: ErrorCode::ConfirmRoot, message } if matches!(req, Request::LoginFinish { confirm_root: false, .. }) => {
                if crate::win::dialog::confirm(owner, app_name(), tr!("以 SYSTEM 身份运行？", "Run as SYSTEM?"), &message, tr!("仍然使用 SYSTEM", "Use SYSTEM Anyway"), tr!("取消", "Cancel"), true) {
                    if let Request::LoginFinish { confirm_root, .. } = &mut req {
                        *confirm_root = true;
                    }
                    self.send(req);
                    return;
                }
            }
            Response::Error { code, message } => {
                // 登录已失效：回到第一步；密码错：清掉密码
                if code == ErrorCode::Invalid && !matches!(req, Request::Login { .. }) && (message.contains("重新登录") || message.contains("start again")) {
                    self.go(Step::Creds);
                }
                if code == ErrorCode::InvalidCredentials {
                    self.pass.set_text("");
                    self.go(Step::Creds);
                }
                self.set_busy(false);
                self.show_error(&message);
                return;
            }
            _ => {}
        }
        self.set_busy(false);
    }
}
