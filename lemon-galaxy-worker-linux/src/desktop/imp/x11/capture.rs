//! X11 截屏（每块屏一个采集器，各用自己的连接）。

use super::super::backend::{Capturer, CursorImage, CursorPos, Frame, Monitor};
use super::connect;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::damage::{self, ConnectionExt as _};
use x11rb::protocol::shm::{self, ConnectionExt as _};
use x11rb::protocol::xfixes::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat, Window};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

enum Shm {
    /// memfd 传给服务器（MIT-SHM 1.2）
    Fd { seg: u32, ptr: *mut u8, len: usize },
    /// SysV 共享内存
    SysV { seg: u32, ptr: *mut u8 },
    /// 没有共享内存：普通 GetImage（慢，远程 X 或扩展被禁用时）
    None,
}

pub struct X11Capturer {
    conn: RustConnection,
    screen: usize,
    root: Window,
    mon: Monitor,
    is_virtual: bool,
    shm: Shm,
    damage: Option<u32>,
    /// Damage 报过变化还没截
    dirty: bool,
    /// 上次无条件截屏比较的时间（合成器绘制偶尔不报 Damage，每秒兜底一次）
    last_full: Instant,
    xfixes: bool,
    cursor_dirty: bool,
    /// 根窗口像素是 RGB 顺序（红在低字节），要换成 BGRA
    swap_rb: bool,
    last_geom: Instant,
}

// 原始指针只在采集线程里用
unsafe impl Send for X11Capturer {}

fn memfd(len: usize) -> Option<(OwnedFd, *mut u8)> {
    unsafe {
        // 直接系统调用：glibc 2.27 之前没有 memfd_create 包装函数
        let fd = libc::syscall(libc::SYS_memfd_create, c"lemon-galaxy-x11".as_ptr(), libc::MFD_CLOEXEC as libc::c_uint) as i32;
        if fd < 0 {
            return None;
        }
        let fd = OwnedFd::from_raw_fd(fd);
        if libc::ftruncate(fd.as_raw_fd(), len as libc::off_t) != 0 {
            return None;
        }
        let p = libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd.as_raw_fd(), 0);
        if p == libc::MAP_FAILED {
            return None;
        }
        Some((fd, p as *mut u8))
    }
}

impl X11Capturer {
    pub fn new(mon: Monitor, is_virtual: bool) -> Result<Self, String> {
        let (conn, screen) = connect()?;
        let s = conn.setup().roots[screen].clone();
        let root = s.root;
        let visual = s.allowed_depths.iter().flat_map(|d| d.visuals.iter()).find(|v| v.visual_id == s.root_visual).cloned();
        let swap_rb = visual.map(|v| v.red_mask == 0xff).unwrap_or(false);
        let damage = if conn.extension_information(damage::X11_EXTENSION_NAME).ok().flatten().is_some() && std::env::var("LEMON_GALAXY_X11_DAMAGE").map(|v| v != "0").unwrap_or(true) {
            let ok = conn.damage_query_version(1, 1).ok().and_then(|c| c.reply().ok()).is_some();
            if ok {
                let id = conn.generate_id().map_err(|e| e.to_string())?;
                conn.damage_create(id, root, damage::ReportLevel::NON_EMPTY).map_err(|e| e.to_string())?;
                Some(id)
            } else {
                None
            }
        } else {
            None
        };
        let xfixes = conn.extension_information(xfixes::X11_EXTENSION_NAME).ok().flatten().is_some() && conn.xfixes_query_version(4, 0).ok().and_then(|c| c.reply().ok()).is_some();
        if xfixes {
            let _ = conn.xfixes_select_cursor_input(root, xfixes::CursorNotifyMask::DISPLAY_CURSOR);
        }
        // 显示器增删、分辨率变化
        let _ = x11rb::protocol::randr::ConnectionExt::randr_select_input(&conn, root, x11rb::protocol::randr::NotifyMask::SCREEN_CHANGE);
        let mut me = Self { conn, screen, root, mon, is_virtual, shm: Shm::None, damage, dirty: true, last_full: Instant::now(), xfixes, cursor_dirty: true, swap_rb, last_geom: Instant::now() };
        me.alloc_shm();
        let _ = me.conn.flush();
        Ok(me)
    }

