//! 没连上被控服务时的整页：正在连接 / 没装（从解压目录运行：完整安装；已在安装目录：只装服务）/ 没运行（启动服务）/
//! 运行着但连不上（老版本服务没有本机控制接口：装这份新版本）。与 WPF 版 Pages.ServiceDown 同一套判断。

use super::*;
use crate::setup::{install_dir, installed_exe, installed_version, running_installed};
use crate::win::layout::{Align, Decor, Flow, Panel};
use windows::Win32::Foundation::RECT;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

struct CardCtls {
    title: Ctl,
    ver: Ctl,
    info: Ctl,
    loc_label: Ctl,
    loc_value: Ctl,
    auto: Ctl,
    tray: Ctl,
    button: Ctl,
}

pub fn build(ctx: &Rc<Ctx>, parent: HWND) -> Rc<Panel> {
    let panel = Panel::new(parent, true);
    panel.center.set(true);
    panel.max_w.set(px(540));
    let p = panel.hwnd;

    let conn_title = center_label(p, tr!("正在连接被控服务…", "Connecting to the service…"), Font::Hero, Tone::Normal);
    let conn_bar = marquee(p);
    let nopipe = label_tone(p, tr!("服务在运行，但连不上它的本机控制接口", "The service is running but its control interface does not respond"), Tone::Dim);
    let nopipe_info = info(p, tr!("多半是还没有本机控制接口的旧版本服务。用新下载的安装包更新一次即可；也可以在命令行运行 lemon-galaxy-worker update。", "Most likely an older service without the local control interface. Update with a newly downloaded package, or run lemon-galaxy-worker update from a command prompt."));

    let stopped_title = center_label(p, tr!("被控服务没有运行", "The service is not running"), Font::Hero, Tone::Normal);
    let noservice_title = center_label(p, tr!("被控服务还没有安装", "The service is not installed"), Font::Hero, Tone::Normal);
    let down_body = center_label(
        p,
        tr!("被控服务负责联网和远程控制，这个窗口只是它的遥控器。服务停着时别人连不进来。", "The service connects this computer and handles remote control; this window only manages it. Nobody can connect while it is stopped."),
        Font::Normal,
        Tone::Dim,
    );
    let start_btn = button(p, tr!("启动服务", "Start Service"));
    let install_svc_btn = button(p, tr!("安装并启动服务", "Install and Start Service"));

    let busy_text = center_label(p, "", Font::Normal, Tone::Dim);
    let busy_bar = marquee(p);

    // 安装卡片
    let ctls = CardCtls {
        title: label_font(p, "", Font::Hero),
        ver: label_tone(p, "", Tone::Dim),
        info: label_tone(
            p,
            tr!(
                "安装会把程序复制到 Program Files，装上被控服务（开机自动运行，没人登录时也能被远程控制），在开始菜单和「添加或删除程序」里加上入口。账号与设置单独保存，更新、卸载都会保留。需要管理员授权。",
                "Installing copies the program to Program Files, installs the LemonGalaxy service (starts at boot, reachable even before anyone signs in), and adds Start menu and Add or Remove Programs entries. Accounts and settings are stored separately and kept on update or uninstall. Requires administrator authorization."
            ),
            Tone::Dim,
        ),
        loc_label: label(p, tr!("安装位置", "Location")),
        loc_value: value(p, &install_dir().display().to_string()),
        auto: checkbox(p, tr!("开机自动启动被控服务", "Start the service at boot")),
        tray: checkbox(p, tr!("所有用户登录后在托盘显示图标", "Show the tray icon after anyone signs in")),
        button: button(p, tr!("安装", "Install")),
    };
    ctls.auto.set_checked(true);
    ctls.tray.set_checked(true);
    for b in [start_btn, install_svc_btn, ctls.button] {
        b.set_shield(true);
    }
    let (cb_auto, cb_tray, install_btn) = (ctls.auto, ctls.tray, ctls.button);

    // 已装那份的版本（状态变了才重读）
    let installed: Rc<RefCell<Option<Option<String>>>> = Rc::new(RefCell::new(None));

    let c = ctx.clone();
    let inst = installed.clone();
    panel.set_layout(move |f| {
        let busy = c.busy.get();
        if matches!(busy, Some("install") | Some("start")) {
            icon(f, px(64));
            f.gap(18);
            busy_text.set_text(if busy == Some("install") { tr!("正在安装…", "Installing…") } else { tr!("正在启动服务…", "Starting the service…") });
            f.center_text(busy_text, px(420));
            f.gap(12);
            f.line(&[busy_bar], Align::Center);
            return;
        }
        let installed_here = running_installed();
        let ver = inst.borrow_mut().get_or_insert_with(installed_version).clone();
        let same_version = installed_here || ver.as_deref() == Some(crate::setup::app_version());
        let connecting = |f: &mut Flow, no_pipe: bool| {
            icon(f, px(64));
            f.gap(18);
            f.center_text(conn_title, px(460));
            f.gap(14);
            f.line(&[conn_bar], Align::Center);
            if no_pipe {
                f.gap(16);
                let (w, h) = measure(nopipe.get_font(), &nopipe.text(), px(420), true);
                let x = f.x + (f.w - w - px(22)) / 2;
                f.place(nopipe, x, f.y, w, h);
                f.place(nopipe_info, x + w + px(6), f.y + (line_h() - px(16)) / 2, px(16), px(16));
                f.y += h;
            }
        };
        let simple = |f: &mut Flow, title: Ctl, action: Ctl| {
            icon(f, px(64));
            f.gap(18);
            f.center_text(title, px(460));
            f.gap(8);
            f.center_text(down_body, px(440));
            f.gap(20);
            f.line(&[action], Align::Center);
        };
        match c.service.get() {
            ServiceState::Unknown => connecting(f, false),
            ServiceState::Running | ServiceState::Starting => {
                if !c.grace_over.get() || c.ever_connected.get() {
                    connecting(f, false)
                } else if same_version {
                    // 服务在跑却一直连不上：多半是没有本机控制接口的老版本，装这份新版本即可
                    connecting(f, true)
                } else {
                    card(f, &ctls, ver.as_deref());
                }
            }
            ServiceState::Stopped if same_version => simple(f, stopped_title, start_btn),
            ServiceState::NotInstalled if installed_here => simple(f, noservice_title, install_svc_btn),
            _ => card(f, &ctls, ver.as_deref()),
        }
    });

    // 动作
    let c = ctx.clone();
    on_click(start_btn, move || {
        let c2 = c.clone();
        c.run_elevated("start", vec!["--start-service".into()], move |ok| {
            if ok {
                c2.toast(tr!("服务已启动", "Service started"));
            }
        });
    });
    let c = ctx.clone();
    on_click(install_svc_btn, move || {
        let c2 = c.clone();
        c.run_elevated("install", vec!["--install".into()], move |ok| {
            if ok {
                c2.toast(tr!("服务已安装并启动", "Service installed and started"));
            }
        });
    });
    let c = ctx.clone();
    on_click(install_btn, move || {
        let mut args = vec!["--install".to_string()];
        if !cb_auto.checked() {
            args.push("--no-autostart".into());
        }
        if !cb_tray.checked() {
            args.push("--no-tray".into());
        }
        let c2 = c.clone();
        c.run_elevated("install", args, move |ok| {
            if ok {
                launch_installed_and_exit(&c2);
            }
        });
    });
    // 状态变了：已装的版本可能也变了（装好、卸载）
    let inst = installed.clone();
    ctx.on_change(move || {
        *inst.borrow_mut() = None;
    });
    panel
}

