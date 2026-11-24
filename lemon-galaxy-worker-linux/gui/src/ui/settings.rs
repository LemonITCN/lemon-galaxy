//! 设置：开机自动启动、登录后显示托盘、远程结束后锁屏、虚拟桌面、登录界面用 X11、自动更新与检查更新。

use super::*;
use crate::control::*;
use crate::tr;
use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

const SIZES: [&str; 5] = ["1280x720", "1600x900", "1920x1080", "2560x1440", "3840x2160"];

fn size_text(id: &str) -> String {
    let t = id.replace('x', " × ");
    match t.split_once('@') {
        Some((a, hz)) => format!("{a}，{hz} Hz"),
        None => t,
    }
}

fn mode_id(m: &DisplayMode) -> String {
    if m.refresh == 60 {
        format!("{}x{}", m.width, m.height)
    } else {
        format!("{}x{}@{}", m.width, m.height, m.refresh)
    }
}

pub fn build(ctx: &Rc<Ctx>) -> (gtk::Widget, Rc<dyn Fn()>) {
    let stack = gtk::Stack::new();
    let (ph, _, _, _) = placeholder(
        "changes-prevent-symbolic",
        tr!("需要管理员权限", "Administrator access required"),
        tr!("只有管理员（sudo、wheel、admin 组）或账号绑定的系统用户能查看设置。", "Only administrators (sudo, wheel or admin group) or the bound system user can view settings."),
    );
    stack.add_named(&ph, "denied");
    let (scroll, content) = page();
    stack.add_named(&scroll, "main");

    // 启动
    let s1 = section(tr!("启动", "Startup"), None);
    let r_boot = row(
        tr!("开机自动启动", "Start at boot"),
        None,
        Some(tr!("开机后自动运行被控服务，没有人登录时也能被远程控制。", "Run the LemonGalaxy service at boot so this computer can be reached even before anyone signs in.")),
    );
    let boot = Toggle::new();
    r_boot.suffix.pack_start(&boot.switch, false, false, 0);
    s1.list.add(&r_boot.row);
    let r_tray = row(
        tr!("登录后显示托盘图标", "Show tray icon after sign-in"),
        None,
        Some(tr!("登录桌面后在托盘里显示 LemonGalaxy 图标：有人在控制这台电脑时图标会变色，也能从这里断开。只影响你自己的桌面。", "Show the LemonGalaxy icon in the tray after you sign in. It changes color while someone controls this computer, and lets you disconnect them. Only affects your own desktop.")),
    );
    let tray = Toggle::new();
    r_tray.suffix.pack_start(&tray.switch, false, false, 0);
    s1.list.add(&r_tray.row);
    content.pack_start(&s1.root, false, false, 0);

    // 安全
    let s2 = section(tr!("安全", "Security"), None);
    let r_lock = row(
        tr!("远程结束后自动锁屏", "Lock the screen after remote sessions"),
        None,
        Some(tr!("别人远程看过桌面后，等这些会话全部结束 10 秒自动锁屏；断线后自动重连不会触发。只用终端、文件的会话不算。", "After someone viewed the desktop remotely, lock the screen 10 seconds after all such sessions end. Automatic reconnects do not trigger it, and terminal-only or file-only sessions do not count.")),
    );
    let lock = Toggle::new();
    r_lock.suffix.pack_start(&lock.switch, false, false, 0);
    s2.list.add(&r_lock.row);
    content.pack_start(&s2.root, false, false, 0);

    // 显示
    let s3 = section(tr!("显示", "Display"), None);
    let r_vd = row(
        tr!("虚拟桌面", "Virtual desktop"),
        None,
        Some(tr!(
            "没有可看的图形会话时（没接显示器、没人登录、远程登录的服务器），有人远程打开屏幕列表，就给这个账号绑定的系统用户开一个虚拟桌面；断开后保留，下次接着用。需要安装 Xvfb 和一个桌面环境（如 Xfce）。",
            "When there is no graphical session to show (no monitor, nobody signed in, or a headless server), open a virtual desktop for the account's system user when someone opens the screen list. It stays after disconnecting. Requires Xvfb and a desktop environment such as Xfce."
        )),
    );
    let vd = Toggle::new();
    let sizes = gtk::ComboBoxText::new();
    for s in SIZES {
        sizes.append(Some(s), &size_text(s));
    }
    sizes.set_active_id(Some("1920x1080"));
    r_vd.suffix.pack_start(&sizes, false, false, 0);
    r_vd.suffix.pack_start(&vd.switch, false, false, 0);
    s3.list.add(&r_vd.row);
    let r_x11 = row(
        tr!("登录界面使用 X11", "Use X11 for the login screen"),
        None,
        Some(tr!(
            "GDM 默认用 Wayland，远程看不到登录界面。打开后，登录界面以及之后登录的桌面都用 X11。重启电脑或注销后生效。",
            "GDM uses Wayland by default, which cannot be shown remotely. When on, the login screen and desktops started from it use X11. Takes effect after you restart or sign out."
        )),
    );
    let x11 = Toggle::new();
    r_x11.suffix.pack_start(&x11.switch, false, false, 0);
    s3.list.add(&r_x11.row);
    content.pack_start(&s3.root, false, false, 0);

    // 更新
    let s4 = section(tr!("更新", "Updates"), None);
    let r_au = row(
        tr!("自动更新", "Update automatically"),
        None,
        Some(tr!("服务每 6 小时用已登录的账号检查一次新版本，发布签名和 SHA-256 都校验通过才安装，装好后服务自动重启。", "The service checks for a new version every 6 hours using a signed-in account, installs it only if the release signature and SHA-256 verify, then restarts itself.")),
    );
    let au = Toggle::new();
    r_au.suffix.pack_start(&au.switch, false, false, 0);
    s4.list.add(&r_au.row);
    let r_ver = row(tr!("版本", "Version"), None, None);
    let check = gtk::Button::with_label(tr!("检查更新", "Check for Updates"));
    let update = gtk::Button::with_label(tr!("立即更新", "Update Now"));
    update.style_context().add_class("suggested-action");
    update.show_all();
    update.set_no_show_all(true);
    update.hide();
    let spin = gtk::Spinner::new();
    r_ver.suffix.pack_start(&spin, false, false, 0);
    r_ver.suffix.pack_start(&check, false, false, 0);
    r_ver.suffix.pack_start(&update, false, false, 0);
    s4.list.add(&r_ver.row);
    content.pack_start(&s4.root, false, false, 0);

    // 动作
    let c = ctx.clone();
    boot.on_toggle(move |on| {
        let c = c.clone();
        async move { c.call(Request::SetAutostart { enabled: on }).await.is_some() }
    });
    let c = ctx.clone();
    tray.on_toggle(move |on| {
        let c = c.clone();
        async move {
            match crate::desktop::set_tray_autostart(on) {
                Ok(()) => true,
                Err(e) => {
                    c.error(&e.to_string());
                    false
                }
            }
        }
    });
    let c = ctx.clone();
    lock.on_toggle(move |on| {
        let c = c.clone();
        async move { c.call(Request::SetLockOnEnd { enabled: on }).await.is_some() }
    });
    let c = ctx.clone();
    au.on_toggle(move |on| {
        let c = c.clone();
        async move { c.call(Request::SetAutoUpdate { enabled: on }).await.is_some() }
    });
    let c = ctx.clone();
    x11.on_toggle(move |on| {
        let c = c.clone();
        async move { c.call(Request::SetLoginScreenX11 { enabled: on }).await.is_some() }
    });
    let quiet_sizes = Rc::new(Cell::new(false));
    let (c, sizes2) = (ctx.clone(), sizes.clone());
    vd.on_toggle(move |on| {
        let (c, sizes) = (c.clone(), sizes2.clone());
        async move {
            let v = if on { sizes.active_id().map(|s| s.to_string()).unwrap_or_default() } else { "off".to_string() };
            let ok = c.call(Request::SetHeadlessDisplay { value: v }).await.is_some();
            if ok {
                sizes.set_sensitive(on);
            }
            ok
        }
    });
    let (c, vd2, q) = (ctx.clone(), vd.clone(), quiet_sizes.clone());
    sizes.connect_changed(move |s| {
        if q.get() || !vd2.switch.is_active() {
            return;
        }
        let Some(v) = s.active_id() else { return };
        let c = c.clone();
        spawn(async move {
            let _ = c.call(Request::SetHeadlessDisplay { value: v.to_string() }).await;
        });
    });

    let (c, spin2, update2, r_ver_sub) = (ctx.clone(), spin.clone(), update.clone(), r_ver.subtitle.clone());
    check.connect_clicked(move |b| {
        let (c, spin, update, sub, b) = (c.clone(), spin2.clone(), update2.clone(), r_ver_sub.clone(), b.clone());
        b.set_sensitive(false);
        spin.start();
        spawn(async move {
            let r = c.client.call(Request::CheckUpdate).await;
            spin.stop();
            b.set_sensitive(true);
            let text = match &r {
                Response::UpdateCheck { latest: Some(u), current } => {
                    update.show();
                    if crate::i18n::zh() {
                        format!("发现新版本 {}（当前 {current}）", u.version)
                    } else {
                        format!("Version {} is available (current {current})", u.version)
                    }
                }
                Response::UpdateCheck { latest: None, current } => {
                    update.hide();
                    if crate::i18n::zh() {
                        format!("已是最新版本（{current}）")
                    } else {
                        format!("Up to date ({current})")
                    }
                }
                Response::Error { message, .. } => format!("{}{message}", tr!("检查更新失败：", "Could not check for updates: ")),
                _ => String::new(),
            };
            sub.set_text(&text);
            sub.set_no_show_all(false);
            sub.show();
        });
    });
    let (c, spin2, r_ver_sub) = (ctx.clone(), spin.clone(), r_ver.subtitle.clone());
    update.connect_clicked(move |b| {
        let (c, spin, sub, b) = (c.clone(), spin2.clone(), r_ver_sub.clone(), b.clone());
        b.set_sensitive(false);
        spin.start();
        sub.set_text(tr!("正在下载并校验新版本…", "Downloading and verifying the new version…"));
        spawn(async move {
            let r = c.client.call(Request::ApplyUpdate).await;
            spin.stop();
            b.set_sensitive(true);
            match r {
                Response::Updated { version: Some(v), restarting } => {
                    b.hide();
                    let t = match (crate::i18n::zh(), restarting) {
                        (true, true) => format!("已更新到 {v}，服务正在重启"),
                        (true, false) => format!("已更新到 {v}，重启服务后生效"),
                        (false, true) => format!("Updated to {v}; the service is restarting"),
                        (false, false) => format!("Updated to {v}; restart the service to use it"),
                    };
                    sub.set_text(&t);
                }
                Response::Updated { version: None, .. } => {
                    b.hide();
                    sub.set_text(tr!("已是最新版本", "Up to date"));
                }
                Response::Error { message, .. } => sub.set_text(&format!("{}{message}", tr!("更新失败：", "Update failed: "))),
                _ => {}
            }
        });
    });

    // 读设置
    let c = ctx.clone();
    let stack2 = stack.clone();
    let loading = Rc::new(Cell::new(false));
    let tray2 = tray.clone();
    let r_boot_row = r_boot;
    let r_x11_row = r_x11;
    let r_ver_sub = r_ver.subtitle.clone();
    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        let Some(r) = c.rights() else { return };
        if !r.view_config {
            stack2.set_visible_child_name("denied");
            return;
        }
        stack2.set_visible_child_name("main");
        tray2.set(crate::desktop::tray_autostart());
        if loading.replace(true) {
            return;
        }
        let manage = r.can_manage();
        let (c, loading) = (c.clone(), loading.clone());
        let (boot, lock, au, vd, x11, sizes, q) = (boot.clone(), lock.clone(), au.clone(), vd.clone(), x11.clone(), sizes.clone(), quiet_sizes.clone());
        let (boot_sub, x11_sub, ver_sub) = (r_boot_row.subtitle.clone(), r_x11_row.subtitle.clone(), r_ver_sub.clone());
        let tray = tray2.clone();
        spawn(async move {
            let r = c.client.call(Request::Settings).await;
            loading.set(false);
            let Response::Settings { settings: s } = r else {
                c.report(&r);
                return;
            };
            boot.set(s.autostart.enabled);
            boot.switch.set_sensitive(manage && s.autostart.available);
            set_sub(&boot_sub, if s.autostart.available { "" } else { &s.autostart.detail });
            // 已经打开但这个系统不能再打开（没有 X11 会话）：只允许关
            x11.switch.set_sensitive(manage && s.login_screen_x11.available);
            lock.set(s.lock_on_end);
            au.set(s.auto_update);
            x11.set(s.login_screen_x11.enabled);
            x11.switch.set_sensitive(manage && s.login_screen_x11.available);
            set_sub(&x11_sub, &s.login_screen_x11.detail);
            q.set(true);
            match &s.headless_display {
                Some(m) => {
                    let id = mode_id(m);
                    if !SIZES.contains(&id.as_str()) && sizes.active_id().as_deref() != Some(id.as_str()) {
                        sizes.append(Some(&id), &size_text(&id));
                    }
                    sizes.set_active_id(Some(&id));
                    vd.set(true);
                }
                None => vd.set(false),
            }
            q.set(false);
            sizes.set_sensitive(manage && s.headless_display.is_some());
            for t in [&lock, &au, &vd] {
                t.switch.set_sensitive(manage);
            }
            // 托盘开关只影响自己，任何人都能改
            tray.switch.set_sensitive(true);
            if ver_sub.text().is_empty() {
                let v = c.status.borrow().as_ref().map(|s| s.version.clone()).unwrap_or_default();
                set_sub(&ver_sub, &if crate::i18n::zh() { format!("被控服务 {v}，界面 {}", galaxy_proto::APP_VERSION) } else { format!("Service {v}, app {}", galaxy_proto::APP_VERSION) });
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
    (stack.upcast(), refresh)
}

fn set_sub(l: &gtk::Label, s: &str) {
    l.set_text(s);
    l.set_no_show_all(s.is_empty());
    l.set_visible(!s.is_empty());
}
