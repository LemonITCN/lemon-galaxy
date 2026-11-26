//! 对话框：顶层窗口里放一个页面容器（`Panel`），高度按内容定；可模态（禁用主窗口）、可置顶。
//! 回车按默认按钮、Esc 按取消（IsDialogMessage）。另有同步的确认框 `confirm`（嵌套消息循环，像 MessageBox）。

use super::layout::*;
use super::*;

/// 对话框里的 IDOK / IDCANCEL
pub const IDOK_: i32 = 1;
pub const IDCANCEL_: i32 = 2;
const DM_GETDEFID: u32 = WM_USER;
const DC_HASDEFID: isize = 0x534B;

pub struct Dialog {
    pub hwnd: HWND,
    pub panel: Rc<Panel>,
    owner: Option<HWND>,
    modal: bool,
    width: i32,
    closed: Cell<bool>,
    ok: RefCell<Option<Rc<dyn Fn()>>>,
    cancel: RefCell<Option<Rc<dyn Fn()>>>,
    /// 回车按哪个按钮
    default_id: Cell<i32>,
}

impl Dialog {
    /// `width`：内容区宽（设计尺寸）
    pub fn new(owner: Option<HWND>, title: &str, width: i32, modal: bool, topmost: bool) -> Rc<Dialog> {
        let cls = wide(CLASS_TOP);
        let t = wide(title);
        let mut ex = WS_EX_DLGMODALFRAME.0 | WS_EX_CONTROLPARENT.0;
        if topmost {
            ex |= WS_EX_TOPMOST.0;
        }
        // 没有主窗口时也要在任务栏上（协助请求、卸载确认）
        if owner.is_none() {
            ex |= WS_EX_APPWINDOW.0;
        }
        let style = WS_POPUP.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_CLIPCHILDREN.0;
        let h = unsafe { CreateWindowExW(WINDOW_EX_STYLE(ex), pcw(&cls), pcw(&t), WINDOW_STYLE(style), 0, 0, px(width), px(200), owner, None, Some(hinstance()), None).unwrap_or_default() };
        let panel = Panel::new(h, false);
        panel.show(true);
        panel.max_w.set(px(width));
        panel.pad.set((px(22), px(20)));
        let d = Rc::new(Dialog {
            hwnd: h,
            panel,
            owner,
            modal,
            width,
            closed: Cell::new(false),
            ok: RefCell::new(None),
            cancel: RefCell::new(None),
            default_id: Cell::new(IDOK_),
        });
        add_root(h);
        // 回车：焦点在按钮上就按它，否则按默认按钮；Esc：取消
        let w = Rc::downgrade(&d);
        on_root_key(h, move |vk| {
            let Some(d) = w.upgrade() else { return false };
            if vk as usize == VK_ESCAPE_ {
                d.cancel_now();
            } else if !click_focused_button(d.hwnd) {
                if d.default_id.get() == IDCANCEL_ {
                    d.cancel_now();
                } else {
                    let f = d.ok.borrow().clone();
                    match f {
                        Some(f) => f(),
                        None => d.close(),
                    }
                }
            }
            true
        });
        let w = Rc::downgrade(&d);
        set_proc(h, move |_h, msg, wp, _lp| {
            let d = w.upgrade()?;
            match msg {
                WM_SIZE => {
                    let rc = client_rect(d.hwnd);
                    d.panel.move_to(0, 0, rc.right, rc.bottom);
                    Some(LRESULT(0))
                }
                DM_GETDEFID => Some(LRESULT(DC_HASDEFID << 16 | d.default_id.get() as isize)),
                // 顶层窗口只有容器一个子窗口，收到的 WM_COMMAND 都来自 IsDialogMessage（回车、Esc）
                WM_COMMAND => {
                    let id = loword(wp.0) as i32;
                    if id == IDCANCEL_ {
                        d.cancel_now();
                    } else if !click_focused_button(d.hwnd) {
                        let f = d.ok.borrow().clone();
                        match f {
                            Some(f) => f(),
                            None => d.close(),
                        }
                    }
                    Some(LRESULT(0))
                }
                WM_CLOSE => {
                    d.cancel_now();
                    Some(LRESULT(0))
                }
                _ => None,
            }
        });
        d
    }

