//! 页面容器与布局：Win32 没有布局引擎，这里每次（改大小、滚动、内容变了）按当前状态把控件重新摆一遍。
//!
//! 页面给一个布局函数（`Panel::set_layout`），里面按从上到下的顺序「摆」控件（`Flow`）；这一遍没摆到的控件自动藏起来，
//! 所以显示 / 隐藏只要在布局函数里按状态决定摆不摆。分组框的边框、行间分隔线、提示条底色、徽标、状态点这些不是控件，
//! 记成装饰（`Decor`）由容器自己画。

use super::*;
use std::rc::Weak;

#[derive(Clone, Debug)]
pub enum Decor {
    /// 1 像素边框
    Frame(RECT),
    /// 横线 (x1, x2, y)
    Line(i32, i32, i32),
    Fill(RECT, u32),
    /// 圆角小标签：文字、前景、底色
    Badge(RECT, String, u32, u32),
    Dot(RECT, u32),
    /// 程序图标（边长）
    AppIcon(i32, i32, i32),
    /// 系统图标（IDI_WARNING 等）
    SysIcon(i32, i32, i32, u16),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// 行右侧的东西
pub enum Right<'a> {
    C(Ctl),
    /// 徽标（画出来的，不是控件）
    Badge(&'a str, Tone),
}

pub struct Panel {
    pub hwnd: HWND,
    scrollable: bool,
    /// 内容少于可见高度时上下居中
    pub center: Cell<bool>,
    /// 内容最宽（像素），左右留白
    pub max_w: Cell<i32>,
    /// (左右, 上下) 边距（像素）
    pub pad: Cell<(i32, i32)>,
    layout: RefCell<Option<Rc<dyn Fn(&mut Flow)>>>,
    scroll: Cell<i32>,
    content_h: Cell<i32>,
    decor: RefCell<Vec<Decor>>,
    in_layout: Cell<bool>,
    again: Cell<bool>,
    queued: Cell<bool>,
}

impl Panel {
    pub fn new(parent: HWND, scrollable: bool) -> Rc<Panel> {
        let cls = wide(CLASS_PANEL);
        let mut style = WS_CHILD.0 | WS_CLIPCHILDREN.0;
        if scrollable {
            style |= WS_VSCROLL.0;
        }
        let h = unsafe { CreateWindowExW(WS_EX_CONTROLPARENT, pcw(&cls), PCWSTR::null(), WINDOW_STYLE(style), 0, 0, 0, 0, Some(parent), None, Some(hinstance()), None).unwrap_or_default() };
        set_bg(h, sys(COLOR_WINDOW));
        if scrollable {
            unsafe {
                let _ = ShowScrollBar(h, SB_VERT, false);
            }
        }
        let p = Rc::new(Panel {
            hwnd: h,
            scrollable,
            center: Cell::new(false),
            max_w: Cell::new(px(720)),
            pad: Cell::new((px(28), px(24))),
            layout: RefCell::new(None),
            scroll: Cell::new(0),
            content_h: Cell::new(0),
            decor: RefCell::new(vec![]),
            in_layout: Cell::new(false),
            again: Cell::new(false),
            queued: Cell::new(false),
        });
        let w: Weak<Panel> = Rc::downgrade(&p);
        set_proc(h, move |h, msg, wp, lp| {
            let p = w.upgrade()?;
            p.handle(h, msg, wp, lp)
        });
        p
    }

    pub fn set_layout(&self, f: impl Fn(&mut Flow) + 'static) {
        *self.layout.borrow_mut() = Some(Rc::new(f));
    }

    pub fn show(&self, on: bool) {
        Ctl(self.hwnd).show(on);
    }

