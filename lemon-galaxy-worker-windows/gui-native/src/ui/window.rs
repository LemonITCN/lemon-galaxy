//! 主窗口：左边侧栏（标志、状态、页面列表、允许远程），右边提示条 + 页面；没连上服务时整页换成连接中 / 安装 / 启动服务。
//! 关窗口只是藏到托盘（第一次提示一下），界面进程与被控服务都继续运行；注销、关机时直接关。

use super::*;
use crate::win::layout::Panel;
use std::rc::Weak;
use windows::Win32::Foundation::{LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, ODS_FOCUS, ODS_NOFOCUSRECT, ODS_SELECTED};
use windows::Win32::UI::WindowsAndMessaging::*;

/// 程序给窗口的回调
pub struct Hooks {
    /// 退出界面
    pub quit: Rc<dyn Fn()>,
    /// 托盘气泡（标题、正文）；没有托盘时返回 false
    pub balloon: Rc<dyn Fn(&str, &str) -> bool>,
    /// 托盘图标在（关窗口时藏到托盘；不在就最小化）
    pub has_tray: Rc<dyn Fn() -> bool>,
}

pub struct MainWindow {
    pub hwnd: HWND,
    ctx: Rc<Ctx>,
    hooks: Hooks,
    side: HWND,
    nav: Ctl,
    remote: Ctl,
    bar: HWND,
    bar_text: RefCell<(String, bool)>,
    bar_serial: Cell<u64>,
    down: Rc<Panel>,
    pages: Vec<(Page, PageView)>,
    current: Cell<Page>,
    really_close: Cell<bool>,
    weak: RefCell<Weak<MainWindow>>,
}

const NAV_ITEM_H: i32 = 34;

