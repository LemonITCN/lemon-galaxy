//! 程序：参数与子命令、单实例（再打开就把窗口切到前面）、主窗口、托盘、连入通知、协助请求、换语言时重建窗口。

use crate::client::Signal;
use crate::control::*;
use crate::settings::{log_error, log_info, AppSettings};
use crate::tr;
use crate::tray::Tray;
use crate::ui::assist::Asks;
use crate::ui::window::{Hooks, MainWindow};
use crate::ui::{app_name, short_device, who, Ctx, Page};
use crate::win::*;
use std::cell::RefCell;
use std::rc::Rc;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
use windows::Win32::System::Threading::{CreateEventW, CreateMutexW, ReleaseMutex, SetEvent, WaitForMultipleObjects, WaitForSingleObject, INFINITE};
use windows::Win32::UI::Controls::{InitCommonControlsEx, ICC_LINK_CLASS, ICC_PROGRESS_CLASS, ICC_STANDARD_CLASSES, ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX};
use windows::Win32::UI::WindowsAndMessaging::PostQuitMessage;

pub fn main() -> i32 {
    std::panic::set_hook(Box::new(|info| {
        log_error(&format!("程序出错：{info}"));
    }));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).filter(|v| !v.starts_with("--")).cloned();
    let has = |name: &str| args.iter().any(|a| a == name);

    let settings = AppSettings::load();
    crate::i18n::apply(&settings.language);
    crate::settings::init_log();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    }

    // 以管理员身份运行的子命令：没有窗口，做完就退
    let result = arg("--result");
    if let Some(key) = arg("--elevate") {
        return crate::elevation::elevate_main(&key, &arg("--pipe").unwrap_or_else(|| SYSTEM_PIPE.to_string()), result.as_deref());
    }
    if has("--install") {
        log_info(&format!("安装：{}", args.join(" ")));
        return crate::setup::install_main(!has("--no-autostart"), !has("--no-tray"), result.as_deref());
    }
    if has("--start-service") {
        return crate::setup::start_service_main(result.as_deref());
    }
    if has("--uninstall") {
        let keep = arg("--keep").and_then(|v| v.parse().ok()).unwrap_or(0);
        return uninstall(result.as_deref(), has("--confirmed"), keep);
    }

    let tray_only = has("--tray");
    if tray_only && !settings.tray_at_login {
        return 0;
    }
    let Some(single) = SingleInstance::acquire(has("--replace"), tray_only) else { return 0 };
    crate::setup::clean_old_files();
    init_ui();
    log_info(&format!("界面 {} 启动（{}）", galaxy_proto::APP_VERSION, crate::setup::os_name()));
    let gui = Gui::start(settings, tray_only);
    single.listen();
    let code = run_loop();
    if let Some(t) = gui.tray.borrow_mut().take() {
        t.remove();
    }
    single.release();
    code
}

/// 窗口要的系统准备：DPI、公共控件、字体、窗口类、调度窗口
fn init_ui() {
    crate::win::compat::set_dpi_aware();
    let icc = INITCOMMONCONTROLSEX { dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_WIN95_CLASSES | ICC_STANDARD_CLASSES | ICC_LINK_CLASS | ICC_PROGRESS_CLASS };
    unsafe {
        let _ = InitCommonControlsEx(&icc);
    }
    init_fonts();
    register_classes();
    init_dispatcher();
}