    pub fn move_to(&self, x: i32, y: i32, w: i32, h: i32) {
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    /// 稍后重新布局（同一轮消息里多次调用只做一次）
    pub fn relayout_soon(self: &Rc<Self>) {
        if self.queued.replace(true) {
            return;
        }
        let w = Rc::downgrade(self);
        defer(move || {
            if let Some(p) = w.upgrade() {
                p.queued.set(false);
                p.relayout();
            }
        });
    }

    /// 给定宽度时内容多高（对话框按内容定高度用）
    pub fn measure(&self, width: i32) -> i32 {
        let Some(f) = self.layout.borrow().clone() else { return 0 };
        let mut flow = self.flow(width, 10_000, 0);
        f(&mut flow);
        flow.y + self.pad.get().1
    }

    fn flow(&self, width: i32, height: i32, offset: i32) -> Flow {
        let (px_, py) = self.pad.get();
        let w = (width - 2 * px_).min(self.max_w.get()).max(px(200));
        let x = ((width - w) / 2).max(px_.min(width / 10));
        Flow { x, w, y: py + offset, cw: width, view_h: height, pad_bottom: py, placed: vec![], decor: vec![] }
    }

    pub fn relayout(&self) {
        if self.in_layout.replace(true) {
            self.again.set(true);
            return;
        }
        let Some(f) = self.layout.borrow().clone() else {
            self.in_layout.set(false);
            return;
        };
        let mut guard = 0;
        loop {
            self.again.set(false);
            guard += 1;
            let rc = client_rect(self.hwnd);
            let (cw, ch) = (rc.right - rc.left, rc.bottom - rc.top);
            let mut flow = self.flow(cw, ch, -self.scroll.get());
            f(&mut flow);
            let total = flow.y + self.scroll.get() + self.pad.get().1;
            if self.center.get() && total < ch {
                let extra = (ch - total) / 2;
                flow = self.flow(cw, ch, extra);
                f(&mut flow);
            }
            if self.scrollable {
                let max = (total - ch).max(0);
                if self.scroll.get() > max {
                    self.scroll.set(max);
                    if guard < 4 {
                        continue;
                    }
                }
                let si = SCROLLINFO {
                    cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
                    nMin: 0,
                    nMax: (total - 1).max(0),
                    nPage: ch.max(0) as u32,
                    nPos: self.scroll.get(),
                    nTrackPos: 0,
                };
                unsafe { SetScrollInfo(self.hwnd, SB_VERT, &si, true) };
                // 滚动条出现 / 消失会改变宽度（WM_SIZE 里要求再排一次）
                if self.again.get() && guard < 4 {
                    continue;
                }
            }
            self.content_h.set(total);
            self.apply(flow);
            break;
        }
        self.in_layout.set(false);
    }

    fn apply(&self, flow: Flow) {
        let Flow { placed, decor, .. } = flow;
        let mut kids: Vec<HWND> = vec![];
        unsafe extern "system" fn collect(h: HWND, lp: LPARAM) -> windows::core::BOOL {
            let v = unsafe { &mut *(lp.0 as *mut (HWND, Vec<HWND>)) };
            if unsafe { GetParent(h) }.ok() == Some(v.0) {
                v.1.push(h);
            }
            true.into()
        }
        let mut st = (self.hwnd, std::mem::take(&mut kids));
        unsafe {
            let _ = EnumChildWindows(Some(self.hwnd), Some(collect), LPARAM(&mut st as *mut _ as isize));
        }
        let kids = st.1;
        unsafe {
            let mut dwp = BeginDeferWindowPos((kids.len() + 1) as i32).ok();
            let mut put = |h: HWND, r: Option<&RECT>| {
                let (x, y, w, hh, flags) = match r {
                    Some(r) => (r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW),
                    None => (0, 0, 0, 0, SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_HIDEWINDOW),
                };
                if let Some(d) = dwp {
                    dwp = DeferWindowPos(d, h, None, x, y, w, hh, flags).ok();
                }
                if dwp.is_none() {
                    let _ = SetWindowPos(h, None, x, y, w, hh, flags);
                }
            };
            for k in &kids {
                match placed.iter().rev().find(|(h, _)| h == k) {
                    Some((_, r)) => put(*k, Some(r)),
                    None => {
                        if IsWindowVisible(*k).as_bool() {
                            put(*k, None)
                        }
                    }
                }
            }
            if let Some(d) = dwp {
                let _ = EndDeferWindowPos(d);
            }
        }
        *self.decor.borrow_mut() = decor;
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn scroll_to(&self, pos: i32) {
        let rc = client_rect(self.hwnd);
        let max = (self.content_h.get() - (rc.bottom - rc.top)).max(0);
        let pos = pos.clamp(0, max);
        if pos != self.scroll.get() {
            self.scroll.set(pos);
            self.relayout();
            unsafe {
                let _ = UpdateWindow(self.hwnd);
            }
        }
    }

    fn handle(&self, h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_SIZE => {
                self.relayout();
                Some(LRESULT(0))
            }
            WM_ERASEBKGND => Some(LRESULT(1)),
            WM_PAINT => {
                unsafe {
                    let mut ps = PAINTSTRUCT::default();
                    let dc = BeginPaint(h, &mut ps);
                    let rc = client_rect(h);
                    // 双缓冲，免得闪
                    let mem = CreateCompatibleDC(Some(dc));
                    let bmp = CreateCompatibleBitmap(dc, rc.right.max(1), rc.bottom.max(1));
                    let old = SelectObject(mem, HGDIOBJ(bmp.0));
                    self.paint(mem, &rc);
                    let _ = BitBlt(dc, 0, 0, rc.right, rc.bottom, Some(mem), 0, 0, SRCCOPY);
                    SelectObject(mem, old);
                    let _ = DeleteObject(HGDIOBJ(bmp.0));
                    let _ = DeleteDC(mem);
                    let _ = EndPaint(h, &ps);
                }
                Some(LRESULT(0))
            }
            // 主题控件（复选框、按钮圆角）画透明部分时要父窗口的底
            WM_PRINTCLIENT => {
                let rc = client_rect(h);
                self.paint(HDC(wp.0 as _), &rc);
                Some(LRESULT(0))
            }
            WM_VSCROLL if self.scrollable => {
                let rc = client_rect(h);
                let page = rc.bottom - rc.top;
                let line = line_h() * 3;
                let cur = self.scroll.get();
                let pos = match SCROLLBAR_COMMAND(loword(wp.0) as i32) {
                    SB_LINEUP => cur - line,
                    SB_LINEDOWN => cur + line,
                    SB_PAGEUP => cur - page,
                    SB_PAGEDOWN => cur + page,
                    SB_TOP => 0,
                    SB_BOTTOM => i32::MAX / 2,
                    SB_THUMBTRACK | SB_THUMBPOSITION => {
                        let mut si = SCROLLINFO { cbSize: std::mem::size_of::<SCROLLINFO>() as u32, fMask: SIF_TRACKPOS, ..Default::default() };
                        unsafe {
                            let _ = GetScrollInfo(h, SB_VERT, &mut si);
                        }
                        si.nTrackPos
                    }
                    _ => cur,
                };
                self.scroll_to(pos);
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL if self.scrollable => {
                let rc = client_rect(h);
                if self.content_h.get() <= rc.bottom - rc.top {
                    return None;
                }
                let delta = hiword(wp.0) as u16 as i16 as i32;
                let mut lines = 3u32;
                unsafe {
                    let _ = SystemParametersInfoW(SPI_GETWHEELSCROLLLINES, 0, Some(&mut lines as *mut u32 as _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
                }
                let step = line_h() * lines.clamp(1, 10) as i32;
                self.scroll_to(self.scroll.get() - delta * step / 120);
                Some(LRESULT(0))
            }
            _ => {
                let _ = lp;
                None
            }
        }
    }

    fn paint(&self, dc: HDC, rc: &RECT) {
        unsafe {
            FillRect(dc, rc, brush(bg_of(self.hwnd)));
            let line = line_color();
            let pen = CreatePen(PS_SOLID, 1, COLORREF(line));
            let old_pen = SelectObject(dc, HGDIOBJ(pen.0));
            let old_font = SelectObject(dc, HGDIOBJ(font(Font::Bold).0));
            SetBkMode(dc, TRANSPARENT);
            for d in self.decor.borrow().iter() {
                match d {
                    Decor::Frame(r) => {
                        let old_b = SelectObject(dc, GetStockObject(NULL_BRUSH));
                        let _ = Rectangle(dc, r.left, r.top, r.right, r.bottom);
                        SelectObject(dc, old_b);
                    }
                    Decor::Line(x1, x2, y) => {
                        let _ = MoveToEx(dc, *x1, *y, None);
                        let _ = LineTo(dc, *x2, *y);
                    }
                    Decor::Fill(r, c) => {
                        FillRect(dc, r, brush(*c));
                    }
                    Decor::Badge(r, text, fg, bg) => {
                        let np = SelectObject(dc, GetStockObject(NULL_PEN));
                        let ob = SelectObject(dc, HGDIOBJ(brush(*bg).0));
                        let rr = r.bottom - r.top;
                        let _ = RoundRect(dc, r.left, r.top, r.right + 1, r.bottom + 1, rr, rr);
                        SelectObject(dc, ob);
                        SelectObject(dc, np);
                        SetTextColor(dc, COLORREF(*fg));
                        let mut t: Vec<u16> = text.encode_utf16().collect();
                        let mut r2 = *r;
                        DrawTextW(dc, &mut t, &mut r2, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
                    }
                    Decor::Dot(r, c) => {
                        let np = SelectObject(dc, GetStockObject(NULL_PEN));
                        let ob = SelectObject(dc, HGDIOBJ(brush(*c).0));
                        let _ = Ellipse(dc, r.left, r.top, r.right + 1, r.bottom + 1);
                        SelectObject(dc, ob);
                        SelectObject(dc, np);
                    }
                    Decor::SysIcon(x, y, size, id) => {
                        if let Ok(icon) = LoadIconW(None, PCWSTR(*id as usize as *const u16)) {
                            let _ = DrawIconEx(dc, *x, *y, icon, *size, *size, 0, None, DI_NORMAL);
                        }
                    }
                    Decor::AppIcon(x, y, size) => {
                        let icon = app_icon(*size);
                        if !icon.0.is_null() {
                            let _ = DrawIconEx(dc, *x, *y, icon, *size, *size, 0, None, DI_NORMAL);
                            let _ = DestroyIcon(icon);
                        }
                    }
                }
            }
            SelectObject(dc, old_font);
            SelectObject(dc, old_pen);
            let _ = DeleteObject(HGDIOBJ(pen.0));
        }
    }
}

/// 一遍布局：当前位置 `y`，内容区左边 `x`、宽 `w`（都是像素，已含滚动偏移）
pub struct Flow {
    pub x: i32,
    pub w: i32,
    pub y: i32,
    /// 容器的整个宽度（铺满左右的装饰用）
    pub cw: i32,
    /// 容器可见高度
    pub view_h: i32,
    pad_bottom: i32,
    placed: Vec<(HWND, RECT)>,
    decor: Vec<Decor>,
}

const GAP: i32 = 8;

impl Flow {
    pub fn place(&mut self, c: Ctl, x: i32, y: i32, w: i32, h: i32) {
        // 下拉框的高度是展开后的列表高度
        let h = if c.class().eq_ignore_ascii_case("combobox") { h + px(220) } else { h };
        self.placed.push((c.0, RECT { left: x, top: y, right: x + w.max(0), bottom: y + h.max(0) }));
    }

    pub fn decor(&mut self, d: Decor) {
        self.decor.push(d);
    }

    pub fn gap(&mut self, dip: i32) {
        self.y += px(dip);
    }

    /// 整行宽、自动折行的文字
    pub fn text(&mut self, c: Ctl) {
        let w = self.w;
        self.text_at(c, self.x, w);
    }

    pub fn text_at(&mut self, c: Ctl, x: i32, w: i32) {
        let (_, h) = measure(c.get_font(), &c.text(), w, true);
        self.place(c, x, self.y, w, h);
        self.y += h;
    }

    /// 居中的文字（控件要是 SS_CENTER），最宽 `max_w` 像素
    pub fn center_text(&mut self, c: Ctl, max_w: i32) {
        let w = self.w.min(max_w);
        let x = self.x + (self.w - w) / 2;
        self.text_at(c, x, w);
    }

    /// 一排控件（按首选大小），垂直居中
    pub fn line(&mut self, items: &[Ctl], align: Align) {
        if items.is_empty() {
            return;
        }
        let sizes: Vec<(i32, i32)> = items.iter().map(|c| pref(*c)).collect();
        let total: i32 = sizes.iter().map(|s| s.0).sum::<i32>() + px(GAP) * (items.len() as i32 - 1);
        let h = sizes.iter().map(|s| s.1).max().unwrap_or(0);
        let mut x = match align {
            Align::Left => self.x,
            Align::Center => self.x + (self.w - total) / 2,
            Align::Right => self.x + self.w - total,
        };
        for (c, (w, ch)) in items.iter().zip(sizes) {
            self.place(*c, x, self.y + (h - ch) / 2, w, ch);
            x += w + px(GAP);
        }
        self.y += h;
    }

    /// 标题行：左边标题（可带「i」），右边一排控件
    pub fn title(&mut self, title: Ctl, info: Option<Ctl>, right: &[Ctl]) {
        let sizes: Vec<(i32, i32)> = right.iter().map(|c| pref(*c)).collect();
        let rw: i32 = sizes.iter().map(|s| s.0).sum::<i32>() + px(GAP) * (right.len() as i32 - 1).max(0);
        let info_w = if info.is_some() { px(16 + 6) } else { 0 };
        let avail = self.w - rw - if right.is_empty() { 0 } else { px(16) } - info_w;
        let (tw, th) = measure(title.get_font(), &title.text(), avail, true);
        let rh = sizes.iter().map(|s| s.1).max().unwrap_or(0);
        let h = th.max(rh);
        let ty = self.y + (h - th) / 2;
        self.place(title, self.x, ty, tw, th);
        if let Some(i) = info {
            let first = measure(title.get_font(), "Ag", 10_000, false).1;
            self.place(i, self.x + tw + px(6), ty + (first - px(16)) / 2, px(16), px(16));
        }
        let mut x = self.x + self.w - rw;
        for (c, (w, ch)) in right.iter().zip(sizes) {
            self.place(*c, x, self.y + (h - ch) / 2, w, ch);
            x += w + px(GAP);
        }
        self.y += h;
    }

    /// 分组：带边框的一组行
    pub fn group(&mut self, f: impl FnOnce(&mut Rows)) {
        let top = self.y;
        let mut rows = Rows { f: self, n: 0 };
        f(&mut rows);
        let n = rows.n;
        if n == 0 {
            return;
        }
        let (x, w) = (self.x, self.w);
        self.decor.push(Decor::Frame(RECT { left: x, top, right: x + w, bottom: self.y + 1 }));
        self.y += 1;
    }

    /// 提示条：左边色条 + 文字 + 右边按钮
    pub fn banner(&mut self, tone: Tone, text: Ctl, button: Option<Ctl>) {
        let fill = tone_fill(tone);
        set_ctl_bg(text, Some(fill));
        let (padx, pady) = (px(12), px(9));
        let bs = button.map(pref);
        let bw = bs.map(|s| s.0 + px(12)).unwrap_or(0);
        let tw = self.w - 2 * padx - px(4) - bw;
        let (_, th) = measure(text.get_font(), &text.text(), tw, true);
        let h = th.max(bs.map(|s| s.1).unwrap_or(0)) + 2 * pady;
        let r = RECT { left: self.x, top: self.y, right: self.x + self.w, bottom: self.y + h };
        self.decor.push(Decor::Fill(r, fill));
        self.decor.push(Decor::Fill(RECT { left: r.left, top: r.top, right: r.left + px(3), bottom: r.bottom }, tone_color(tone)));
        self.place(text, self.x + padx + px(4), self.y + (h - th) / 2, tw, th);
        if let (Some(b), Some((w, bh))) = (button, bs) {
            self.place(b, self.x + self.w - padx - w, self.y + (h - bh) / 2, w, bh);
        }
        self.y += h;
    }

    /// 占满剩下的高度（日志列表）
    pub fn fill(&mut self, c: Ctl, min_h: i32) {
        let h = (self.view_h - self.y - self.pad_bottom).max(min_h);
        self.place(c, self.x, self.y, self.w, h);
        self.y += h;
    }

    /// 表单的一行：左边标签（右对齐的列宽 `label_w`），中间输入框占满，右边可带「i」
    pub fn field(&mut self, label: Ctl, label_w: i32, input: Ctl, info: Option<Ctl>) {
        let (_, ih) = pref(input);
        let (_, lh) = measure(label.get_font(), &label.text(), label_w, true);
        let h = ih.max(lh);
        self.place(label, self.x, self.y + (h - lh) / 2, label_w, lh);
        let info_w = px(16 + 8);
        let iw = self.w - label_w - px(10) - info_w;
        let iw = fixed_width(input).map(|w| w.min(iw)).unwrap_or(iw);
        self.place(input, self.x + label_w + px(10), self.y + (h - ih) / 2, iw, ih);
        if let Some(i) = info {
            self.place(i, self.x + label_w + px(10) + iw + px(8), self.y + (h - px(16)) / 2, px(16), px(16));
        }
        self.y += h;
    }

    /// 对话框底部的按钮条：灰底铺满左右、一直到窗口底边，按钮靠右
    pub fn footer(&mut self, buttons: &[Ctl], left: &[Ctl]) {
        self.y += px(18);
        let pad = px(11);
        let sizes: Vec<(i32, i32)> = buttons.iter().map(|c| pref(*c)).collect();
        let h = sizes.iter().map(|s| s.1).max().unwrap_or(button_h());
        let top = self.y;
        self.decor.push(Decor::Fill(RECT { left: 0, top, right: self.cw, bottom: top + h + 2 * pad + self.pad_bottom + px(4) }, sys(COLOR_BTNFACE)));
        self.decor.push(Decor::Line(0, self.cw, top));
        let total: i32 = sizes.iter().map(|s| s.0).sum::<i32>() + px(GAP) * (buttons.len() as i32 - 1).max(0);
        let mut x = self.x + self.w - total;
        for (c, (w, ch)) in buttons.iter().zip(sizes) {
            self.place(*c, x, top + pad + (h - ch) / 2, w, ch);
            x += w + px(GAP);
        }
        let mut lx = self.x;
        for c in left {
            let (w, ch) = pref(*c);
            set_ctl_bg(*c, Some(sys(COLOR_BTNFACE)));
            self.place(*c, lx, top + pad + (h - ch) / 2, w, ch);
            lx += w + px(GAP);
        }
        self.y = top + h + 2 * pad - self.pad_bottom;
    }

    pub fn sys_icon(&mut self, x: i32, y: i32, size: i32, id: u16) {
        self.decor.push(Decor::SysIcon(x, y, size, id));
    }

    pub fn app_icon(&mut self, x: i32, y: i32, size: i32) {
        self.decor.push(Decor::AppIcon(x, y, size));
    }

    pub fn dot(&mut self, x: i32, y: i32, size: i32, color: u32) {
        self.decor.push(Decor::Dot(RECT { left: x, top: y, right: x + size, bottom: y + size }, color));
    }

    pub fn badge_size(text: &str) -> (i32, i32) {
        let (tw, th) = measure(font(Font::Bold), text, 10_000, false);
        (tw + px(14), th + px(4))
    }

    pub fn badge(&mut self, x: i32, y: i32, text: &str, tone: Tone) -> i32 {
        let (w, h) = Self::badge_size(text);
        self.decor.push(Decor::Badge(RECT { left: x, top: y, right: x + w, bottom: y + h }, text.to_string(), tone_color(tone), tone_fill(tone)));
        w
    }
}

/// 分组里的行
pub struct Rows<'a> {
    f: &'a mut Flow,
    n: usize,
}

impl Rows<'_> {
    fn sep(&mut self) {
        if self.n > 0 {
            let (x, w, y) = (self.f.x, self.f.w, self.f.y);
            self.f.decor.push(Decor::Line(x + 1, x + w - 1, y));
        }
        self.n += 1;
    }

    /// 一行：左边标题（文字或复选框，可带「i」）与灰色副标题，右边一排控件 / 徽标
    pub fn row(&mut self, label: Ctl, info: Option<Ctl>, sub: Option<Ctl>, right: &[Right]) {
        self.sep();
        let f = &mut *self.f;
        let (padx, pady) = (px(14), px(9));
        let ix = f.x + padx;
        let iw = f.w - 2 * padx;
        let sizes: Vec<(i32, i32)> = right
            .iter()
            .map(|r| match r {
                Right::C(c) => pref(*c),
                Right::Badge(t, _) => Flow::badge_size(t),
            })
            .collect();
        let rw: i32 = sizes.iter().map(|s| s.0).sum::<i32>() + px(GAP) * (right.len() as i32 - 1).max(0);
        let rh = sizes.iter().map(|s| s.1).max().unwrap_or(0);
        let avail = (iw - rw - if right.is_empty() { 0 } else { px(16) }).max(px(80));
        let info_w = if info.is_some() { px(16 + 6) } else { 0 };
        let is_check = label.class().eq_ignore_ascii_case("button");
        let (lw, lh) = if is_check {
            let (w, h) = pref(label);
            (w.min(avail - info_w), h)
        } else {
            let natural = measure(label.get_font(), &label.text(), 10_000, false).0;
            let (_, h) = measure(label.get_font(), &label.text(), avail - info_w, true);
            (natural.min(avail - info_w), h)
        };
        let sh = sub.map(|s| measure(s.get_font(), &s.text(), avail, true).1).unwrap_or(0);
        let content = lh + if sub.is_some() { px(2) + sh } else { 0 };
        let h = content.max(rh).max(px(22)) + 2 * pady;
        let cy = f.y + (h - content) / 2;
        f.place(label, ix, cy, lw, lh);
        if let Some(i) = info {
            let first = measure(label.get_font(), "Ag", 10_000, false).1;
            let iy = cy + (first.min(lh) - px(16)) / 2 + if is_check { px(1) } else { 0 };
            f.place(i, ix + lw + px(6), iy, px(16), px(16));
        }
        if let Some(s) = sub {
            // 复选框的副标题与文字对齐（让过方框）
            let sx = if is_check { ix + px(18) } else { ix };
            f.place(s, sx, cy + lh + px(2), avail - (sx - ix), sh);
        }
        let mut x = ix + iw - rw;
        for (r, (w, ch)) in right.iter().zip(sizes) {
            let y = f.y + (h - ch) / 2;
            match r {
                Right::C(c) => f.place(*c, x, y, w, ch),
                Right::Badge(t, tone) => {
                    f.badge(x, y, t, *tone);
                }
            }
            x += w + px(GAP);
        }
        f.y += h;
    }

    /// 整行一段文字（空列表的提示等）
    pub fn note(&mut self, c: Ctl) {
        self.sep();
        let f = &mut *self.f;
        let (padx, pady) = (px(14), px(11));
        let w = f.w - 2 * padx;
        let (_, h) = measure(c.get_font(), &c.text(), w, true);
        f.place(c, f.x + padx, f.y + pady, w, h);
        f.y += h + 2 * pady;
    }
}