    fn cancel_now(&self) {
        let f = self.cancel.borrow().clone();
        match f {
            Some(f) => f(),
            None => self.close(),
        }
    }

    /// 回车（默认按钮）
    pub fn on_ok(&self, f: impl Fn() + 'static) {
        *self.ok.borrow_mut() = Some(Rc::new(f));
    }

    /// Esc、标题栏的关闭、取消按钮
    pub fn on_cancel(&self, f: impl Fn() + 'static) {
        *self.cancel.borrow_mut() = Some(Rc::new(f));
    }

    pub fn set_default(&self, id: i32) {
        self.default_id.set(id);
    }

    pub fn set_layout(&self, f: impl Fn(&mut Flow) + 'static) {
        self.panel.set_layout(f);
    }

    pub fn is_closed(&self) -> bool {
        self.closed.get()
    }

    /// 按内容定高度（步骤变了、错误提示出现时再调一次），保持中心不动
    pub fn fit(&self) {
        let cw = px(self.width) + 2 * self.panel.pad.get().0;
        let ch = self.panel.measure(cw);
        let style = WINDOW_STYLE(unsafe { get_long(self.hwnd, GWL_STYLE) } as u32);
        let ex = WINDOW_EX_STYLE(unsafe { get_long(self.hwnd, GWL_EXSTYLE) } as u32);
        let mut r = RECT { left: 0, top: 0, right: cw, bottom: ch };
        unsafe {
            let _ = AdjustWindowRectEx(&mut r, style, false, ex);
        }
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let mut cur = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.hwnd, &mut cur);
        }
        let visible = Ctl(self.hwnd).visible();
        let (x, y) = if visible {
            ((cur.left + cur.right) / 2 - w / 2, (cur.top + cur.bottom) / 2 - h / 2)
        } else {
            // 第一次：在主窗口（没有就在屏幕工作区）中间
            let area = match self.owner.filter(|o| unsafe { IsWindowVisible(*o).as_bool() && !IsIconic(*o).as_bool() }) {
                Some(o) => {
                    let mut r = RECT::default();
                    unsafe {
                        let _ = GetWindowRect(o, &mut r);
                    }
                    r
                }
                None => work_area(self.owner),
            };
            ((area.left + area.right) / 2 - w / 2, (area.top + area.bottom) / 2 - h / 2)
        };
        let wa = work_area(Some(self.hwnd));
        let x = x.clamp(wa.left, (wa.right - w).max(wa.left));
        let y = y.clamp(wa.top, (wa.bottom - h).max(wa.top));
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
        }
        self.panel.relayout();
    }

    pub fn show(&self) {
        self.fit();
        if self.modal {
            if let Some(o) = self.owner {
                Ctl(o).enable(false);
            }
        }
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.hwnd);
        }
    }

    /// 关掉（模态的先把主窗口恢复可用，焦点才回得去）
    pub fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        // 回调里常持有对话框本身：去掉，免得互相引用放不掉
        *self.ok.borrow_mut() = None;
        *self.cancel.borrow_mut() = None;
        if self.modal {
            if let Some(o) = self.owner {
                Ctl(o).enable(true);
            }
        }
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
        if let Some(o) = self.owner {
            if unsafe { IsWindowVisible(o).as_bool() } {
                unsafe {
                    let _ = SetForegroundWindow(o);
                }
            }
        }
    }

    /// 闪任务栏按钮（不抢焦点地提醒）
    pub fn flash(&self) {
        let fi = FLASHWINFO { cbSize: std::mem::size_of::<FLASHWINFO>() as u32, hwnd: self.hwnd, dwFlags: FLASHW_ALL | FLASHW_TIMERNOFG, uCount: 5, dwTimeout: 0 };
        unsafe {
            let _ = FlashWindowEx(&fi);
        }
    }
}