/// 卸载。「添加或删除程序」里调用时先确认（没提权就自己提权再做）；界面「关于」页调用时已确认过（--confirmed），
/// 并带上界面自己的进程号（--keep），卸载时不关它，由它自己提示后退出。
fn uninstall(result: Option<&str>, confirmed: bool, keep: u32) -> i32 {
    if confirmed {
        return crate::setup::uninstall_main(result, keep);
    }
    init_ui();
    let data = crate::setup::service_data_dir().display().to_string();
    let ok = crate::win::dialog::confirm(
        None,
        app_name(),
        tr!("卸载 LemonGalaxy 被控端？", "Uninstall LemonGalaxy Worker?"),
        &crate::i18n::fmt(tr!("被控服务会停止并删除，别人再也连不进这台电脑。账号与设置保留在 {0}，重新安装后接着用。", "The service is stopped and removed, so nobody can connect to this computer. Accounts and settings are kept in {0} for a later reinstall."), &[&data]),
        tr!("卸载", "Uninstall"),
        tr!("取消", "Cancel"),
        true,
    );
    if !ok {
        return 1;
    }
    let done = tr!("LemonGalaxy 被控端已卸载。", "LemonGalaxy Worker has been uninstalled.");
    let ok_text = tr!("确定", "OK");
    if crate::elevation::is_admin() {
        let code = crate::setup::uninstall_main(result, keep);
        if code == 0 {
            crate::win::dialog::notice(None, app_name(), done, "", ok_text, false);
        } else {
            crate::win::dialog::notice(None, app_name(), tr!("没有完成", "Did not complete"), "", ok_text, true);
        }
        return code;
    }
    let pid = std::process::id().to_string();
    let r = crate::elevation::run_blocking(&["--uninstall".into(), "--confirmed".into(), "--keep".into(), pid], 0);
    if r.ok {
        crate::win::dialog::notice(None, app_name(), done, "", ok_text, false);
        0
    } else {
        if !r.cancelled {
            crate::win::dialog::notice(None, app_name(), tr!("没有完成", "Did not complete"), &r.error.unwrap_or_default(), ok_text, true);
        }
        1
    }
}

// ---------------- 单实例 ----------------

/// 按程序位置区分（解压目录里的新版本与已安装的旧版本可以同时开，安装后由新的那份 --replace 接替）
struct SingleInstance {
    mutex: isize,
    show: isize,
    quit: isize,
}

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn named(prefix: &str, tag: &str) -> Vec<u16> {
    wide(&format!(r"Local\LemonGalaxyWorkerUI.{prefix}{tag}"))
}

impl SingleInstance {
    /// 拿到了就返回；已经有一份在运行时把它的窗口切到前面（登录自启的那份不打扰）后返回 None
    fn acquire(replace: bool, tray: bool) -> Option<SingleInstance> {
        let tag = format!("{:016x}", fnv(&crate::setup::exe_path().display().to_string().to_lowercase()));
        unsafe {
            // 「Local\」命名空间（XP 起都有）；万一建不了就不做单实例，照常打开
            let Ok(m) = CreateMutexW(None, false, pcw(&named("", &tag))) else {
                log_error("建不了单实例的互斥体");
                return Some(SingleInstance { mutex: 0, show: 0, quit: 0 });
            };
            let show = CreateEventW(None, false, false, pcw(&named("show.", &tag))).unwrap_or_default();
            let quit = CreateEventW(None, false, false, pcw(&named("quit.", &tag))).unwrap_or_default();
            let got = |ms: u32| {
                let r = WaitForSingleObject(m, ms);
                r == WAIT_OBJECT_0 || r == WAIT_ABANDONED
            };
            let mut mine = got(0);
            if !mine && replace {
                let _ = SetEvent(quit);
                mine = got(8000);
            }
            if !mine {
                if !tray {
                    // 让正在运行的那份能把自己的窗口切到前台（前台锁只放行刚被用户打开的程序）
                    let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(u32::MAX);
                    let _ = SetEvent(show);
                }
                let _ = CloseHandle(m);
                return None;
            }
            Some(SingleInstance { mutex: m.0 as isize, show: show.0 as isize, quit: quit.0 as isize })
        }
    }