fn icon(f: &mut Flow, s: i32) {
    let x = f.x + (f.w - s) / 2;
    f.app_icon(x, f.y, s);
    f.y += s;
}

/// 完整安装（从解压目录运行时）：装到 Program Files、装服务、开始菜单、托盘自启。`upgrade`：已装那份的版本
fn card(f: &mut Flow, ctls: &CardCtls, upgrade: Option<&str>) {
    ctls.title.set_text(if upgrade.is_none() { tr!("安装 LemonGalaxy 被控端", "Install LemonGalaxy Worker") } else { tr!("更新 LemonGalaxy 被控端", "Update LemonGalaxy Worker") });
    let me = crate::setup::app_version();
    ctls.ver.set_text(&match upgrade {
        None => crate::i18n::fmt(tr!("版本 {0}", "Version {0}"), &[me]),
        Some(v) => crate::i18n::fmt(tr!("本机装的是 {0}，这份是 {1}", "Installed {0}, this copy is {1}"), &[if v.is_empty() { "?" } else { v }, me]),
    });
    ctls.button.set_text(if upgrade.is_none() { tr!("安装", "Install") } else { tr!("更新", "Update") });
    let top = f.y;
    let (x0, w0) = (f.x, f.w);
    let pad = px(22);
    f.x += pad;
    f.w -= 2 * pad;
    f.y += pad;
    let s = px(48);
    f.app_icon(f.x, f.y, s);
    let tx = f.x + s + px(14);
    let tw = f.w - s - px(14);
    let (_, th) = measure(font(Font::Hero), &ctls.title.text(), tw, true);
    let (_, vh) = measure(font(Font::Normal), &ctls.ver.text(), tw, true);
    let ty = f.y + ((s - th - vh - px(3)) / 2).max(0);
    f.place(ctls.title, tx, ty, tw, th);
    f.place(ctls.ver, tx, ty + th + px(3), tw, vh);
    f.y += s.max(th + vh + px(3));
    f.gap(14);
    f.text(ctls.info);
    f.gap(14);
    f.decor(Decor::Line(f.x, f.x + f.w, f.y));
    f.gap(14);
    let (lw, lh) = pref(ctls.loc_label);
    let (_, vh) = pref(ctls.loc_value);
    let rh = lh.max(vh);
    f.place(ctls.loc_label, f.x, f.y + (rh - lh) / 2, lw, lh);
    f.place(ctls.loc_value, f.x + lw + px(16), f.y + (rh - vh) / 2, f.w - lw - px(16), vh);
    f.y += rh;
    f.gap(12);
    f.line(&[ctls.auto], Align::Left);
    f.gap(6);
    f.line(&[ctls.tray], Align::Left);
    f.gap(18);
    f.line(&[ctls.button], Align::Right);
    f.y += pad;
    f.x = x0;
    f.w = w0;
    f.decor(Decor::Frame(RECT { left: f.x, top, right: f.x + f.w, bottom: f.y }));
}

/// 安装好了：打开装好的那份（普通权限，`--replace` 接替同位置的旧实例），自己退出
pub fn launch_installed_and_exit(ctx: &Rc<Ctx>) {
    if running_installed() {
        return;
    }
    let exe = wide(&installed_exe().display().to_string());
    let dir = wide(&install_dir().display().to_string());
    let args = wide("--replace");
    let op = wide("open");
    let r = unsafe { windows::Win32::UI::Shell::ShellExecuteW(None, pcw(&op), pcw(&exe), pcw(&args), pcw(&dir), SW_SHOWNORMAL) };
    if (r.0 as isize) <= 32 {
        ctx.error(&format!("{}{}", tr!("打不开装好的程序：", "Could not open the installed program: "), installed_exe().display()));
        return;
    }
    ctx.quit();
}