    fn free_shm(&mut self) {
        match std::mem::replace(&mut self.shm, Shm::None) {
            Shm::Fd { seg, ptr, len } => unsafe {
                let _ = self.conn.shm_detach(seg);
                libc::munmap(ptr as *mut _, len);
            },
            Shm::SysV { seg, ptr } => unsafe {
                let _ = self.conn.shm_detach(seg);
                libc::shmdt(ptr as *const _);
            },
            Shm::None => {}
        }
        let _ = self.conn.flush();
    }

    fn alloc_shm(&mut self) {
        self.free_shm();
        let len = self.mon.width as usize * self.mon.height as usize * 4;
        if self.conn.extension_information(shm::X11_EXTENSION_NAME).ok().flatten().is_none() || std::env::var("LEMON_GALAXY_X11_SHM").is_ok_and(|v| v == "0") {
            return;
        }
        let Some(ver) = self.conn.shm_query_version().ok().and_then(|c| c.reply().ok()) else { return };
        let Ok(seg) = self.conn.generate_id() else { return };
        // memfd（1.2 起）
        if (ver.major_version, ver.minor_version) >= (1, 2) {
            if let Some((fd, ptr)) = memfd(len) {
                let dup = unsafe { libc::dup(fd.as_raw_fd()) };
                if dup >= 0 {
                    let dup = unsafe { OwnedFd::from_raw_fd(dup) };
                    let ok = self.conn.shm_attach_fd(seg, dup, false).ok().map(|c| c.check().is_ok()).unwrap_or(false);
                    if ok {
                        self.shm = Shm::Fd { seg, ptr, len };
                        return;
                    }
                }
                unsafe { libc::munmap(ptr as *mut _, len) };
            }
        }
        // SysV
        unsafe {
            let id = libc::shmget(libc::IPC_PRIVATE, len, libc::IPC_CREAT | 0o600);
            if id < 0 {
                return;
            }
            let p = libc::shmat(id, std::ptr::null(), 0);
            if p as isize == -1 {
                libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut());
                return;
            }
            let ok = self.conn.shm_attach(seg, id as u32, false).ok().map(|c| c.check().is_ok()).unwrap_or(false);
            // 服务器挂上后就可以标记删除，两边都脱离时系统回收
            libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut());
            if ok {
                self.shm = Shm::SysV { seg, ptr: p as *mut u8 };
            } else {
                libc::shmdt(p);
            }
        }
    }

    /// 处理已到的事件；返回连接是否还好
    fn drain(&mut self) -> Result<(), String> {
        loop {
            match self.conn.poll_for_event() {
                Ok(Some(ev)) => match ev {
                    Event::DamageNotify(_) => self.dirty = true,
                    Event::XfixesCursorNotify(_) => self.cursor_dirty = true,
                    Event::RandrScreenChangeNotify(_) => self.last_geom = Instant::now() - Duration::from_secs(10),
                    _ => {}
                },
                Ok(None) => return Ok(()),
                Err(e) => return Err(format!("X 连接断了：{e}")),
            }
        }
    }

    /// 等事件（Damage、光标）最多 `d`
    fn wait(&mut self, d: Duration) -> Result<(), String> {
        self.drain()?;
        if self.dirty || d.is_zero() {
            return Ok(());
        }
        let mut pfd = libc::pollfd { fd: self.conn.stream().as_raw_fd(), events: libc::POLLIN, revents: 0 };
        unsafe { libc::poll(&mut pfd, 1, d.as_millis().min(1000) as i32) };
        if pfd.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            return Err("X 连接断了".into());
        }
        self.drain()
    }

    /// 显示器的位置尺寸变了：更新并重建共享内存
    fn check_geometry(&mut self) -> Result<(), String> {
        if self.last_geom.elapsed() < Duration::from_secs(2) {
            return Ok(());
        }
        self.last_geom = Instant::now();
        let list = super::monitors(&self.conn, self.screen, self.is_virtual);
        let m = list.iter().find(|m| m.id == self.mon.id).or_else(|| list.iter().find(|m| m.primary)).cloned().ok_or("屏幕没了")?;
        if (m.x, m.y, m.width, m.height) != (self.mon.x, self.mon.y, self.mon.width, self.mon.height) {
            self.mon = m;
            self.alloc_shm();
            self.dirty = true;
        }
        Ok(())
    }

    /// 截一张到 `out`（BGRA）。返回和 `out` 原来的内容比有没有变。
    fn grab(&mut self, out: &mut Frame) -> Result<bool, String> {
        let (w, h) = (self.mon.width, self.mon.height);
        let (x, y) = (self.mon.x as i16, self.mon.y as i16);
        let len = w as usize * h as usize * 4;
        let src: &[u8];
        let owned;
        match self.shm {
            Shm::Fd { seg, ptr, .. } | Shm::SysV { seg, ptr } => {
                self.conn.shm_get_image(self.root, x, y, w as u16, h as u16, !0, ImageFormat::Z_PIXMAP.into(), seg, 0).map_err(|e| e.to_string())?.reply().map_err(|e| format!("截屏失败：{e}"))?;
                src = unsafe { std::slice::from_raw_parts(ptr, len) };
            }
            Shm::None => {
                let r = self.conn.get_image(ImageFormat::Z_PIXMAP, self.root, x, y, w as u16, h as u16, !0).map_err(|e| e.to_string())?.reply().map_err(|e| format!("截屏失败：{e}"))?;
                owned = r.data;
                if owned.len() < len {
                    return Err("截屏数据不完整".into());
                }
                src = &owned[..len];
            }
        }
        let same_size = out.width == w && out.height == h;
        if self.swap_rb {
            out.resize(w, h);
            let mut changed = !same_size;
            for (d, s) in out.data.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                let px = [s[2], s[1], s[0], 255];
                if !changed && d != px {
                    changed = true;
                }
                d.copy_from_slice(&px);
            }
            return Ok(changed);
        }
        if same_size && out.data[..] == src[..] {
            return Ok(false);
        }
        out.resize(w, h);
        out.data.copy_from_slice(src);
        Ok(true)
    }
}

