//! PipeWire 收屏幕画面（Mutter 与桌面门户都用它）。运行时加载 libpipewire-0.3.so.0，程序不链接它。
//!
//! 结构体布局与常量已在 Ubuntu 24.04（PipeWire 1.0）上用原头文件 sizeof / offsetof 核对过：
//! spa_hook 48、pw_stream_events 96（process 在 64）、spa_buffer 24、spa_data 40（data 在 24、chunk 在 32）、
//! spa_chunk 16、spa_meta 16、spa_meta_cursor 28、spa_meta_bitmap 20。
//!
//! 只要共享内存缓冲（不带 modifier 的格式），收到一帧就拷进共享槽，采集线程从槽里取。
//! 光标走元数据（位置每帧都有，图像换光标时才带）。

use super::super::backend::{CursorImage, CursorPos, Frame};
use super::super::encode::dl::Lib;
use super::spa;
use parking_lot::{Condvar, Mutex};
use std::ffi::{c_char, c_int, c_void};
use std::os::fd::{IntoRawFd, OwnedFd};
use std::sync::Arc;
use std::time::Duration;

crate::dl_fns! {
    struct Pw {
        pw_init: fn(*mut c_int, *mut *mut *mut c_char);
        pw_thread_loop_new: fn(*const c_char, *const c_void) -> *mut c_void;
        pw_thread_loop_get_loop: fn(*mut c_void) -> *mut c_void;
        pw_thread_loop_start: fn(*mut c_void) -> c_int;
        pw_thread_loop_stop: fn(*mut c_void);
        pw_thread_loop_destroy: fn(*mut c_void);
        pw_thread_loop_lock: fn(*mut c_void);
        pw_thread_loop_unlock: fn(*mut c_void);
        pw_context_new: fn(*mut c_void, *mut c_void, usize) -> *mut c_void;
        pw_context_destroy: fn(*mut c_void);
        pw_context_connect: fn(*mut c_void, *mut c_void, usize) -> *mut c_void;
        pw_context_connect_fd: fn(*mut c_void, c_int, *mut c_void, usize) -> *mut c_void;
        pw_core_disconnect: fn(*mut c_void) -> c_int;
        pw_properties_new_string: fn(*const c_char) -> *mut c_void;
        pw_stream_new: fn(*mut c_void, *const c_char, *mut c_void) -> *mut c_void;
        pw_stream_add_listener: fn(*mut c_void, *mut c_void, *const Events, *mut c_void);
        pw_stream_connect: fn(*mut c_void, u32, u32, u32, *mut *const u8, u32) -> c_int;
        pw_stream_disconnect: fn(*mut c_void) -> c_int;
        pw_stream_destroy: fn(*mut c_void);
        pw_stream_dequeue_buffer: fn(*mut c_void) -> *mut PwBuffer;
        pw_stream_queue_buffer: fn(*mut c_void, *mut PwBuffer) -> c_int;
        pw_stream_update_params: fn(*mut c_void, *mut *const u8, u32) -> c_int;
    }
}

#[repr(C)]
pub struct Events {
    version: u32,
    destroy: Option<unsafe extern "C" fn(*mut c_void)>,
    state_changed: Option<unsafe extern "C" fn(*mut c_void, c_int, c_int, *const c_char)>,
    control_info: Option<unsafe extern "C" fn(*mut c_void, u32, *const c_void)>,
    io_changed: Option<unsafe extern "C" fn(*mut c_void, u32, *mut c_void, u32)>,
    param_changed: Option<unsafe extern "C" fn(*mut c_void, u32, *const u8)>,
    add_buffer: Option<unsafe extern "C" fn(*mut c_void, *mut PwBuffer)>,
    remove_buffer: Option<unsafe extern "C" fn(*mut c_void, *mut PwBuffer)>,
    process: Option<unsafe extern "C" fn(*mut c_void)>,
    drained: Option<unsafe extern "C" fn(*mut c_void)>,
    command: Option<unsafe extern "C" fn(*mut c_void, *const c_void)>,
    trigger_done: Option<unsafe extern "C" fn(*mut c_void)>,
}
const _: () = assert!(std::mem::size_of::<Events>() == 96 && std::mem::offset_of!(Events, process) == 64 && std::mem::offset_of!(Events, param_changed) == 40);

