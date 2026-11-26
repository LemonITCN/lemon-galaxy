//! 设置：开机自动启动被控服务、登录后在托盘显示图标、远程结束后自动锁屏、没有显示器时自动加虚拟屏、
//! 自动更新被控服务 + 检查更新 / 立即更新；我的偏好（有人连入时通知、语言）。与 WPF 版的设置页一致
//! （Linux 专有的「远程连入时自动解锁」「登录界面使用 X11」Windows 不显示）。

use super::*;
use windows::Win32::UI::WindowsAndMessaging::CBN_SELCHANGE;
use crate::win::layout::{Panel, Right};

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

/// 检查更新的状态
#[derive(Clone)]
enum Upd {
    Idle(Option<String>),
    Busy(&'static str),
    Available(String, String),
}

pub fn build(ctx: &Rc<Ctx>, parent: HWND) -> PageView {
    let panel = Panel::new(parent, true);
    let p = panel.hwnd;
    let title = label_font(p, tr!("设置", "Settings"), Font::Title);

    // 启动
    let startup_h = label_font(p, tr!("启动", "Startup"), Font::Bold);
    let boot = checkbox(p, tr!("开机自动启动被控服务", "Start the service at boot"));
    let boot_info = info(p, tr!("被控服务是系统服务（Windows 服务 LemonGalaxyWorker）：开机就运行，没有人登录时也能被远程控制，与界面开不开无关。关掉后服务改为手动启动。", "The service is a Windows service (LemonGalaxyWorker): it starts at boot and can be reached before anyone signs in, whether or not this app is open. When off, the service starts manually."));
    let boot_sub = label_tone(p, "", Tone::Dim);
    let tray = checkbox(p, tr!("登录后在托盘显示图标", "Show tray icon after sign-in"));
    let tray_info = info(p, tr!("登录 Windows 后在托盘里放 LemonGalaxy 图标：有人在控制这台电脑时图标会变色，也能从这里断开。只影响你自己，与被控服务开机启动是两件事。", "Put the LemonGalaxy icon in the tray after you sign in to Windows. It changes color while someone controls this computer and lets you disconnect them. Only affects you, and is separate from starting the service at boot."));
    let denied = label(p, tr!("只有管理员组成员或账号绑定的系统用户能查看被控服务的设置。", "Only members of the Administrators group or the bound system user can view the service settings."));
    let loading = label_tone(p, tr!("正在读取…", "Loading…"), Tone::Dim);

    // 安全
    let sec_h = label_font(p, tr!("安全", "Security"), Font::Bold);
    let lock = checkbox(p, tr!("远程结束后自动锁屏", "Lock the screen after remote sessions"));
    let lock_info = info(p, tr!("别人远程看过桌面后，等这些会话全部结束 10 秒自动锁屏；断线后自动重连不会触发。只用终端、文件的会话不算。", "After someone viewed the desktop remotely, lock the screen 10 seconds after all such sessions end. Automatic reconnects do not trigger it, and terminal-only or file-only sessions do not count."));

    // 显示
    let disp_h = label_font(p, tr!("显示", "Display"), Font::Bold);
    let vd = checkbox(p, tr!("没有显示器时自动加虚拟屏", "Add a virtual display when there is no monitor"));
    let vd_info = info(p, tr!("这台电脑没接显示器（或显示器关着、远程桌面断开后）时，有人远程打开屏幕列表就自动加一块这个分辨率的虚拟屏，最后一个会话结束后删掉。需要 LemonGalaxy 虚拟屏驱动。", "When no monitor is connected (or it is off, or after a Remote Desktop session ends), a virtual display at this resolution is added when someone opens the screen list, and removed after the last session ends. Requires the LemonGalaxy virtual display driver."));
    let items: Vec<(String, String)> = SIZES.iter().map(|s| (s.to_string(), size_text(s))).collect();
    let refs: Vec<(&str, &str)> = items.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let sizes = combo(p, &refs);
    sizes.select_id("1920x1080");
    set_width(sizes, px(170));

    // 更新
    let upd_h = label_font(p, tr!("更新", "Updates"), Font::Bold);
    let au = checkbox(p, tr!("自动更新被控服务", "Update the service automatically"));
    let au_info = info(p, tr!("服务每 6 小时用已登录的账号检查一次新版本，发布签名和 SHA-256 都校验通过才安装，装好后服务自动重启。界面程序不随之更新，下次用新安装包更新。", "The service checks for a new version every 6 hours using a signed-in account, installs it only if the release signature and SHA-256 verify, then restarts itself. This app is not updated with it; update it with a new package."));
    let ver_l = label(p, tr!("版本", "Version"));
    let ver_info = info(p, tr!("检查与更新的是被控服务（lemon-galaxy-worker.exe），由服务自己下载、核对签名并重启。", "Checks and updates the service (lemon-galaxy-worker.exe); the service downloads, verifies the signature and restarts itself."));
    let ver_sub = label_tone(p, "", Tone::Dim);
    let check = button(p, tr!("检查更新", "Check for Updates"));
    let update = button(p, tr!("立即更新", "Update Now"));
    let upd_bar = marquee(p);
    set_width(upd_bar, px(80));

    // 我的偏好
    let me_h = label_font(p, tr!("我的偏好", "My Preferences"), Font::Bold);
    let me_info = info(p, tr!("只影响这台电脑上你自己的界面，不影响被控服务和别的用户。", "Only affects your own app on this computer, not the service or other users."));
    let notify = checkbox(p, tr!("有人连入时发通知", "Notify me when someone connects"));
    let notify_info = info(p, tr!("有人连入、有人请求协助时在屏幕右下角发系统通知。账号的「被控时提示」关掉时，它连入也不通知。", "Show a system notification when someone connects or asks for assistance. Accounts with Notify when controlled turned off do not trigger it."));
    let lang_l = label(p, tr!("语言", "Language"));
    let lang = combo(p, &[("system", tr!("跟随系统", "System")), ("zh-Hans", "简体中文"), ("en", "English")]);
    set_width(lang, px(170));
    {
        let s = ctx.settings.borrow();
        tray.set_checked(s.tray_at_login);
        notify.set_checked(s.notifications);
        if !lang.select_id(&s.language) {
            lang.select_id("system");
        }
    }

    let worker: Rc<RefCell<Option<Settings>>> = Rc::new(RefCell::new(None));
    let upd = Rc::new(RefCell::new(Upd::Idle(None)));

    let c = ctx.clone();
    let w2 = worker.clone();
    let u2 = upd.clone();
    panel.set_layout(move |f| {
        let st = c.status.borrow();
        let Some(s) = st.as_ref() else { return };
        f.title(title, None, &[]);
        f.gap(18);
        f.title(startup_h, None, &[]);
        f.gap(8);
        let w = w2.borrow();
        let view = s.rights.view_config;
        f.group(|g| {
            if let (true, Some(ws)) = (view, w.as_ref()) {
                let sub = (!ws.autostart.available && !ws.autostart.detail.is_empty()).then_some(boot_sub);
                g.row(boot, Some(boot_info), sub, &[]);
            }
            g.row(tray, Some(tray_info), None, &[]);
        });
        if !view {
            f.gap(12);
            f.banner(Tone::Normal, denied, None);
        } else if w.is_none() {
            f.gap(12);
            f.text(loading);
        } else {
            f.gap(22);
            f.title(sec_h, None, &[]);
            f.gap(8);
            f.group(|g| g.row(lock, Some(lock_info), None, &[]));
            f.gap(22);
            f.title(disp_h, None, &[]);
            f.gap(8);
            f.group(|g| g.row(vd, Some(vd_info), None, &[Right::C(sizes)]));
            f.gap(22);
            f.title(upd_h, None, &[]);
            f.gap(8);
            let state = u2.borrow().clone();
            f.group(|g| {
                g.row(au, Some(au_info), None, &[]);
                match state {
                    Upd::Busy(_) => g.row(ver_l, Some(ver_info), Some(ver_sub), &[Right::C(upd_bar)]),
                    Upd::Available(..) => g.row(ver_l, Some(ver_info), Some(ver_sub), &[Right::C(check), Right::C(update)]),
                    Upd::Idle(_) => g.row(ver_l, Some(ver_info), Some(ver_sub), &[Right::C(check)]),
                }
            });
        }
        f.gap(22);
        f.title(me_h, Some(me_info), &[]);
        f.gap(8);
        f.group(|g| {
            g.row(notify, Some(notify_info), None, &[]);
            g.row(lang_l, None, None, &[Right::C(lang)]);
        });
    });

    // 版本那一行的副标题
    let c = ctx.clone();
    let u2 = upd.clone();
    let pw = Rc::downgrade(&panel);
    let show_upd: Rc<dyn Fn(Upd)> = Rc::new(move |u: Upd| {
        let text = match &u {
            Upd::Idle(Some(t)) => t.clone(),
            Upd::Idle(None) => {
                let v = c.status.borrow().as_ref().map(|s| s.version.clone()).unwrap_or_default();
                crate::i18n::fmt(tr!("被控服务 {0}，界面 {1}", "Service {0}, app {1}"), &[&v, crate::setup::app_version()])
            }
            Upd::Busy(t) => t.to_string(),
            Upd::Available(t, _) => t.clone(),
        };
        ver_sub.set_text(&text);
        *u2.borrow_mut() = u;
        if let Some(p) = pw.upgrade() {
            p.relayout();
        }
    });

    // 动作
    req_toggle(ctx, boot, |on| Request::SetAutostart { enabled: on });
    req_toggle(ctx, lock, |on| Request::SetLockOnEnd { enabled: on });
    req_toggle(ctx, au, |on| Request::SetAutoUpdate { enabled: on });
    let c = ctx.clone();
    on_click(tray, move || crate::setup::set_user_tray(&mut c.settings.borrow_mut(), tray.checked()));
    let c = ctx.clone();
    on_click(notify, move || {
        let mut s = c.settings.borrow_mut();
        s.notifications = notify.checked();
        s.save();
    });
    let c = ctx.clone();
    on_command(lang, move |code| {
        if code != CBN_SELCHANGE {
            return;
        }
        let v = lang.selected_id().unwrap_or_else(|| "system".into());
        let changed = {
            let mut s = c.settings.borrow_mut();
            let changed = s.language != v;
            s.language = v.clone();
            s.save();
            changed
        };
        if changed {
            crate::i18n::apply(&v);
            // 整个窗口按新语言重建（在这次消息处理完之后）
            let c2 = c.clone();
            defer(move || c2.rebuild());
        }
    });
    let c = ctx.clone();
    let w2 = worker.clone();
    toggle(vd, move |on, done| {
        let v = if on { sizes.selected_id().unwrap_or_else(|| "1920x1080".into()) } else { "off".into() };
        let (w2, m) = (w2.clone(), c.rights().can_manage());
        c.act(Request::SetHeadlessDisplay { value: v }, move |r| {
            let ok = r.is_some();
            if ok {
                sizes.enable(on && m);
                if let Some(ws) = w2.borrow_mut().as_mut() {
                    ws.headless_display = if on { Some(DisplayMode { width: 0, height: 0, refresh: 60 }) } else { None };
                }
            }
            done(ok);
        });
    });
    let c = ctx.clone();
    on_command(sizes, move |code| {
        if code != CBN_SELCHANGE || !vd.checked() {
            return;
        }
        if let Some(v) = sizes.selected_id() {
            c.act(Request::SetHeadlessDisplay { value: v }, |_| {});
        }
    });
    let c = ctx.clone();
    let su = show_upd.clone();
    on_click(check, move || {
        su(Upd::Busy(tr!("正在检查…", "Checking…")));
        let su = su.clone();
        c.call(Request::CheckUpdate, move |r| match r {
            Response::UpdateCheck { latest: Some(l), current } => su(Upd::Available(crate::i18n::fmt(tr!("发现新版本 {0}（当前 {1}）", "Version {0} is available (current {1})"), &[&l.version, &current]), l.version)),
            Response::UpdateCheck { latest: None, current } => su(Upd::Idle(Some(crate::i18n::fmt(tr!("已是最新版本（{0}）", "Up to date ({0})"), &[&current])))),
            Response::Error { message, .. } => su(Upd::Idle(Some(crate::i18n::fmt(tr!("检查更新失败：{0}", "Could not check for updates: {0}"), &[&message])))),
            _ => su(Upd::Idle(None)),
        });
    });
    let c = ctx.clone();
    let su = show_upd.clone();
    let u2 = upd.clone();
    on_click(update, move || {
        let prev = u2.borrow().clone();
        su(Upd::Busy(tr!("正在下载并校验新版本…", "Downloading and verifying…")));
        let (su, c2) = (su.clone(), c.clone());
        c.call(Request::ApplyUpdate, move |r| match r {
            Response::Updated { version: Some(v), restarting } => su(Upd::Idle(Some(crate::i18n::fmt(
                if restarting { tr!("已更新到 {0}，服务正在重启", "Updated to {0}; the service is restarting") } else { tr!("已更新到 {0}，重启服务后生效", "Updated to {0}; restart the service to use it") },
                &[&v],
            )))),
            Response::Updated { version: None, .. } => {
                let v = c2.status.borrow().as_ref().map(|s| s.version.clone()).unwrap_or_default();
                su(Upd::Idle(Some(crate::i18n::fmt(tr!("已是最新版本（{0}）", "Up to date ({0})"), &[&v]))))
            }
            Response::Error { message, .. } => {
                let text = crate::i18n::fmt(tr!("更新失败：{0}", "Update failed: {0}"), &[&message]);
                match prev {
                    Upd::Available(_, v) => su(Upd::Available(text, v)),
                    _ => su(Upd::Idle(Some(text))),
                }
            }
            _ => su(Upd::Idle(None)),
        });
    });

    // 读被控服务的设置
    let c = ctx.clone();
    let w2 = worker.clone();
    let pw = Rc::downgrade(&panel);
    let loading_now = Rc::new(Cell::new(false));
    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        tray.set_checked(c.settings.borrow().tray_at_login);
        let r = c.rights();
        if !r.view_config || loading_now.replace(true) {
            return;
        }
        let manage = r.can_manage();
        let shield = r.manage == ManageRight::Elevate;
        let (c2, w2, pw, ln) = (c.clone(), w2.clone(), pw.clone(), loading_now.clone());
        c.client.call(Request::Settings, move |r| {
            ln.set(false);
            let Response::Settings { settings: s } = r else {
                c2.report(&r);
                return;
            };
            boot.set_checked(s.autostart.enabled);
            boot.enable(manage && s.autostart.available);
            boot_sub.set_text(&s.autostart.detail);
            lock.set_checked(s.lock_on_end);
            au.set_checked(s.auto_update);
            match &s.headless_display {
                Some(m) => {
                    let id = mode_id(m);
                    if !sizes.select_id(&id) {
                        let mut items: Vec<(String, String)> = SIZES.iter().map(|s| (s.to_string(), size_text(s))).collect();
                        items.push((id.clone(), size_text(&id)));
                        let refs: Vec<(&str, &str)> = items.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
                        sizes.set_items(&refs);
                        sizes.select_id(&id);
                    }
                    vd.set_checked(true);
                }
                None => vd.set_checked(false),
            }
            sizes.enable(manage && s.headless_display.is_some());
            for t in [lock, au, vd] {
                t.enable(manage);
            }
            for t in [boot, lock, au, vd] {
                t.set_shield(shield);
            }
            update.enable(manage);
            update.set_shield(shield);
            *w2.borrow_mut() = Some(s);
            if let Some(p) = pw.upgrade() {
                p.relayout();
            }
        });
    });
    // 配置被改过（界面或命令行）、权限变了：重新读；服务版本变了：刷新版本那一行
    let sig = Rc::new(Cell::new(None::<(u64, bool, bool)>));
    let r2 = refresh.clone();
    let su = show_upd.clone();
    let u2 = upd.clone();
    let last_ver = Rc::new(RefCell::new(String::new()));
    let pw = Rc::downgrade(&panel);
    ctx.on_status(move |s| {
        let sg = (s.config_rev, s.rights.view_config, s.rights.can_manage());
        if sig.replace(Some(sg)) != Some(sg) {
            r2();
        }
        if *last_ver.borrow() != s.version {
            *last_ver.borrow_mut() = s.version.clone();
            if matches!(*u2.borrow(), Upd::Idle(None)) {
                su(Upd::Idle(None));
            }
        }
        if let Some(p) = pw.upgrade() {
            p.relayout_soon();
        }
    });
    PageView { panel, refresh: Some(refresh) }
}