    /// 后台线程等「切到前面」「退出」两个事件，交回界面线程
    fn listen(&self) {
        let (show, quit) = (self.show, self.quit);
        if show == 0 || quit == 0 {
            return;
        }
        let _ = std::thread::Builder::new().name("single-instance".into()).spawn(move || loop {
            let hs = [HANDLE(show as _), HANDLE(quit as _)];
            let r = unsafe { WaitForMultipleObjects(&hs, false, INFINITE) };
            if r == WAIT_OBJECT_0 {
                ui_post(|| {
                    let g = GUI.with(|g| g.borrow().clone());
                    if let Some(g) = g {
                        g.present(None);
                    }
                });
            } else if r.0 == WAIT_OBJECT_0.0 + 1 {
                ui_post(|| {
                    let g = GUI.with(|g| g.borrow().clone());
                    if let Some(g) = g {
                        g.quit();
                    }
                });
            } else {
                break;
            }
        });
    }

    fn release(&self) {
        if self.mutex == 0 {
            return;
        }
        unsafe {
            let _ = ReleaseMutex(HANDLE(self.mutex as _));
        }
    }
}

// ---------------- 界面 ----------------

thread_local! {
    static GUI: RefCell<Option<Rc<Gui>>> = const { RefCell::new(None) };
}

struct Gui {
    ctx: Rc<Ctx>,
    win: RefCell<Option<Rc<MainWindow>>>,
    tray: RefCell<Option<Rc<Tray>>>,
    asks: Rc<Asks>,
}

impl Gui {
    fn start(settings: AppSettings, tray_only: bool) -> Rc<Gui> {
        let client = crate::client::Client::start();
        let ctx = Ctx::new(client.clone(), settings);
        let gui = Rc::new(Gui { ctx: ctx.clone(), win: RefCell::new(None), tray: RefCell::new(None), asks: Rc::new(Asks::default()) });
        GUI.with(|g| *g.borrow_mut() = Some(gui.clone()));

        let open: Rc<dyn Fn(Option<Page>)> = Rc::new(|p| {
            if let Some(g) = GUI.with(|g| g.borrow().clone()) {
                g.present(p);
            }
        });
        let quit: Rc<dyn Fn()> = Rc::new(|| {
            if let Some(g) = GUI.with(|g| g.borrow().clone()) {
                g.quit();
            }
        });
        *gui.tray.borrow_mut() = Some(Tray::new(&ctx, open, quit));
        let w = Rc::downgrade(&gui);
        ctx.on_app_change(move || {
            if let Some(g) = w.upgrade() {
                if let Some(t) = g.tray.borrow().as_ref() {
                    t.update();
                }
            }
        });
        ctx.set_quit(|| {
            if let Some(g) = GUI.with(|g| g.borrow().clone()) {
                g.quit();
            }
        });
        ctx.set_rebuild(|| {
            if let Some(g) = GUI.with(|g| g.borrow().clone()) {
                g.rebuild();
            }
        });

        let win = gui.make_window();
        *gui.win.borrow_mut() = Some(win.clone());
        if !tray_only {
            win.present(None);
        }

        let w = Rc::downgrade(&gui);
        client.subscribe(move |s| {
            if let Some(g) = w.upgrade() {
                g.on_signal(s);
            }
        });

        // 一秒多还连不上才显示「服务没运行」，免得一闪；没连上时每 3 秒查一次服务；托盘没加上（资源管理器还没起来）就重试
        let c = ctx.clone();
        set_timeout(1200, move || {
            c.grace_over.set(true);
            c.changed();
        });
        ctx.refresh_service();
        let w = Rc::downgrade(&gui);
        set_interval(3000, move || {
            let Some(g) = w.upgrade() else { return };
            if !g.ctx.connected() {
                g.ctx.refresh_service();
            }
            let t = g.tray.borrow().clone();
            if let Some(t) = t {
                if !t.added() {
                    t.add();
                }
            }
        });
        gui
    }

    fn make_window(&self) -> Rc<MainWindow> {
        let hooks = Hooks {
            quit: Rc::new(|| {
                if let Some(g) = GUI.with(|g| g.borrow().clone()) {
                    g.quit();
                }
            }),
            balloon: Rc::new(|title, text| GUI.with(|g| g.borrow().as_ref().and_then(|g| g.tray.borrow().as_ref().map(|t| t.balloon(title, text, false, None)))).unwrap_or(false)),
            has_tray: Rc::new(|| GUI.with(|g| g.borrow().as_ref().and_then(|g| g.tray.borrow().as_ref().map(|t| t.added()))).unwrap_or(false)),
        };
        MainWindow::new(&self.ctx, hooks)
    }