/// 回车时焦点在这个窗口里的某个按钮上：按它（返回 true）
pub fn click_focused_button(root: HWND) -> bool {
    let f = unsafe { GetFocus() };
    if f.0.is_null() || !unsafe { IsChild(root, f).as_bool() } {
        return false;
    }
    let c = Ctl(f);
    let kind = c.style() & 0xF;
    if c.class().eq_ignore_ascii_case("button") && (kind == BS_PUSHBUTTON as u32 || kind == BS_DEFPUSHBUTTON as u32) {
        c.send(BM_CLICK, 0, 0);
        return true;
    }
    false
}

/// 确认框（模态，等用户选完才返回）。`destructive` 时默认按钮是「取消」
pub fn confirm(owner: Option<HWND>, caption: &str, title: &str, body: &str, ok: &str, cancel: &str, destructive: bool) -> bool {
    let d = Dialog::new(owner, caption, 380, owner.is_some(), owner.is_none());
    let p = d.panel.hwnd;
    let head = label_font(p, title, Font::Bold);
    let text = label(p, body);
    let ok_b = button_id(p, ok, IDOK_, !destructive);
    let cancel_b = button_id(p, cancel, IDCANCEL_, destructive);
    let result = Rc::new(Cell::new(None::<bool>));
    let icon = if destructive { 32515u16 /* IDI_WARNING */ } else { 32514 /* IDI_QUESTION */ };
    d.set_layout(move |f| {
        let s = px(32);
        f.sys_icon(f.x, f.y, s, icon);
        let (x0, w0) = (f.x, f.w);
        f.x += s + px(14);
        f.w -= s + px(14);
        let top = f.y;
        f.text(head);
        f.gap(6);
        f.text(text);
        f.y = f.y.max(top + s);
        f.x = x0;
        f.w = w0;
        f.footer(&[ok_b, cancel_b], &[]);
    });
    let r = result.clone();
    let dw = Rc::downgrade(&d);
    let done_ok = move || {
        r.set(Some(true));
        if let Some(d) = dw.upgrade() {
            d.close();
        }
    };
    on_click(ok_b, done_ok.clone());
    d.on_ok(done_ok);
    let r = result.clone();
    let dw = Rc::downgrade(&d);
    let done_cancel = move || {
        r.set(Some(false));
        if let Some(d) = dw.upgrade() {
            d.close();
        }
    };
    on_click(cancel_b, done_cancel.clone());
    d.on_cancel(done_cancel);
    if destructive {
        d.set_default(IDCANCEL_);
    }
    d.show();
    if destructive {
        cancel_b.focus();
    } else {
        ok_b.focus();
    }
    let r = result.clone();
    let dw = Rc::downgrade(&d);
    pump_until(move || r.get().is_some() || dw.upgrade().is_none_or(|d| d.is_closed()));
    result.get().unwrap_or(false)
}

/// 只有一个「确定」的提示框
pub fn notice(owner: Option<HWND>, caption: &str, title: &str, body: &str, ok: &str, error: bool) {
    let d = Dialog::new(owner, caption, 360, owner.is_some(), owner.is_none());
    let p = d.panel.hwnd;
    let head = label_font(p, title, Font::Bold);
    let text = label(p, body);
    let ok_b = button_id(p, ok, IDOK_, true);
    let icon = if error { 32513u16 /* IDI_ERROR */ } else { 32516 /* IDI_INFORMATION */ };
    d.set_layout(move |f| {
        let s = px(32);
        f.sys_icon(f.x, f.y, s, icon);
        let (x0, w0) = (f.x, f.w);
        f.x += s + px(14);
        f.w -= s + px(14);
        let top = f.y;
        f.text(head);
        if !text.text().is_empty() {
            f.gap(6);
            f.text(text);
        }
        f.y = f.y.max(top + s);
        f.x = x0;
        f.w = w0;
        f.footer(&[ok_b], &[]);
    });
    let dw = Rc::downgrade(&d);
    let done = move || {
        if let Some(d) = dw.upgrade() {
            d.close();
        }
    };
    on_click(ok_b, done.clone());
    d.on_ok(done.clone());
    d.on_cancel(done);
    d.show();
    ok_b.focus();
    let dw = Rc::downgrade(&d);
    pump_until(move || dw.upgrade().is_none_or(|d| d.is_closed()));
}
