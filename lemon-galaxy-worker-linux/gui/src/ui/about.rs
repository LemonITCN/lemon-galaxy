//! 关于：版本、设备、运行方式、数据目录、源代码与许可证。

use super::*;
use crate::control::*;
use crate::tr;
use gtk::prelude::*;
use std::rc::Rc;

pub fn build(ctx: &Rc<Ctx>) -> gtk::Widget {
    let (scroll, content) = page();
    let head = gtk::Box::new(gtk::Orientation::Vertical, 6);
    head.set_halign(gtk::Align::Center);
    let icon = gtk::Image::from_icon_name(Some(ICON), gtk::IconSize::Dialog);
    icon.set_pixel_size(96);
    head.pack_start(&icon, false, false, 0);
    let name = gtk::Label::new(Some(tr!("LemonGalaxy 被控端", "LemonGalaxy Worker")));
    name.style_context().add_class("lg-hero-title");
    head.pack_start(&name, false, false, 0);
    let tag = gtk::Label::new(Some(tr!("自建的远程控制：这台电脑的被控端", "Self-hosted remote control, worker for this computer")));
    tag.style_context().add_class("dim-label");
    head.pack_start(&tag, false, false, 0);
    content.pack_start(&head, false, false, 0);

    let sec = section("", None);
    let mk = |title: &str| {
        let r = row(title, None, None);
        let v = label("", &["dim-label"]);
        v.set_selectable(true);
        v.set_can_focus(false);
        v.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        v.set_max_width_chars(40);
        r.suffix.pack_start(&v, false, false, 0);
        sec.list.add(&r.row);
        v
    };
    let app_v = mk(tr!("界面版本", "App version"));
    app_v.set_text(galaxy_proto::APP_VERSION);
    let svc_v = mk(tr!("被控服务版本", "Service version"));
    let mode = mk(tr!("运行方式", "Runs as"));
    let dir = mk(tr!("数据目录", "Data folder"));
    let sock = mk(tr!("控制接口", "Control socket"));
    let os = mk(tr!("系统", "System"));
    os.set_text(&os_name());
    content.pack_start(&sec.root, false, false, 0);

    let links = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    links.set_halign(gtk::Align::Center);
    let src = gtk::LinkButton::with_label("https://github.com/LemonITCN/lemon-galaxy", tr!("源代码", "Source code"));
    links.pack_start(&src, false, false, 0);
    let lic = gtk::LinkButton::with_label("https://github.com/LemonITCN/lemon-galaxy/blob/main/LICENSE", tr!("MIT 许可证", "MIT License"));
    links.pack_start(&lic, false, false, 0);
    content.pack_start(&links, false, false, 0);

    let c = ctx.clone();
    ctx.on_status(move |st| {
        svc_v.set_text(&st.version);
        let m = match st.service.mode {
            ServiceMode::System => tr!("系统服务", "System service"),
            ServiceMode::User => tr!("用户服务", "User service"),
            ServiceMode::Foreground => tr!("前台运行", "Foreground"),
        };
        mode.set_text(&format!("{m}（{}）", st.service.user));
        dir.set_text(&st.data_dir);
        sock.set_text(&c.client.socket_path().map(|p| p.display().to_string()).unwrap_or_default());
    });
    scroll.upcast()
}

fn os_name() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string())))
        .unwrap_or_else(|| "Linux".into())
}