#[repr(C)]
pub struct PwBuffer {
    buffer: *mut SpaBuffer,
}

#[repr(C)]
struct SpaBuffer {
    n_metas: u32,
    n_datas: u32,
    metas: *mut SpaMeta,
    datas: *mut SpaData,
}

#[repr(C)]
struct SpaMeta {
    ty: u32,
    size: u32,
    data: *mut c_void,
}

#[repr(C)]
struct SpaData {
    ty: u32,
    flags: u32,
    fd: i64,
    mapoffset: u32,
    maxsize: u32,
    data: *mut c_void,
    chunk: *mut SpaChunk,
}
const _: () = assert!(std::mem::size_of::<SpaData>() == 40 && std::mem::offset_of!(SpaData, data) == 24 && std::mem::offset_of!(SpaData, chunk) == 32);

#[repr(C)]
struct SpaChunk {
    offset: u32,
    size: u32,
    stride: i32,
    flags: i32,
}

#[repr(C)]
struct MetaCursor {
    id: u32,
    flags: u32,
    pos_x: i32,
    pos_y: i32,
    hot_x: i32,
    hot_y: i32,
    bitmap_offset: u32,
}

#[repr(C)]
struct MetaBitmap {
    format: u32,
    width: u32,
    height: u32,
    stride: i32,
    offset: u32,
}

const STREAM_STATE_ERROR: c_int = -1;
const STREAM_STATE_STREAMING: c_int = 3;
const DIRECTION_INPUT: u32 = 0;
const FLAG_AUTOCONNECT: u32 = 1;
const FLAG_MAP_BUFFERS: u32 = 4;

fn pw() -> Result<&'static Pw, String> {
    use std::sync::OnceLock;
    static P: OnceLock<Result<Pw, String>> = OnceLock::new();
    P.get_or_init(|| {
        let lib: &'static Lib = Box::leak(Box::new(Lib::open(&["libpipewire-0.3.so.0", "libpipewire-0.3.so"])?));
        let p = Pw::load(lib)?;
        unsafe { (p.pw_init)(std::ptr::null_mut(), std::ptr::null_mut()) };
        Ok(p)
    })
    .as_ref()
    .map_err(|e| format!("没有 PipeWire 客户端库（{e}）"))
}

#[derive(Default)]
struct Slot {
    /// 协商好的格式：(视频格式, 宽, 高)
    format: Option<(u32, u32, u32)>,
    frame: Frame,
    fresh: bool,
    cursor: Option<CursorPos>,
    cursor_img: Option<CursorImage>,
    cursor_img_fresh: bool,
    error: Option<String>,
    streaming: bool,
}

struct Shared {
    pw: &'static Pw,
    stream: Mutex<*mut c_void>,
    slot: Mutex<Slot>,
    cv: Condvar,
}
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

/// 一路 PipeWire 视频流
pub struct PwStream {
    shared: Arc<Shared>,
    tloop: *mut c_void,
    ctx: *mut c_void,
    core: *mut c_void,
    stream: *mut c_void,
    _hook: Box<[u64; 8]>,
    _events: Box<Events>,
    data: *const Shared,
}
unsafe impl Send for PwStream {}

unsafe extern "C" fn on_state(data: *mut c_void, _old: c_int, state: c_int, err: *const c_char) {
    let s = &*(data as *const Shared);
    let mut g = s.slot.lock();
    if state == STREAM_STATE_ERROR {
        g.error = Some(if err.is_null() { "PipeWire 流出错".into() } else { std::ffi::CStr::from_ptr(err).to_string_lossy().into_owned() });
        s.cv.notify_all();
    }
    g.streaming = state == STREAM_STATE_STREAMING;
}

