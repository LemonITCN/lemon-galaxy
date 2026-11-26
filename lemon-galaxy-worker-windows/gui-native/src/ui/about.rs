//! 关于：界面与服务版本、运行方式、数据目录、控制接口、程序位置、系统；源代码与许可证；卸载。

use super::*;
use crate::setup::{installed_version, running_installed, LICENSE_URL, REPO_URL};
use crate::win::layout::{Align, Panel, Right};

pub fn build(ctx: &Rc<Ctx>, parent: HWND) -> PageView {
    let panel = Panel::new(parent, true);
    let p = panel.hwnd;
    let name = center_label(p, app_name(), Font::Hero, Tone::Normal);
    let tag = center_label(p, tr!("自建的远程控制：这台电脑的被控端", "Self-hosted remote control, worker for this computer"), Font::Normal, Tone::Dim);

    let mk = |title: &str| (label(p, title), value(p, "—"));
    let app_v = mk(tr!("界面版本", "App version"));
    app_v.1.set_text(crate::setup::app_version());
    let svc_v = mk(tr!("被控服务版本", "Service version"));
    let mode = mk(tr!("运行方式", "Runs as"));
    let dir = mk(tr!("数据目录", "Data folder"));
    let pipe = mk(tr!("控制接口", "Control pipe"));
    pipe.1.set_font(Font::Mono);
    let prog = mk(tr!("程序位置", "Program folder"));
    prog.1.set_text(&crate::setup::base_dir().display().to_string());
    let os = mk(tr!("系统", "System"));
    os.1.set_text(&crate::setup::os_name());
    let rows = [app_v, svc_v, mode, dir, pipe, prog, os];

    let src = link(p, &format!("<a>{}</a>", tr!("源代码", "Source code")), REPO_URL);
    let lic = link(p, &format!("<a>{}</a>", tr!("MIT 许可证", "MIT License")), LICENSE_URL);

    let un_h = label_font(p, tr!("卸载", "Uninstall"), Font::Bold);
    let un_l = label(p, tr!("从这台电脑上移除被控端", "Remove the worker from this computer"));
    let un_info = info(
        p,
        tr!(
            "停止并删除被控服务，删除程序文件、开始菜单与托盘自启。账号与设置保留，重新安装后接着用。也可以在「添加或删除程序」里卸载。",
            "Stops and removes the service, the program files, the Start menu entry and the tray start-up entry. Accounts and settings are kept for a later reinstall. You can also uninstall from Add or Remove Programs."
        ),
    );
    let un_btn = button(p, tr!("卸载", "Uninstall"));
    un_btn.set_shield(true);
    // 装过才显示卸载（启动时看一次）
    let can_uninstall = running_installed() || installed_version().is_some();

    panel.set_layout(move |f| {
        f.gap(6);
        let s = px(64);
        f.app_icon(f.x + (f.w - s) / 2, f.y, s);
        f.y += s;
        f.gap(12);
        f.center_text(name, px(500));
        f.gap(4);
        f.center_text(tag, px(500));
        f.gap(20);
        f.group(|g| {
            for (l, v) in rows {
                g.row(l, None, None, &[Right::C(v)]);
            }
        });
        f.gap(14);
        f.line(&[src, lic], Align::Center);
        if can_uninstall {
            f.gap(26);
            f.title(un_h, None, &[]);
            f.gap(8);
            f.group(|g| g.row(un_l, Some(un_info), None, &[Right::C(un_btn)]));
        }
    });
    // 值的宽度跟着文字走，但不能超过行宽
    for (_, v) in rows {
        set_width(v, pref(v).0.min(px(440)));
    }

    let c = ctx.clone();
    on_click(un_btn, move || {
        let data = crate::setup::service_data_dir().display().to_string();
        let ok = c.confirm(
            tr!("卸载 LemonGalaxy 被控端？", "Uninstall LemonGalaxy Worker?"),
            &crate::i18n::fmt(tr!("被控服务会停止并删除，别人再也连不进这台电脑。账号与设置保留在 {0}，重新安装后接着用。", "The service is stopped and removed, so nobody can connect to this computer. Accounts and settings are kept in {0} for a later reinstall."), &[&data]),
            tr!("卸载", "Uninstall"),
            true,
        );
        if !ok {
            return;
        }
        let pid = std::process::id().to_string();
        let c2 = c.clone();
        c.run_elevated("uninstall", vec!["--uninstall".into(), "--confirmed".into(), "--keep".into(), pid], move |ok| {
            if !ok {
                return;
            }
            crate::win::dialog::notice(c2.parent(), app_name(), tr!("LemonGalaxy 被控端已卸载。", "LemonGalaxy Worker has been uninstalled."), "", tr!("确定", "OK"), false);
            c2.quit();
        });
    });

    let c = ctx.clone();
    let pw = Rc::downgrade(&panel);
    ctx.on_status(move |s| {
        svc_v.1.set_text(&s.version);
        let m = match s.service.mode {
            ServiceMode::System => tr!("系统服务", "System service"),
            ServiceMode::User => tr!("用户服务", "User service"),
            ServiceMode::Foreground => tr!("前台运行", "Foreground"),
        };
        mode.1.set_text(&crate::i18n::fmt(tr!("{0}（{1}）", "{0} ({1})"), &[m, &s.service.user]));
        dir.1.set_text(&s.data_dir);
        pipe.1.set_text(&c.client.pipe_name().unwrap_or_else(|| "—".into()));
        for (_, v) in rows {
            set_width(v, pref_natural(v).min(px(440)));
        }
        if let Some(p) = pw.upgrade() {
            p.relayout_soon();
        }
    });
    PageView { panel, refresh: None }
}

/// 只读文字框按文字算的宽度（不管固定宽度）
fn pref_natural(c: Ctl) -> i32 {
    measure(c.get_font(), &c.text(), 10_000, false).0 + px(6)
}