impl Capturer for X11Capturer {
    fn next(&mut self, frame: &mut Frame, timeout: Duration) -> Result<bool, String> {
        self.check_geometry()?;
        let full_due = self.last_full.elapsed() >= Duration::from_secs(1);
        if self.damage.is_some() && !self.dirty && !full_due && frame.width == self.mon.width {
            let left = Duration::from_secs(1).saturating_sub(self.last_full.elapsed());
            self.wait(timeout.min(left))?;
            if !self.dirty {
                return Ok(false);
            }
        }
        // 先清掉 Damage 区域再截：截屏期间的新变化会再报一次
        if let Some(d) = self.damage {
            let _ = self.conn.damage_subtract(d, x11rb::NONE, x11rb::NONE);
        }
        self.dirty = false;
        self.last_full = Instant::now();
        let changed = self.grab(frame)?;
        if self.damage.is_none() && !changed {
            // 没有 Damage 扩展：按帧率轮询，这里稍等一下免得空转
            std::thread::sleep(timeout.min(Duration::from_millis(16)));
        }
        Ok(changed)
    }

    fn size(&self) -> (u32, u32) {
        (self.mon.width, self.mon.height)
    }

    fn cursor(&mut self) -> Option<(CursorPos, Option<CursorImage>)> {
        let p = self.conn.query_pointer(self.root).ok()?.reply().ok()?;
        let (x, y) = (p.root_x as i32 - self.mon.x, p.root_y as i32 - self.mon.y);
        let visible = p.same_screen && x >= 0 && y >= 0 && x < self.mon.width as i32 && y < self.mon.height as i32;
        let pos = CursorPos { x, y, visible };
        let img = if self.xfixes && self.cursor_dirty {
            self.cursor_dirty = false;
            self.conn.xfixes_get_cursor_image().ok().and_then(|c| c.reply().ok()).map(|r| CursorImage { serial: r.cursor_serial as u64, width: r.width as u32, height: r.height as u32, hot_x: r.xhot as i32, hot_y: r.yhot as i32, argb: r.cursor_image })
        } else {
            None
        };
        Some((pos, img))
    }

    fn kind(&self) -> &'static str {
        match self.shm {
            Shm::None => "x11-getimage",
            _ => "x11-shm",
        }
    }
}

impl Drop for X11Capturer {
    fn drop(&mut self) {
        if let Some(d) = self.damage.take() {
            let _ = self.conn.damage_destroy(d);
        }
        self.free_shm();
    }
}