unsafe extern "C" fn on_param_changed(data: *mut c_void, id: u32, param: *const u8) {
    if param.is_null() || id != spa::PARAM_FORMAT {
        return;
    }
    let s = &*(data as *const Shared);
    let size = u32::from_le_bytes(std::slice::from_raw_parts(param, 4).try_into().unwrap_or([0; 4])) as usize;
    let pod = std::slice::from_raw_parts(param, 8 + size);
    let fmt = spa::prop_u32(pod, spa::FORMAT_VIDEO_FORMAT).unwrap_or(0);
    let (w, h) = spa::prop_rect(pod, spa::FORMAT_VIDEO_SIZE).unwrap_or((0, 0));
    s.slot.lock().format = Some((fmt, w, h));
    // 回复缓冲与元数据要求
    let b = spa::buffers();
    let m = spa::meta_cursor();
    let mut params: [*const u8; 2] = [b.as_ptr(), m.as_ptr()];
    let stream = *s.stream.lock();
    if !stream.is_null() {
        (s.pw.pw_stream_update_params)(stream, params.as_mut_ptr(), 2);
    }
}

unsafe extern "C" fn on_process(data: *mut c_void) {
    let s = &*(data as *const Shared);
    let stream = *s.stream.lock();
    if stream.is_null() {
        return;
    }
    // 只要最新的一帧：先把排着的都取出来
    let mut b = (s.pw.pw_stream_dequeue_buffer)(stream);
    if b.is_null() {
        return;
    }
    loop {
        let next = (s.pw.pw_stream_dequeue_buffer)(stream);
        if next.is_null() {
            break;
        }
        (s.pw.pw_stream_queue_buffer)(stream, b);
        b = next;
    }
    let sb = (*b).buffer;
    if !sb.is_null() {
        let mut g = s.slot.lock();
        copy_buffer(&mut g, &*sb);
        s.cv.notify_all();
    }
    (s.pw.pw_stream_queue_buffer)(stream, b);
}

unsafe fn copy_buffer(g: &mut Slot, sb: &SpaBuffer) {
    // 光标元数据
    for i in 0..sb.n_metas as usize {
        let m = &*sb.metas.add(i);
        if m.ty != spa::META_CURSOR || m.data.is_null() || (m.size as usize) < std::mem::size_of::<MetaCursor>() {
            continue;
        }
        let c = &*(m.data as *const MetaCursor);
        if c.id == 0 {
            continue;
        }
        g.cursor = Some(CursorPos { x: c.pos_x, y: c.pos_y, visible: true });
        if c.bitmap_offset as usize >= std::mem::size_of::<MetaCursor>() && (c.bitmap_offset as usize) + std::mem::size_of::<MetaBitmap>() <= m.size as usize {
            let bm = &*((m.data as *const u8).add(c.bitmap_offset as usize) as *const MetaBitmap);
            if bm.format != 0 && bm.offset > 0 && bm.width > 0 && bm.height > 0 && bm.width <= 512 && bm.height <= 512 {
                let base = (bm as *const MetaBitmap as *const u8).add(bm.offset as usize);
                let stride = if bm.stride > 0 { bm.stride as usize } else { bm.width as usize * 4 };
                let end = c.bitmap_offset as usize + bm.offset as usize + stride * bm.height as usize;
                if end <= m.size as usize {
                    let mut argb = Vec::with_capacity((bm.width * bm.height) as usize);
                    for y in 0..bm.height as usize {
                        let row = std::slice::from_raw_parts(base.add(y * stride), bm.width as usize * 4);
                        for px in row.chunks_exact(4) {
                            argb.push(pixel_to_argb(bm.format, px));
                        }
                    }
                    let serial = g.cursor_img.as_ref().map(|c| c.serial + 1).unwrap_or(1);
                    g.cursor_img = Some(CursorImage { serial, width: bm.width, height: bm.height, hot_x: c.hot_x, hot_y: c.hot_y, argb });
                    g.cursor_img_fresh = true;
                } else if bm.offset == 0 {
                    g.cursor = Some(CursorPos { x: c.pos_x, y: c.pos_y, visible: false });
                }
            }
        }
    }
    if sb.n_datas == 0 {
        return;
    }
    let d = &*sb.datas;
    if d.data.is_null() || d.chunk.is_null() {
        return;
    }
    let ch = &*d.chunk;
    // 只有光标变化的缓冲：没有画面数据
    if ch.size == 0 {
        return;
    }
    let Some((fmt, w, h)) = g.format else { return };
    if w == 0 || h == 0 {
        return;
    }
    let stride = if ch.stride > 0 { ch.stride as usize } else { w as usize * 4 };
    let need = ch.offset as usize + stride * (h as usize - 1) + w as usize * 4;
    if need > d.maxsize as usize {
        return;
    }
    let base = (d.data as *const u8).add(ch.offset as usize);
    g.frame.resize(w, h);
    let row = w as usize * 4;
    for y in 0..h as usize {
        let src = std::slice::from_raw_parts(base.add(y * stride), row);
        let dst = &mut g.frame.data[y * row..(y + 1) * row];
        match fmt {
            spa::VIDEO_FORMAT_BGRX | spa::VIDEO_FORMAT_BGRA => dst.copy_from_slice(src),
            _ => {
                for (o, i) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                    let argb = pixel_to_argb(fmt, i);
                    o.copy_from_slice(&[(argb & 0xff) as u8, (argb >> 8 & 0xff) as u8, (argb >> 16 & 0xff) as u8, 255]);
                }
            }
        }
    }
    g.fresh = true;
}

