//! 系统声音：PulseAudio 简易接口录默认输出设备的监听源（@DEFAULT_MONITOR@）。PipeWire 系统上由 pipewire-pulse 提供同样的接口。
//! 运行时加载 libpulse-simple.so.0，没有就不支持声音。48 kHz 立体声浮点，每 20 ms 推一次。

use super::encode::dl::Lib;
use std::ffi::{c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[repr(C)]
struct SampleSpec {
    format: c_int,
    rate: u32,
    channels: u8,
}

#[repr(C)]
struct BufferAttr {
    maxlength: u32,
    tlength: u32,
    prebuf: u32,
    minreq: u32,
    fragsize: u32,
}

const PA_STREAM_RECORD: c_int = 2;
const PA_SAMPLE_FLOAT32LE: c_int = 5;
const RATE: u32 = 48_000;
const CHANNELS: u16 = 2;

crate::dl_fns! {
    struct Pulse {
        pa_simple_new: fn(*const c_char, *const c_char, c_int, *const c_char, *const c_char, *const SampleSpec, *const c_void, *const BufferAttr, *mut c_int) -> *mut c_void;
        pa_simple_read: fn(*mut c_void, *mut c_void, usize, *mut c_int) -> c_int;
        pa_simple_free: fn(*mut c_void);
        pa_strerror: fn(c_int) -> *const c_char;
    }
}

fn pulse() -> Result<&'static Pulse, String> {
    use std::sync::OnceLock;
    static P: OnceLock<Result<Pulse, String>> = OnceLock::new();
    P.get_or_init(|| {
        let lib: &'static Lib = Box::leak(Box::new(Lib::open(&["libpulse-simple.so.0", "libpulse-simple.so"])?));
        Pulse::load(lib)
    })
    .as_ref()
    .map_err(|e| format!("没有 PulseAudio 客户端库（{e}）"))
}

struct Stream {
    p: &'static Pulse,
    s: *mut c_void,
}
unsafe impl Send for Stream {}

impl Drop for Stream {
    fn drop(&mut self) {
        unsafe { (self.p.pa_simple_free)(self.s) }
    }
}

fn open() -> Result<Stream, String> {
    let p = pulse()?;
    let ss = SampleSpec { format: PA_SAMPLE_FLOAT32LE, rate: RATE, channels: CHANNELS as u8 };
    let frag = RATE / 50 * CHANNELS as u32 * 4;
    let attr = BufferAttr { maxlength: u32::MAX, tlength: u32::MAX, prebuf: u32::MAX, minreq: u32::MAX, fragsize: frag };
    let mut err: c_int = 0;
    let s = unsafe { (p.pa_simple_new)(std::ptr::null(), c"LemonGalaxy".as_ptr(), PA_STREAM_RECORD, c"@DEFAULT_MONITOR@".as_ptr(), c"remote audio".as_ptr(), &ss, std::ptr::null(), &attr, &mut err) };
    if s.is_null() {
        let msg = unsafe {
            let m = (p.pa_strerror)(err);
            if m.is_null() {
                format!("错误 {err}")
            } else {
                std::ffi::CStr::from_ptr(m).to_string_lossy().into_owned()
            }
        };
        return Err(format!("连不上声音服务：{msg}"));
    }
    Ok(Stream { p, s })
}

/// 开始采集。`push(样本, 采样率, 声道)` 在采集线程里调用。返回采样率与声道数。
pub fn start(stop: Arc<AtomicBool>, push: impl Fn(Vec<f32>, u32, u16) + Send + 'static) -> Result<(u32, u16), String> {
    let st = open()?;
    std::thread::Builder::new()
        .name("audio".into())
        .spawn(move || {
            // 整个 Stream 搬进线程（不只是字段）
            let st = st;
            let n = (RATE / 50) as usize * CHANNELS as usize;
            let mut buf = vec![0f32; n];
            while !stop.load(Ordering::Relaxed) {
                let mut err = 0;
                let r = unsafe { (st.p.pa_simple_read)(st.s, buf.as_mut_ptr() as *mut c_void, n * 4, &mut err) };
                if r < 0 {
                    eprintln!("读声音失败（{err}）");
                    break;
                }
                push(buf.clone(), RATE, CHANNELS);
            }
        })
        .map_err(|e| e.to_string())?;
    Ok((RATE, CHANNELS))
}

/// 自检用：能不能连上声音服务
pub fn probe() -> Result<String, String> {
    let _s = open()?;
    Ok(format!("PulseAudio 接口，{RATE} Hz × {CHANNELS}"))
}

/// 服务这边判断有没有声音能力：只看库在不在（真正连不连得上要在用户会话里才知道）
pub fn library_present() -> bool {
    ["/usr/lib/x86_64-linux-gnu/libpulse-simple.so.0", "/usr/lib/aarch64-linux-gnu/libpulse-simple.so.0", "/usr/lib64/libpulse-simple.so.0", "/usr/lib/libpulse-simple.so.0"].iter().any(|p| std::path::Path::new(p).exists())
}
