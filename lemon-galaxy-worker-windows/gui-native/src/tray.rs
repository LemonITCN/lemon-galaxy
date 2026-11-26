//! 托盘图标（Shell_NotifyIconW，结构体按 XP 认的 V2 大小）：状态画在标志右下角的小圆点上（绿在线、橙被控、
//! 红禁止或服务没运行），没连上服务时整个标志变灰；提示文字写谁在控制。右键是系统原生菜单：逐个断开、全部断开、
//! 停止 AI、允许远程（打勾）、临时协助（开着时显示设备码与验证码，点一下复制）、打开主界面、退出界面。
//! 单击 / 双击打开主窗口；有人连入、请求协助时发气泡通知。资源管理器重启后自动重新加上。

use crate::control::*;
use crate::tr;
use crate::ui::{app_name, overall, overall_text, who, Ctx, Overall, Page};
use crate::win::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

mod pixels {
    include!(concat!(env!("OUT_DIR"), "/icon_pixels.rs"));
}

const TRAY_ID: u32 = 1;

pub struct Tray {
    ctx: Rc<Ctx>,
    open: Rc<dyn Fn(Option<Page>)>,
    quit: Rc<dyn Fn()>,
    icon: Cell<isize>,
    added: Cell<bool>,
    look: Cell<Option<Overall>>,
    tip: RefCell<String>,
    balloon_action: RefCell<Option<Rc<dyn Fn()>>>,
    taskbar_created: u32,
}

/// XP 认的大小：到 guidItem 之前（V2，带气泡字段，不带 Vista 起才有的 guidItem、hBalloonIcon）
fn nid_size() -> u32 {
    std::mem::offset_of!(NOTIFYICONDATAW, guidItem) as u32
}

fn fill<const N: usize>(s: &str) -> [u16; N] {
    let mut a = [0u16; N];
    for (i, c) in s.encode_utf16().take(N - 1).enumerate() {
        a[i] = c;
    }
    a
}

/// 托盘提示最多 127 个字符（XP 是 63），超长截断
fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut t: String = s.chars().take(n - 1).collect();
    t.push('…');
    t
}

impl Tray {
    pub fn new(ctx: &Rc<Ctx>, open: Rc<dyn Fn(Option<Page>)>, quit: Rc<dyn Fn()>) -> Rc<Tray> {
        let msg = wide("TaskbarCreated");
        let tc = unsafe { RegisterWindowMessageW(pcw(&msg)) };
        let t = Rc::new(Tray { ctx: ctx.clone(), open, quit, icon: Cell::new(0), added: Cell::new(false), look: Cell::new(None), tip: RefCell::new(String::new()), balloon_action: RefCell::new(None), taskbar_created: tc });
        let w = Rc::downgrade(&t);
        on_dispatcher(move |h, msg, wp, lp| w.upgrade()?.handle(h, msg, wp, lp));
        t.update();
        t
    }

    pub fn added(&self) -> bool {
        self.added.get()
    }

    fn base(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW { cbSize: nid_size(), hWnd: dispatcher(), uID: TRAY_ID, ..Default::default() }
    }

    /// 加上图标（资源管理器还没起来时失败，之后定时重试、TaskbarCreated 时再加）
    pub fn add(&self) {
        if self.added.get() {
            return;
        }
        let mut n = self.base();
        n.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        n.uCallbackMessage = WM_APP_TRAY;
        n.hIcon = HICON(self.icon.get() as _);
        n.szTip = fill(&clip(&self.tip.borrow(), if crate::win::compat::legacy() { 63 } else { 127 }));
        if unsafe { Shell_NotifyIconW(NIM_ADD, &n) }.as_bool() {
            self.added.set(true);
            let mut v = self.base();
            v.Anonymous.uVersion = NOTIFYICON_VERSION;
            unsafe {
                let _ = Shell_NotifyIconW(NIM_SETVERSION, &v);
            }
        }
    }