/// 一个像素（按 SPA 视频格式的字节顺序）→ 预乘 ARGB（光标图像本来就是预乘的；画面不在乎 alpha）
fn pixel_to_argb(fmt: u32, p: &[u8]) -> u32 {
    let (r, g, b, a) = match fmt {
        spa::VIDEO_FORMAT_BGRA => (p[2], p[1], p[0], p[3]),
        spa::VIDEO_FORMAT_BGRX => (p[2], p[1], p[0], 255),
        spa::VIDEO_FORMAT_RGBA => (p[0], p[1], p[2], p[3]),
        spa::VIDEO_FORMAT_RGBX => (p[0], p[1], p[2], 255),
        spa::VIDEO_FORMAT_ARGB => (p[1], p[2], p[3], p[0]),
        spa::VIDEO_FORMAT_XRGB => (p[1], p[2], p[3], 255),
        spa::VIDEO_FORMAT_ABGR => (p[3], p[2], p[1], p[0]),
        spa::VIDEO_FORMAT_XBGR => (p[3], p[2], p[1], 255),
        _ => (p[2], p[1], p[0], p[3]),
    };
    (a as u32) << 24 | (r as u32) << 16 | (g as u32) << 8 | b as u32
}

impl PwStream {
    /// 连到 `node`。`fd` 是门户给的 PipeWire 连接（None 时连用户默认的 PipeWire）。
    pub fn connect(node: u32, fd: Option<OwnedFd>, fps: u32) -> Result<Self, String> {
        let p = pw()?;
        unsafe {
            let tloop = (p.pw_thread_loop_new)(c"lemon-galaxy-pw".as_ptr(), std::ptr::null());
            if tloop.is_null() {
                return Err("建 PipeWire 线程失败".into());
            }
            let ctx = (p.pw_context_new)((p.pw_thread_loop_get_loop)(tloop), std::ptr::null_mut(), 0);
            if ctx.is_null() {
                (p.pw_thread_loop_destroy)(tloop);
                return Err("建 PipeWire 上下文失败".into());
            }
            if (p.pw_thread_loop_start)(tloop) < 0 {
                (p.pw_context_destroy)(ctx);
                (p.pw_thread_loop_destroy)(tloop);
                return Err("启动 PipeWire 线程失败".into());
            }
            (p.pw_thread_loop_lock)(tloop);
            let core = match fd {
                Some(f) => (p.pw_context_connect_fd)(ctx, f.into_raw_fd(), std::ptr::null_mut(), 0),
                None => (p.pw_context_connect)(ctx, std::ptr::null_mut(), 0),
            };
            if core.is_null() {
                (p.pw_thread_loop_unlock)(tloop);
                (p.pw_thread_loop_stop)(tloop);
                (p.pw_context_destroy)(ctx);
                (p.pw_thread_loop_destroy)(tloop);
                return Err("连不上 PipeWire（用户会话里没有 PipeWire 服务？）".into());
            }
            let shared = Arc::new(Shared { pw: p, stream: Mutex::new(std::ptr::null_mut()), slot: Mutex::new(Slot::default()), cv: Condvar::new() });
            // 目标节点用 pw_stream_connect 的节点号参数（target.object 属性要的是对象序号或节点名，不是节点号）
            let props = std::ffi::CString::new("media.type=Video media.category=Capture media.role=Screen").unwrap();
            let stream = (p.pw_stream_new)(core, c"lemon-galaxy-screen".as_ptr(), (p.pw_properties_new_string)(props.as_ptr()));
            if stream.is_null() {
                (p.pw_core_disconnect)(core);
                (p.pw_thread_loop_unlock)(tloop);
                (p.pw_thread_loop_stop)(tloop);
                (p.pw_context_destroy)(ctx);
                (p.pw_thread_loop_destroy)(tloop);
                return Err("建 PipeWire 流失败".into());
            }
            *shared.stream.lock() = stream;
            let events = Box::new(Events { version: 2, destroy: None, state_changed: Some(on_state), control_info: None, io_changed: None, param_changed: Some(on_param_changed), add_buffer: None, remove_buffer: None, process: Some(on_process), drained: None, command: None, trigger_done: None });
            let mut hook: Box<[u64; 8]> = Box::new([0; 8]);
            let data = Arc::as_ptr(&shared);
            (p.pw_stream_add_listener)(stream, hook.as_mut_ptr() as *mut c_void, &*events, data as *mut c_void);
            let fmt = spa::enum_format(fps.clamp(1, 60));
            let mut params: [*const u8; 1] = [fmt.as_ptr()];
            let r = (p.pw_stream_connect)(stream, DIRECTION_INPUT, node, FLAG_AUTOCONNECT | FLAG_MAP_BUFFERS, params.as_mut_ptr(), 1);
            (p.pw_thread_loop_unlock)(tloop);
            let me = Self { shared, tloop, ctx, core, stream, _hook: hook, _events: events, data };
            if r < 0 {
                return Err(format!("连接 PipeWire 流失败（{r}）"));
            }
            Ok(me)
        }
    }