impl MainWindow {
    pub fn new(ctx: &Rc<Ctx>, hooks: Hooks) -> Rc<MainWindow> {
        let cls = wide(CLASS_TOP);
        let title = wide(app_name());
        let (w, h) = {
            let s = ctx.settings.borrow();
            (px((s.window_width as i32).clamp(760, 2400)), px((s.window_height as i32).clamp(540, 1800)))
        };
        let area = work_area(None);
        let (w, h) = (w.min(area.right - area.left), h.min(area.bottom - area.top));
        let x = area.left + (area.right - area.left - w) / 2;
        let y = area.top + (area.bottom - area.top - h) / 2;
        let hwnd_ = unsafe {
            CreateWindowExW(WS_EX_CONTROLPARENT, pcw(&cls), pcw(&title), WINDOW_STYLE(WS_OVERLAPPEDWINDOW.0 | WS_CLIPCHILDREN.0), x, y, w, h, None, None, Some(hinstance()), None).unwrap_or_default()
        };
        ctx.set_main(hwnd_);
        add_root(hwnd_);

        // 侧栏（自己画、自己摆）
        let side_h = child_window(hwnd_, WS_EX_CONTROLPARENT.0, WS_CLIPCHILDREN.0);
        set_bg(side_h, sys(COLOR_BTNFACE));
        let nav = create("LISTBOX", "", (LBS_OWNERDRAWFIXED | LBS_HASSTRINGS | LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32 | WS_TABSTOP.0, 0, side_h, None);
        for p in Page::ALL {
            let t = wide(p.title());
            nav.send(LB_ADDSTRING, 0, t.as_ptr() as isize);
        }
        nav.send(LB_SETITEMHEIGHT, 0, px(NAV_ITEM_H) as isize);
        nav.send(LB_SETCURSEL, 0, 0);
        let remote = checkbox(side_h, tr!("允许远程控制", "Allow remote control"));
        add_tip(remote, tr!("关闭后，所有账号都连不进来，正在进行的连接会立即断开。屏幕前的用户可以直接关闭；重新允许需要管理员授权。", "When off, no account can connect and active sessions end immediately. The person at this computer can turn it off; turning it back on needs administrator authorization."));

        // 提示条
        let bar_h = child_window(hwnd_, 0, 0);

        // 没连上服务时的整页
        let down = super::setup_page::build(ctx, hwnd_);

        // 各页
        let mut pages = vec![];
        for p in Page::ALL {
            let v = match p {
                Page::Overview => super::overview::build(ctx, hwnd_),
                Page::Accounts => super::accounts::build(ctx, hwnd_),
                Page::Assist => super::assist::build(ctx, hwnd_),
                Page::Settings => super::settings::build(ctx, hwnd_),
                Page::Events => super::logs::build(ctx, hwnd_),
                Page::About => super::about::build(ctx, hwnd_),
            };
            pages.push((p, v));
        }

        let me = Rc::new(MainWindow {
            hwnd: hwnd_,
            ctx: ctx.clone(),
            hooks,
            side: side_h,
            nav,
            remote,
            bar: bar_h,
            bar_text: RefCell::new((String::new(), false)),
            bar_serial: Cell::new(0),
            down,
            pages,
            current: Cell::new(Page::Overview),
            really_close: Cell::new(false),
            weak: RefCell::new(Weak::new()),
        });
        *me.weak.borrow_mut() = Rc::downgrade(&me);

        let w = Rc::downgrade(&me);
        set_proc(hwnd_, move |h, msg, wp, lp| w.upgrade()?.handle(h, msg, wp, lp));
        // 回车：焦点在按钮上就按它
        on_root_key(hwnd_, move |vk| vk as usize == VK_RETURN_ && crate::win::dialog::click_focused_button(hwnd_));
        let w = Rc::downgrade(&me);
        set_proc(side_h, move |h, msg, wp, lp| w.upgrade()?.side_proc(h, msg, wp, lp));
        let w = Rc::downgrade(&me);
        set_proc(bar_h, move |h, msg, _wp, lp| w.upgrade()?.bar_proc(h, msg, lp));
        let w = Rc::downgrade(&me);
        on_draw(nav, move |d| {
            if let Some(m) = w.upgrade() {
                m.draw_nav(d);
            }
        });
        let w = Rc::downgrade(&me);
        on_command(nav, move |code| {
            if code == LBN_SELCHANGE {
                if let Some(m) = w.upgrade() {
                    let i = m.nav.send(LB_GETCURSEL, 0, 0);
                    if let Some(p) = Page::ALL.get(i.max(0) as usize) {
                        m.show_page(*p);
                    }
                }
            }
        });
        let c = ctx.clone();
        toggle(remote, move |on, done| c.act(Request::SetRemoteEnabled { enabled: on }, move |r| done(r.is_some())));

        let w = Rc::downgrade(&me);
        ctx.set_toast(move |msg, error| {
            if let Some(m) = w.upgrade() {
                m.show_toast(msg, error);
            }
        });
        let w = Rc::downgrade(&me);
        ctx.set_goto(move |p| {
            if let Some(m) = w.upgrade() {
                m.present(Some(p));
            }
        });
        let w = Rc::downgrade(&me);
        ctx.on_change(move || {
            if let Some(m) = w.upgrade() {
                m.on_change();
            }
        });
        me.on_change();
        me
    }

    fn on_change(&self) {
        let st = self.ctx.status.borrow().clone();
        if let Some(s) = &st {
            let r = &s.rights;
            self.remote.set_checked(s.remote_enabled);
            self.remote.enable(self.ctx.busy.get().is_none() && (r.can_manage() || (r.operate && s.remote_enabled)));
            self.remote.set_shield(!s.remote_enabled && r.manage == ManageRight::Elevate);
        }
        self.layout();
        // 状态文字变长变短时页面列表跟着挪
        self.layout_side();
        Ctl(self.side).redraw();
        self.nav.redraw();
        if st.is_none() {
            self.down.relayout_soon();
        }
    }

    pub fn current(&self) -> Page {
        self.current.get()
    }

    /// 显示窗口（从托盘、另一个实例唤起）；`page` 给了就切到那一页
    pub fn present(&self, page: Option<Page>) {
        if let Some(p) = page {
            self.show_page(p);
        }
        bring_to_front(self.hwnd);
    }

    pub fn show_page(&self, p: Page) {
        self.current.set(p);
        let i = Page::ALL.iter().position(|x| *x == p).unwrap_or(0);
        if self.nav.send(LB_GETCURSEL, 0, 0) != i as isize {
            self.nav.send(LB_SETCURSEL, i, 0);
        }
        self.layout();
        for (pp, v) in &self.pages {
            if *pp == p {
                if let Some(r) = &v.refresh {
                    r();
                }
                v.panel.relayout();
            }
        }
    }

    /// 主窗口里各块的位置
    fn layout(&self) {
        let rc = client_rect(self.hwnd);
        let (w, h) = (rc.right, rc.bottom);
        let connected = self.ctx.connected();
        let side_w = if connected { px(212) } else { 0 };
        Ctl(self.side).show(connected);
        if connected {
            unsafe {
                let _ = SetWindowPos(self.side, None, 0, 0, side_w, h, SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
        let x = side_w;
        let cw = w - x;
        let (text, _) = self.bar_text.borrow().clone();
        let bar_h = if text.is_empty() { 0 } else { self.bar_height(cw) };
        Ctl(self.bar).show(bar_h > 0);
        if bar_h > 0 {
            unsafe {
                let _ = SetWindowPos(self.bar, None, x, 0, cw, bar_h, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            Ctl(self.bar).redraw();
        }
        let cur = self.current.get();
        for (p, v) in &self.pages {
            let on = connected && *p == cur;
            if on {
                v.panel.move_to(x, bar_h, cw, h - bar_h);
            }
            v.panel.show(on);
        }
        if !connected {
            self.down.move_to(x, bar_h, cw, h - bar_h);
        }
        self.down.show(!connected);
    }

    // ---------------- 提示条 ----------------

    fn show_toast(&self, msg: &str, error: bool) {
        *self.bar_text.borrow_mut() = (msg.to_string(), error);
        let n = self.bar_serial.get() + 1;
        self.bar_serial.set(n);
        self.layout();
        // 4 秒（出错 8 秒）后收起；中间又来了新的就按新的算
        let me = self.weak.borrow().clone();
        set_timeout(if error { 8000 } else { 4000 }, move || {
            if let Some(m) = me.upgrade() {
                if m.bar_serial.get() == n {
                    m.hide_toast();
                }
            }
        });
    }

    fn hide_toast(&self) {
        *self.bar_text.borrow_mut() = (String::new(), false);
        self.layout();
    }

    fn bar_height(&self, w: i32) -> i32 {
        let (text, _) = self.bar_text.borrow().clone();
        let tw = w - px(24 + 6 + 28);
        let (_, th) = measure(font(Font::Normal), &text, tw, true);
        th + px(20)
    }

    fn bar_proc(&self, h: HWND, msg: u32, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_ERASEBKGND => Some(LRESULT(1)),
            WM_PAINT => {
                let (text, error) = self.bar_text.borrow().clone();
                let tone = if error { Tone::Error } else { Tone::Accent };
                unsafe {
                    let mut ps = PAINTSTRUCT::default();
                    let dc = BeginPaint(h, &mut ps);
                    let rc = client_rect(h);
                    FillRect(dc, &rc, brush(tone_fill(tone)));
                    let bar = RECT { left: 0, top: 0, right: px(3), bottom: rc.bottom };
                    FillRect(dc, &bar, brush(tone_color(tone)));
                    let line = RECT { left: 0, top: rc.bottom - 1, right: rc.right, bottom: rc.bottom };
                    FillRect(dc, &line, brush(line_color()));
                    let old = SelectObject(dc, HGDIOBJ(font(Font::Normal).0));
                    SetBkMode(dc, TRANSPARENT);
                    SetTextColor(dc, windows::Win32::Foundation::COLORREF(sys(COLOR_WINDOWTEXT)));
                    let mut t: Vec<u16> = text.encode_utf16().collect();
                    let mut r = RECT { left: px(18), top: px(10), right: rc.right - px(34), bottom: rc.bottom - px(10) };
                    DrawTextW(dc, &mut t, &mut r, DT_WORDBREAK | DT_NOPREFIX | DT_EXPANDTABS);
                    // 关闭
                    SetTextColor(dc, windows::Win32::Foundation::COLORREF(tone_color(Tone::Dim)));
                    let mut x: Vec<u16> = "×".encode_utf16().collect();
                    let mut r = self.close_rect(&rc);
                    DrawTextW(dc, &mut x, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
                    SelectObject(dc, old);
                    let _ = EndPaint(h, &ps);
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let x = (lp.0 & 0xffff) as i16 as i32;
                let y = ((lp.0 >> 16) & 0xffff) as i16 as i32;
                let rc = client_rect(h);
                let r = self.close_rect(&rc);
                if x >= r.left && x < r.right && y >= r.top && y < r.bottom {
                    self.hide_toast();
                }
                Some(LRESULT(0))
            }
            WM_SETCURSOR => None,
            _ => None,
        }
    }

    fn close_rect(&self, rc: &RECT) -> RECT {
        RECT { left: rc.right - px(30), top: 0, right: rc.right - px(4), bottom: px(36).min(rc.bottom) }
    }

    // ---------------- 侧栏 ----------------

    fn header_h(&self, w: i32) -> i32 {
        let tw = w - px(16 + 32 + 10 + 10);
        let (_, nh) = measure(font(Font::Bold), app_name(), tw, true);
        let (_, sh) = measure(font(Font::Normal), overall_text(overall(&self.ctx)), tw - px(12), true);
        px(18) + (nh + px(3) + sh).max(px(32)) + px(16)
    }

    fn side_proc(&self, h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let _ = (wp, lp);
        match msg {
            WM_SIZE => {
                self.layout_side();
                Some(LRESULT(0))
            }
            WM_ERASEBKGND => Some(LRESULT(1)),
            WM_PAINT => {
                self.paint_side(h);
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn layout_side(&self) {
        let rc = client_rect(self.side);
        let (w, h) = (rc.right, rc.bottom);
        let top = self.header_h(w);
        let (_, ch) = pref(self.remote);
        let bottom = ch + px(28);
        unsafe {
            let _ = SetWindowPos(self.nav.0, None, px(8), top, w - px(16) - 1, (h - top - bottom).max(0), SWP_NOZORDER | SWP_NOACTIVATE);
            let _ = SetWindowPos(self.remote.0, None, px(16), h - bottom + px(14), w - px(32), ch, SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    fn paint_side(&self, h: HWND) {
        unsafe {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(h, &mut ps);
            let rc = client_rect(h);
            let bg = sys(COLOR_BTNFACE);
            FillRect(dc, &rc, brush(bg));
            // 右边分隔线、底部分隔线
            let edge = blend(bg, sys(COLOR_BTNSHADOW), 0.7);
            FillRect(dc, &RECT { left: rc.right - 1, top: 0, right: rc.right, bottom: rc.bottom }, brush(edge));
            let (_, ch) = pref(self.remote);
            let by = rc.bottom - ch - px(28);
            FillRect(dc, &RECT { left: px(16), top: by, right: rc.right - px(16), bottom: by + 1 }, brush(edge));
            // 标志、名字、状态
            let icon = app_icon(px(32));
            let (ix, iy) = (px(16), px(18));
            if !icon.0.is_null() {
                let _ = DrawIconEx(dc, ix, iy, icon, px(32), px(32), 0, None, DI_NORMAL);
                let _ = DestroyIcon(icon);
            }
            let tx = ix + px(32) + px(10);
            let tw = rc.right - tx - px(10);
            SetBkMode(dc, TRANSPARENT);
            let old = SelectObject(dc, HGDIOBJ(font(Font::Bold).0));
            SetTextColor(dc, windows::Win32::Foundation::COLORREF(sys(COLOR_BTNTEXT)));
            let mut name: Vec<u16> = app_name().encode_utf16().collect();
            let mut r = RECT { left: tx, top: iy, right: tx + tw, bottom: iy + px(200) };
            let nh = DrawTextW(dc, &mut name, &mut r, DT_WORDBREAK | DT_NOPREFIX);
            let o = overall(&self.ctx);
            let sy = iy + nh + px(3);
            let lh = line_h();
            let d = px(8);
            let dot = RECT { left: tx, top: sy + (lh - d) / 2, right: tx + d, bottom: sy + (lh - d) / 2 + d };
            let np = SelectObject(dc, GetStockObject(NULL_PEN));
            let ob = SelectObject(dc, HGDIOBJ(brush(overall_color(o)).0));
            let _ = Ellipse(dc, dot.left, dot.top, dot.right + 1, dot.bottom + 1);
            SelectObject(dc, ob);
            SelectObject(dc, np);
            SelectObject(dc, HGDIOBJ(font(Font::Normal).0));
            SetTextColor(dc, windows::Win32::Foundation::COLORREF(blend(sys(COLOR_BTNTEXT), bg, 0.4)));
            let mut st: Vec<u16> = overall_text(o).encode_utf16().collect();
            let mut r = RECT { left: tx + d + px(6), top: sy, right: tx + tw, bottom: sy + px(200) };
            DrawTextW(dc, &mut st, &mut r, DT_WORDBREAK | DT_NOPREFIX);
            SelectObject(dc, old);
            let _ = EndPaint(h, &ps);
        }
    }

    fn draw_nav(&self, d: &DRAWITEMSTRUCT) {
        if d.itemID == u32::MAX {
            return;
        }
        let Some(p) = Page::ALL.get(d.itemID as usize).copied() else { return };
        unsafe {
            let dc = d.hDC;
            let r = d.rcItem;
            let bg = sys(COLOR_BTNFACE);
            FillRect(dc, &r, brush(bg));
            let sel = d.itemState.0 & ODS_SELECTED.0 != 0;
            let inner = RECT { left: r.left + px(2), top: r.top + px(2), right: r.right - px(2), bottom: r.bottom - px(2) };
            if sel {
                let pen = CreatePen(PS_SOLID, 1, windows::Win32::Foundation::COLORREF(blend(bg, sys(COLOR_BTNSHADOW), 0.55)));
                let op = SelectObject(dc, HGDIOBJ(pen.0));
                let ob = SelectObject(dc, HGDIOBJ(brush(sys(COLOR_WINDOW)).0));
                let _ = RoundRect(dc, inner.left, inner.top, inner.right, inner.bottom, px(8), px(8));
                SelectObject(dc, ob);
                SelectObject(dc, op);
                let _ = DeleteObject(HGDIOBJ(pen.0));
                // 选中项左边的色条
                let bar = RECT { left: inner.left + px(1), top: inner.top + px(7), right: inner.left + px(4), bottom: inner.bottom - px(7) };
                FillRect(dc, &bar, brush(tone_color(Tone::Accent)));
            }
            SetBkMode(dc, TRANSPARENT);
            let old = SelectObject(dc, HGDIOBJ(font(if sel { Font::Bold } else { Font::Normal }).0));
            SetTextColor(dc, windows::Win32::Foundation::COLORREF(if sel { sys(COLOR_WINDOWTEXT) } else { sys(COLOR_BTNTEXT) }));
            let mut t: Vec<u16> = p.title().encode_utf16().collect();
            let mut tr_ = RECT { left: inner.left + px(14), top: inner.top, right: inner.right - px(40), bottom: inner.bottom };
            DrawTextW(dc, &mut t, &mut tr_, DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS);
            // 徽标：概览 = 连接数，账号 = 要重新登录，协助 = 进行中
            let st = self.ctx.status.borrow();
            let badge: Option<(String, u32)> = match (p, st.as_ref()) {
                (Page::Overview, Some(s)) if !s.sessions.is_empty() => {
                    let ai = s.sessions.iter().any(|x| x.ai_active);
                    Some((s.sessions.len().to_string(), if ai { rgb(0xE8, 0x96, 0x1E) } else { rgb(0xD9, 0x3B, 0x3B) }))
                }
                (Page::Accounts, Some(s)) if s.needs_relogin > 0 => Some(("!".into(), rgb(0xE8, 0x96, 0x1E))),
                (Page::Assist, Some(s)) if s.assist.is_some() => Some((String::new(), rgb(0x2E, 0xB8, 0x6B))),
                _ => None,
            };
            if let Some((text, color)) = badge {
                let np = SelectObject(dc, GetStockObject(NULL_PEN));
                let ob = SelectObject(dc, HGDIOBJ(brush(color).0));
                let cy = (inner.top + inner.bottom) / 2;
                if text.is_empty() {
                    let s = px(8);
                    let x = inner.right - px(14) - s;
                    let _ = Ellipse(dc, x, cy - s / 2, x + s + 1, cy - s / 2 + s + 1);
                } else {
                    SelectObject(dc, HGDIOBJ(font(Font::Bold).0));
                    let (tw, th) = measure(font(Font::Bold), &text, 1000, false);
                    let bw = (tw + px(10)).max(th + px(2));
                    let bh = th + px(2);
                    let x = inner.right - px(10) - bw;
                    let _ = RoundRect(dc, x, cy - bh / 2, x + bw + 1, cy - bh / 2 + bh + 1, bh, bh);
                    SetTextColor(dc, windows::Win32::Foundation::COLORREF(rgb(255, 255, 255)));
                    let mut bt: Vec<u16> = text.encode_utf16().collect();
                    let mut br = RECT { left: x, top: cy - bh / 2, right: x + bw, bottom: cy - bh / 2 + bh };
                    DrawTextW(dc, &mut bt, &mut br, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
                }
                SelectObject(dc, ob);
                SelectObject(dc, np);
            }
            SelectObject(dc, old);
            if d.itemState.0 & ODS_FOCUS.0 != 0 && d.itemState.0 & ODS_NOFOCUSRECT.0 == 0 {
                let fr = RECT { left: inner.left + px(2), top: inner.top + px(2), right: inner.right - px(2), bottom: inner.bottom - px(2) };
                let _ = DrawFocusRect(dc, &fr);
            }
        }
    }

    // ---------------- 主窗口消息 ----------------

    fn save_size(&self) {
        let mut r = RECT::default();
        unsafe {
            if IsIconic(self.hwnd).as_bool() || GetWindowRect(self.hwnd, &mut r).is_err() {
                return;
            }
        }
        let mut s = self.ctx.settings.borrow_mut();
        s.window_width = ((r.right - r.left) * 96 / dpi()) as f64;
        s.window_height = ((r.bottom - r.top) * 96 / dpi()) as f64;
        s.save();
    }

    /// 真的关掉窗口（退出界面、换语言重建时）
    pub fn destroy(&self) {
        self.really_close.set(true);
        self.save_size();
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }

    fn handle(&self, h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_SIZE => {
                self.layout();
                Some(LRESULT(0))
            }
            WM_GETMINMAXINFO => {
                let mi = unsafe { &mut *(lp.0 as *mut MINMAXINFO) };
                mi.ptMinTrackSize.x = px(760);
                mi.ptMinTrackSize.y = px(520);
                Some(LRESULT(0))
            }
            WM_CLOSE => {
                if self.really_close.get() {
                    return None;
                }
                self.save_size();
                if (self.hooks.has_tray)() {
                    // 藏到托盘，进程与服务都继续
                    unsafe {
                        let _ = ShowWindow(h, SW_HIDE);
                    }
                    let told = self.ctx.settings.borrow().told_about_tray;
                    if !told && (self.hooks.balloon)(tr!("LemonGalaxy 还在托盘里", "LemonGalaxy is still in the tray"), tr!("关掉窗口不影响远程控制。要退出界面，在托盘图标的右键菜单里选「退出界面」。", "Closing the window does not affect remote control. To exit the app, right-click the tray icon and choose Exit App.")) {
                        let mut s = self.ctx.settings.borrow_mut();
                        s.told_about_tray = true;
                        s.save();
                    }
                } else {
                    unsafe {
                        let _ = ShowWindow(h, SW_MINIMIZE);
                    }
                }
                Some(LRESULT(0))
            }
            // 注销、关机：直接退出，不藏到托盘（否则会挡住注销）
            WM_QUERYENDSESSION => Some(LRESULT(1)),
            WM_ENDSESSION => {
                if wp.0 != 0 {
                    self.save_size();
                    (self.hooks.quit)();
                }
                Some(LRESULT(0))
            }
            // 回车：焦点在按钮上就按它（IsDialogMessage 找不到默认按钮时发 IDOK）
            WM_COMMAND if lp.0 == 0 => {
                crate::win::dialog::click_focused_button(h);
                Some(LRESULT(0))
            }
            WM_SETTINGCHANGE | WM_SYSCOLORCHANGE => {
                self.layout();
                unsafe {
                    let _ = RedrawWindow(Some(h), None, None, RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_ERASE);
                }
                None
            }
            _ => None,
        }
    }
}