    pub fn remove(&self) {
        if self.added.replace(false) {
            let n = self.base();
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &n);
            }
        }
    }

    /// 状态变了：换图标、提示文字
    pub fn update(&self) {
        let o = overall(&self.ctx);
        let tip = format!("{} · {}", app_name(), self.tip_text(o));
        let tip_changed = *self.tip.borrow() != tip;
        *self.tip.borrow_mut() = tip;
        let look_changed = self.look.get() != Some(o);
        if look_changed {
            self.look.set(Some(o));
            let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
            let new = compose(o, size);
            let old = self.icon.replace(new.0 as isize);
            if old != 0 {
                unsafe {
                    let _ = DestroyIcon(HICON(old as _));
                }
            }
        }
        if !self.added.get() {
            self.add();
            return;
        }
        if look_changed || tip_changed {
            let mut n = self.base();
            n.uFlags = NIF_ICON | NIF_TIP;
            n.hIcon = HICON(self.icon.get() as _);
            n.szTip = fill(&clip(&self.tip.borrow(), if crate::win::compat::legacy() { 63 } else { 127 }));
            unsafe {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &n);
            }
        }
    }

    fn tip_text(&self, o: Overall) -> String {
        let st = self.ctx.status.borrow();
        match st.as_ref().map(|s| s.sessions.len()) {
            Some(1) => {
                let s = st.as_ref().unwrap();
                crate::i18n::fmt(tr!("{0} 正在控制", "{0} is controlling this computer"), &[&who(&s.sessions[0].from_user)])
            }
            Some(n) if n > 1 => crate::i18n::fmt(tr!("{0} 个连接正在控制", "{0} sessions are controlling this computer"), &[&n.to_string()]),
            _ => overall_text(o).to_string(),
        }
    }

    /// 气泡通知；`action`：点气泡时做的事（没有就打开主窗口）。没有托盘时返回 false
    pub fn balloon(&self, title: &str, text: &str, warning: bool, action: Option<Rc<dyn Fn()>>) -> bool {
        if !self.added.get() {
            return false;
        }
        *self.balloon_action.borrow_mut() = action;
        let mut n = self.base();
        n.uFlags = NIF_INFO;
        n.szInfo = fill(text);
        n.szInfoTitle = fill(title);
        n.dwInfoFlags = if warning { NIIF_WARNING } else { NIIF_INFO };
        n.Anonymous.uTimeout = 6000;
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &n).as_bool() }
    }

    fn handle(&self, _h: HWND, msg: u32, _wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        if msg == self.taskbar_created && msg != 0 {
            // 资源管理器重启了：重新加上
            self.added.set(false);
            self.add();
            return Some(LRESULT(0));
        }
        if msg != WM_APP_TRAY {
            return None;
        }
        let ev = loword(lp.0 as usize);
        match ev {
            WM_LBUTTONUP | WM_LBUTTONDBLCLK | NIN_SELECT | 0x401 /* NIN_KEYSELECT */ => (self.open)(None),
            WM_CONTEXTMENU => self.menu(),
            NIN_BALLOONUSERCLICK => {
                let a = self.balloon_action.borrow_mut().take();
                match a {
                    Some(a) => a(),
                    None => (self.open)(None),
                }
            }
            _ => {}
        }
        Some(LRESULT(0))
    }

    fn menu(&self) {
        let mut items: Vec<Rc<dyn Fn()>> = vec![];
        unsafe {
            let Ok(m) = CreatePopupMenu() else { return };
            let add = |items: &mut Vec<Rc<dyn Fn()>>, text: &str, enabled: bool, checked: bool, f: Rc<dyn Fn()>| {
                items.push(f);
                let w = wide(text);
                let mut flags = MF_STRING;
                if !enabled {
                    flags |= MF_GRAYED;
                }
                if checked {
                    flags |= MF_CHECKED;
                }
                let _ = AppendMenuW(m, flags, items.len(), pcw(&w));
            };
            let sep = || {
                let _ = AppendMenuW(m, MF_SEPARATOR, 0, windows::core::PCWSTR::null());
            };
            let head = wide(&self.tip.borrow());
            let _ = AppendMenuW(m, MF_STRING | MF_GRAYED, 0, pcw(&head));
            let st = self.ctx.status.borrow().clone();
            if let Some(s) = &st {
                let r = &s.rights;
                if !s.sessions.is_empty() {
                    sep();
                    for x in &s.sessions {
                        let (c, id) = (self.ctx.clone(), x.id.clone());
                        add(&mut items, &crate::i18n::fmt(tr!("断开 {0}", "Disconnect {0}"), &[&who(&x.from_user)]), r.operate, false, Rc::new(move || c.act(Request::Disconnect { session_id: id.clone() }, |_| {})));
                    }
                    if s.sessions.len() > 1 {
                        let c = self.ctx.clone();
                        add(&mut items, tr!("全部断开", "Disconnect All"), r.operate, false, Rc::new(move || c.act(Request::DisconnectAll, |_| {})));
                    }
                    if s.sessions.iter().any(|x| x.ai_active) {
                        let c = self.ctx.clone();
                        add(&mut items, tr!("停止 AI 操作", "Stop AI"), r.operate, false, Rc::new(move || c.act(Request::StopAi, |_| {})));
                    }
                }
                sep();
                let (c, on) = (self.ctx.clone(), s.remote_enabled);
                add(&mut items, tr!("允许远程控制", "Allow remote control"), r.can_manage() || (r.operate && on), on, Rc::new(move || c.act(Request::SetRemoteEnabled { enabled: !on }, |_| {})));
                match &s.assist {
                    Some(a) => {
                        if a.mine {
                            let text = format!("{} {}", a.device_code.replace(' ', ""), a.code);
                            let c = self.ctx.clone();
                            add(&mut items, &crate::i18n::fmt(tr!("设备码 {0} · 验证码 {1}（点击复制）", "Device code {0} · code {1} (click to copy)"), &[&a.device_code, &a.code]), true, false, Rc::new(move || crate::ui::copy(&c, &text)));
                        }
                        let c = self.ctx.clone();
                        add(&mut items, tr!("结束临时协助", "End Assistance"), r.operate, false, Rc::new(move || c.act(Request::AssistStop, |_| {})));
                    }
                    None => {
                        let o = self.open.clone();
                        add(&mut items, tr!("临时协助…", "Assistance…"), r.operate, false, Rc::new(move || o(Some(Page::Assist))));
                    }
                }
            }
            sep();
            let o = self.open.clone();
            add(&mut items, tr!("打开主界面", "Open LemonGalaxy Worker"), true, false, Rc::new(move || o(None)));
            let _ = SetMenuDefaultItem(m, items.len() as u32, 0);
            let q = self.quit.clone();
            add(&mut items, tr!("退出界面", "Exit App"), true, false, Rc::new(move || q()));

            // 先把调度窗口设成前台，菜单点别处才会收起（托盘菜单的老规矩）
            let p = cursor_pos();
            let _ = SetForegroundWindow(dispatcher());
            let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_RIGHTALIGN, p.x, p.y, None, dispatcher(), None);
            let _ = PostMessageW(Some(dispatcher()), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(m);
            let i = cmd.0 as usize;
            if i >= 1 {
                if let Some(f) = items.get(i - 1).cloned() {
                    f();
                }
            }
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
        let h = self.icon.get();
        if h != 0 {
            unsafe {
                let _ = DestroyIcon(HICON(h as _));
            }
        }
    }
}