    /// 等下一帧最多 `timeout`。新帧拷进 `out` 返回 Ok(true)。
    pub fn next(&mut self, out: &mut Frame, timeout: Duration) -> Result<bool, String> {
        let mut g = self.shared.slot.lock();
        if !g.fresh && g.error.is_none() {
            self.shared.cv.wait_for(&mut g, timeout);
        }
        if let Some(e) = g.error.clone() {
            return Err(e);
        }
        if !g.fresh {
            return Ok(false);
        }
        g.fresh = false;
        std::mem::swap(out, &mut g.frame);
        // 槽里留同尺寸的缓冲，下一帧直接覆盖
        if g.frame.width != out.width || g.frame.height != out.height {
            g.frame = out.clone();
        }
        Ok(true)
    }

    pub fn size(&self) -> (u32, u32) {
        self.shared.slot.lock().format.map(|(_, w, h)| (w, h)).unwrap_or((0, 0))
    }

    /// 光标位置与新图像（图像只在变化后第一次取时给）
    pub fn cursor(&mut self) -> Option<(CursorPos, Option<CursorImage>)> {
        let mut g = self.shared.slot.lock();
        let pos = g.cursor?;
        let img = if g.cursor_img_fresh {
            g.cursor_img_fresh = false;
            g.cursor_img.clone()
        } else {
            None
        };
        Some((pos, img))
    }
}

impl Drop for PwStream {
    fn drop(&mut self) {
        let p = self.shared.pw;
        unsafe {
            (p.pw_thread_loop_lock)(self.tloop);
            *self.shared.stream.lock() = std::ptr::null_mut();
            (p.pw_stream_disconnect)(self.stream);
            (p.pw_stream_destroy)(self.stream);
            (p.pw_core_disconnect)(self.core);
            (p.pw_thread_loop_unlock)(self.tloop);
            (p.pw_thread_loop_stop)(self.tloop);
            (p.pw_context_destroy)(self.ctx);
            (p.pw_thread_loop_destroy)(self.tloop);
        }
        let _ = self.data;
    }
}