    fn present(&self, page: Option<Page>) {
        let w = self.win.borrow().clone();
        if let Some(w) = w {
            w.present(page);
        }
    }

    /// 退出界面（被控服务照常运行）
    fn quit(&self) {
        self.asks.close_all();
        if let Some(t) = self.tray.borrow().as_ref() {
            t.remove();
        }
        let w = self.win.borrow_mut().take();
        if let Some(w) = w {
            w.destroy();
        }
        unsafe { PostQuitMessage(0) };
    }

    /// 换了语言：整个窗口按新语言重建，停在原来那一页
    fn rebuild(&self) {
        let old = self.win.borrow_mut().take();
        let (visible, page) = match &old {
            Some(w) => (Ctl(w.hwnd).visible(), w.current()),
            None => (false, Page::Settings),
        };
        self.ctx.reset_window_hooks();
        if let Some(w) = old {
            w.destroy();
        }
        let w = self.make_window();
        *self.win.borrow_mut() = Some(w.clone());
        self.ctx.client.send(Request::Hello { lang: crate::i18n::lang_tag().into() });
        self.ctx.replay_status();
        w.show_page(page);
        if visible {
            w.present(None);
        }
        if let Some(t) = self.tray.borrow().as_ref() {
            t.update();
        }
    }

    fn on_signal(self: &Rc<Self>, s: &Signal) {
        let ctx = &self.ctx;
        match s {
            Signal::Connected => {
                let c = ctx.clone();
                ctx.client.call(Request::Hello { lang: crate::i18n::lang_tag().into() }, move |_| {
                    let c2 = c.clone();
                    c.client.call(Request::Status, move |r| {
                        if let Response::Status { status } = r {
                            c2.set_status(status);
                        }
                    });
                });
            }
            Signal::Disconnected => {
                self.asks.close_all();
                ctx.clear_status();
                ctx.refresh_service();
            }
            Signal::Event(e) => match e {
                Event::Status { status } => ctx.set_status(status.clone()),
                Event::SessionStarted { session } => {
                    if session.notify && ctx.settings.borrow().notifications {
                        let body = crate::i18n::fmt(tr!("{0} 从设备 {1} 连入。", "{0} connected from device {1}."), &[&who(&session.from_user), &short_device(&session.from_device)]);
                        if let Some(t) = self.tray.borrow().as_ref() {
                            let open: Rc<dyn Fn()> = Rc::new(|| {
                                if let Some(g) = GUI.with(|g| g.borrow().clone()) {
                                    g.present(Some(Page::Overview));
                                }
                            });
                            t.balloon(tr!("有人连入了这台电脑", "Someone connected to this computer"), &body, false, Some(open));
                        }
                    }
                }
                Event::AssistRequest { request_id, from_user, from_device } => {
                    self.asks.ask(ctx, *request_id, from_user, from_device);
                    let body = crate::i18n::fmt(tr!("{0} 从设备 {1} 发起了协助请求。", "{0} on device {1} asked to help."), &[&who(from_user), &short_device(from_device)]);
                    if let Some(t) = self.tray.borrow().as_ref() {
                        let open: Rc<dyn Fn()> = Rc::new(|| {
                            if let Some(g) = GUI.with(|g| g.borrow().clone()) {
                                g.present(Some(Page::Assist));
                            }
                        });
                        t.balloon(tr!("协助请求", "Assistance Request"), &body, true, Some(open));
                    }
                }
                Event::AssistRequestClosed { request_id } => self.asks.closed(*request_id),
                Event::AssistEnded { .. } => ctx.toast(tr!("临时协助已结束", "Assistance ended")),
                Event::SessionEnded { .. } | Event::AiActivity { .. } => {}
            },
        }
    }
}