// ---------------- 图标 ----------------

/// 标志的像素（RGBA），按需缩放到 `size`
fn base_pixels(size: u32) -> Vec<[f32; 4]> {
    let list = pixels::PIXELS;
    let (src_size, src) = list.iter().find(|(s, _)| *s >= size).or_else(|| list.last()).map(|(s, p)| (*s, *p)).unwrap_or((0, &[]));
    let n = size as usize;
    let mut out = vec![[0f32; 4]; n * n];
    if src_size == 0 {
        return out;
    }
    let ss = src_size as usize;
    // 面积平均缩小（像素是直通 Alpha：先按 Alpha 预乘再平均）
    let scale = ss as f32 / n as f32;
    for y in 0..n {
        for x in 0..n {
            let (x0, x1) = (x as f32 * scale, (x + 1) as f32 * scale);
            let (y0, y1) = (y as f32 * scale, (y + 1) as f32 * scale);
            let mut acc = [0f32; 4];
            let mut area = 0f32;
            let mut sy = y0.floor() as usize;
            while (sy as f32) < y1 && sy < ss {
                let hy = (y1.min(sy as f32 + 1.0) - y0.max(sy as f32)).max(0.0);
                let mut sx = x0.floor() as usize;
                while (sx as f32) < x1 && sx < ss {
                    let wx = (x1.min(sx as f32 + 1.0) - x0.max(sx as f32)).max(0.0);
                    let wgt = wx * hy;
                    let p = &src[(sy * ss + sx) * 4..];
                    let a = p[3] as f32 / 255.0;
                    acc[0] += p[0] as f32 * a * wgt;
                    acc[1] += p[1] as f32 * a * wgt;
                    acc[2] += p[2] as f32 * a * wgt;
                    acc[3] += a * wgt;
                    area += wgt;
                    sx += 1;
                }
                sy += 1;
            }
            if area > 0.0 && acc[3] > 0.0 {
                out[y * n + x] = [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3], acc[3] / area];
            }
        }
    }
    out
}

fn hex(c: u32) -> [f32; 3] {
    [((c >> 16) & 0xff) as f32, ((c >> 8) & 0xff) as f32, (c & 0xff) as f32]
}

/// 标志 + 状态点。没连上服务时整体变灰
fn compose(o: Overall, size: i32) -> HICON {
    let n = size as usize;
    let mut px_ = base_pixels(size as u32);
    let gray = matches!(o, Overall::NotRunning | Overall::Connecting);
    if gray {
        for p in px_.iter_mut() {
            let l = 0.3 * p[0] + 0.59 * p[1] + 0.11 * p[2];
            let l = 110.0 + l * 0.45;
            *p = [l, l, l, p[3]];
        }
    }
    let dot = match o {
        Overall::Online => Some(0x34C77B),
        Overall::Controlled | Overall::Ai => Some(0xF0A43A),
        Overall::Disabled | Overall::NotRunning => Some(0xF2555A),
        _ => None,
    };
    if let Some(c) = dot {
        let s = size as f32;
        let r = s * 0.2;
        let ring = (s * 0.07).max(1.0);
        let (cx, cy) = (s - r - ring * 0.5, s - r - ring * 0.5);
        let fill_c = hex(c);
        let ring_c = hex(0x0A1719);
        for y in 0..n {
            for x in 0..n {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                let outer = (r + ring + 0.5 - d).clamp(0.0, 1.0);
                if outer <= 0.0 {
                    continue;
                }
                let inner = (r + 0.5 - d).clamp(0.0, 1.0);
                // 圆点：里面是状态色，外圈一道深色描边，与底图叠在一起
                let col = [ring_c[0] + (fill_c[0] - ring_c[0]) * inner, ring_c[1] + (fill_c[1] - ring_c[1]) * inner, ring_c[2] + (fill_c[2] - ring_c[2]) * inner];
                let p = &mut px_[y * n + x];
                let a = outer + p[3] * (1.0 - outer);
                if a > 0.0 {
                    for k in 0..3 {
                        p[k] = (col[k] * outer + p[k] * p[3] * (1.0 - outer)) / a;
                    }
                }
                p[3] = a;
            }
        }
    }
    unsafe {
        let bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: size, biHeight: -size, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(color) = CreateDIBSection(None, &bi, DIB_RGB_COLORS, &mut bits, None, 0) else { return app_icon(size) };
        let dst = std::slice::from_raw_parts_mut(bits as *mut u8, n * n * 4);
        for (i, p) in px_.iter().enumerate() {
            dst[i * 4] = p[2].round().clamp(0.0, 255.0) as u8;
            dst[i * 4 + 1] = p[1].round().clamp(0.0, 255.0) as u8;
            dst[i * 4 + 2] = p[0].round().clamp(0.0, 255.0) as u8;
            dst[i * 4 + 3] = (p[3] * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        let mask_bytes = vec![0u8; n.div_ceil(16) * 2 * n];
        let mask = CreateBitmap(size, size, 1, 1, Some(mask_bytes.as_ptr() as _));
        let ii = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&ii).unwrap_or_else(|_| app_icon(size));
        let _ = DeleteObject(HGDIOBJ(color.0));
        let _ = DeleteObject(HGDIOBJ(mask.0));
        icon
    }
}
