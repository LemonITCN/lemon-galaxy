//! 没有 WASAPI 的系统（XP / Server 2003）录系统声音：用混音器把录音源切到「立体声混音」，
//! 再用 waveIn 录，录到的就是正在播放的声音。结束时把录音源、静音、音量恢复原样。
//!
//! 立体声混音是声卡驱动提供的（AC'97 / HD Audio 的大多数驱动、VMware 的 Sound Blaster PCI 都有），
//! 找不到就报这台电脑不支持传声音。XP 的录音源是全局的：录的时候本机别的程序录音也会录到这一路。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use windows::core::PSTR;
use windows::Win32::Media::Audio::{
    mixerClose, mixerGetControlDetailsW, mixerGetID, mixerGetLineControlsW, mixerGetLineInfoW, mixerOpen, mixerSetControlDetails, waveInAddBuffer, waveInClose, waveInGetNumDevs, waveInOpen, waveInPrepareHeader, waveInReset,
    waveInStart, waveInUnprepareHeader, CALLBACK_NULL, HMIXER, HMIXEROBJ, HWAVEIN, MIXERCONTROLDETAILS, MIXERCONTROLDETAILS_BOOLEAN, MIXERCONTROLDETAILS_LISTTEXTW, MIXERCONTROLDETAILS_UNSIGNED, MIXERCONTROLW, MIXERCONTROL_CONTROLTYPE_MIXER,
    MIXERCONTROL_CONTROLTYPE_MUTE, MIXERCONTROL_CONTROLTYPE_MUX, MIXERCONTROL_CONTROLTYPE_VOLUME, MIXERLINECONTROLSW, MIXERLINEW, MIXERLINE_COMPONENTTYPE_DST_WAVEIN, MIXERLINE_COMPONENTTYPE_SRC_WAVEOUT, MIXER_GETCONTROLDETAILSF_LISTTEXT,
    MIXER_GETCONTROLDETAILSF_VALUE, MIXER_GETLINECONTROLSF_ONEBYTYPE, MIXER_GETLINEINFOF_COMPONENTTYPE, MIXER_GETLINEINFOF_SOURCE, MIXER_OBJECTF_MIXER, MIXER_OBJECTF_WAVEIN, MIXER_SETCONTROLDETAILSF_VALUE, WAVEFORMATEX, WAVEHDR,
    WAVE_FORMAT_PCM, WHDR_DONE,
};

/// 立体声混音在各家驱动里的名字（按优先级，比较时不分大小写）
const NAMES: &[&str] = &["stereo mix", "立体声混音", "立體聲混音", "what u hear", "wave out mix", "wave-out mix", "stereo mixer", "mono mix", "混音", "sum", "loopback"];

const RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
/// 每块缓冲 20 毫秒，共 10 块
const BUF_FRAMES: usize = 960;
const BUFS: usize = 10;

fn mx(h: HMIXER) -> Option<HMIXEROBJ> {
    Some(HMIXEROBJ(h.0))
}

fn utf16(s: &[u16]) -> String {
    let n = s.iter().position(|c| *c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..n])
}

/// 录音目标线上的选择开关：MUX（单选）或 MIXER（多选），都只选立体声混音
#[derive(Clone, Copy)]
struct Select {
    control: u32,
    items: u32,
}

/// 找到的立体声混音
#[derive(Clone)]
pub struct Found {
    /// waveIn 设备号
    pub wavein: u32,
    mixer: u32,
    /// 立体声混音这条源线的 ID
    line: u32,
    select: Option<Select>,
    pub name: String,
}

struct Mixer(HMIXER);

impl Mixer {
    fn open(id: u32) -> Option<Self> {
        let mut h = HMIXER::default();
        (unsafe { mixerOpen(Some(&mut h as *mut _), id, None, None, MIXER_OBJECTF_MIXER as u32) } == 0).then_some(Self(h))
    }

    /// 某条线上指定类型的第一个控件
    fn control(&self, line: u32, kind: u32) -> Option<MIXERCONTROLW> {
        unsafe {
            let mut c: MIXERCONTROLW = std::mem::zeroed();
            c.cbStruct = std::mem::size_of::<MIXERCONTROLW>() as u32;
            let mut lc: MIXERLINECONTROLSW = std::mem::zeroed();
            lc.cbStruct = std::mem::size_of::<MIXERLINECONTROLSW>() as u32;
            lc.dwLineID = line;
            lc.Anonymous.dwControlType = kind;
            lc.cControls = 1;
            lc.cbmxctrl = std::mem::size_of::<MIXERCONTROLW>() as u32;
            lc.pamxctrl = &mut c;
            (mixerGetLineControlsW(mx(self.0), &mut lc, MIXER_GETLINECONTROLSF_ONEBYTYPE as u32) == 0).then_some(c)
        }
    }

    fn details<T: Copy + Default>(&self, control: u32, items: u32, flags: i32) -> Option<Vec<T>> {
        unsafe {
            let n = items.max(1) as usize;
            let mut v = vec![T::default(); n];
            let mut d: MIXERCONTROLDETAILS = std::mem::zeroed();
            d.cbStruct = std::mem::size_of::<MIXERCONTROLDETAILS>() as u32;
            d.dwControlID = control;
            d.cChannels = 1;
            d.Anonymous.cMultipleItems = items;
            d.cbDetails = std::mem::size_of::<T>() as u32;
            d.paDetails = v.as_mut_ptr() as _;
            (mixerGetControlDetailsW(mx(self.0), &mut d, flags as u32) == 0).then_some(v)
        }
    }

    fn set<T: Copy>(&self, control: u32, items: u32, vals: &[T]) -> bool {
        unsafe {
            let mut d: MIXERCONTROLDETAILS = std::mem::zeroed();
            d.cbStruct = std::mem::size_of::<MIXERCONTROLDETAILS>() as u32;
            d.dwControlID = control;
            d.cChannels = 1;
            d.Anonymous.cMultipleItems = items;
            d.cbDetails = std::mem::size_of::<T>() as u32;
            d.paDetails = vals.as_ptr() as _;
            mixerSetControlDetails(mx(self.0), &d, MIXER_SETCONTROLDETAILSF_VALUE as u32) == 0
        }
    }
}

impl Drop for Mixer {
    fn drop(&mut self) {
        unsafe {
            mixerClose(self.0);
        }
    }
}

impl Default for MixerListText {
    fn default() -> Self {
        Self(unsafe { std::mem::zeroed() })
    }
}
#[derive(Clone, Copy)]
struct MixerListText(MIXERCONTROLDETAILS_LISTTEXTW);

/// 找立体声混音：每个 waveIn 设备对应的混音器里，录音目标线的各个源，按名字（其次按「波形输出」类型）认。
pub fn find() -> Option<Found> {
    let n = unsafe { waveInGetNumDevs() };
    for wavein in 0..n {
        let mut mid = 0u32;
        if unsafe { mixerGetID(Some(HMIXEROBJ(wavein as usize as _)), &mut mid, MIXER_OBJECTF_WAVEIN as u32) } != 0 {
            continue;
        }
        let Some(m) = Mixer::open(mid) else { continue };
        let mut dst: MIXERLINEW = unsafe { std::mem::zeroed() };
        dst.cbStruct = std::mem::size_of::<MIXERLINEW>() as u32;
        dst.dwComponentType = MIXERLINE_COMPONENTTYPE_DST_WAVEIN;
        if unsafe { mixerGetLineInfoW(mx(m.0), &mut dst, MIXER_GETLINEINFOF_COMPONENTTYPE as u32) } != 0 {
            continue;
        }
        let mut sources: Vec<(u32, u32, String)> = Vec::new();
        for s in 0..dst.cConnections {
            let mut src: MIXERLINEW = unsafe { std::mem::zeroed() };
            src.cbStruct = std::mem::size_of::<MIXERLINEW>() as u32;
            src.dwDestination = dst.dwDestination;
            src.dwSource = s;
            if unsafe { mixerGetLineInfoW(mx(m.0), &mut src, MIXER_GETLINEINFOF_SOURCE as u32) } == 0 {
                let name = utf16(&{ src.szName });
                sources.push((src.dwLineID, src.dwComponentType.0, name));
            }
        }
        let pick = NAMES
            .iter()
            .find_map(|n| sources.iter().find(|s| s.2.to_lowercase().contains(n)))
            .or_else(|| sources.iter().find(|s| s.1 == MIXERLINE_COMPONENTTYPE_SRC_WAVEOUT.0));
        let Some((line, _, name)) = pick.cloned() else { continue };
        let select = m
            .control(dst.dwLineID, MIXERCONTROL_CONTROLTYPE_MUX)
            .or_else(|| m.control(dst.dwLineID, MIXERCONTROL_CONTROLTYPE_MIXER))
            .map(|c| Select { control: c.dwControlID, items: c.cMultipleItems });
        return Some(Found { wavein, mixer: mid, line, select, name });
    }
    None
}

/// 采集期间改动过的混音器设置，丢掉时恢复
struct Restore {
    mixer: Mixer,
    select: Option<(Select, Vec<MIXERCONTROLDETAILS_BOOLEAN>)>,
    mute: Option<(u32, Vec<MIXERCONTROLDETAILS_BOOLEAN>)>,
    volume: Option<(u32, Vec<MIXERCONTROLDETAILS_UNSIGNED>)>,
}

impl Drop for Restore {
    fn drop(&mut self) {
        if let Some((s, v)) = &self.select {
            self.mixer.set(s.control, s.items, v);
        }
        if let Some((c, v)) = &self.mute {
            self.mixer.set(*c, 0, v);
        }
        if let Some((c, v)) = &self.volume {
            self.mixer.set(*c, 0, v);
        }
    }
}

/// 把录音源切到立体声混音并取消静音（音量为 0 时调到一半）
fn route(f: &Found) -> Result<Restore, String> {
    let mixer = Mixer::open(f.mixer).ok_or("打开混音器失败")?;
    let mut r = Restore { mixer, select: None, mute: None, volume: None };
    if let Some(s) = f.select {
        let names: Vec<MixerListText> = r.mixer.details(s.control, s.items, MIXER_GETCONTROLDETAILSF_LISTTEXT).ok_or("读取录音源列表失败")?;
        let old: Vec<MIXERCONTROLDETAILS_BOOLEAN> = r.mixer.details(s.control, s.items, MIXER_GETCONTROLDETAILSF_VALUE).ok_or("读取录音源失败")?;
        // 只选立体声混音；多选开关（MIXER）也把别的源（比如麦克风）关掉，免得混进来
        let new: Vec<MIXERCONTROLDETAILS_BOOLEAN> = names.iter().map(|n| MIXERCONTROLDETAILS_BOOLEAN { fValue: (n.0.dwParam1 == f.line) as i32 }).collect();
        if !r.mixer.set(s.control, s.items, &new) {
            return Err("切换录音源失败".into());
        }
        r.select = Some((s, old));
    }
    if let Some(c) = r.mixer.control(f.line, MIXERCONTROL_CONTROLTYPE_MUTE) {
        if let Some(old) = r.mixer.details::<MIXERCONTROLDETAILS_BOOLEAN>(c.dwControlID, 0, MIXER_GETCONTROLDETAILSF_VALUE) {
            if old.iter().any(|b| b.fValue != 0) && r.mixer.set(c.dwControlID, 0, &[MIXERCONTROLDETAILS_BOOLEAN { fValue: 0 }]) {
                r.mute = Some((c.dwControlID, old));
            }
        }
    }
    if let Some(c) = r.mixer.control(f.line, MIXERCONTROL_CONTROLTYPE_VOLUME) {
        if let Some(old) = r.mixer.details::<MIXERCONTROLDETAILS_UNSIGNED>(c.dwControlID, 0, MIXER_GETCONTROLDETAILSF_VALUE) {
            let (lo, hi) = unsafe { (c.Bounds.Anonymous2.dwMinimum, c.Bounds.Anonymous2.dwMaximum) };
            if old.iter().all(|v| v.dwValue <= lo) && hi > lo && r.mixer.set(c.dwControlID, 0, &[MIXERCONTROLDETAILS_UNSIGNED { dwValue: lo + (hi - lo) / 2 }]) {
                r.volume = Some((c.dwControlID, old));
            }
        }
    }
    Ok(r)
}

/// 一块录音缓冲：头和数据放一起，地址不变（交给驱动后不能挪）
#[repr(C, align(8))]
struct Buf {
    hdr: WAVEHDR,
    data: [i16; BUF_FRAMES * CHANNELS as usize],
}

struct WaveIn {
    h: HWAVEIN,
    bufs: Vec<Box<Buf>>,
}

impl WaveIn {
    fn open(dev: u32) -> Result<Self, String> {
        let fmt = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_PCM as u16,
            nChannels: CHANNELS,
            nSamplesPerSec: RATE,
            nAvgBytesPerSec: RATE * CHANNELS as u32 * 2,
            nBlockAlign: CHANNELS * 2,
            wBitsPerSample: 16,
            cbSize: 0,
        };
        let mut h = HWAVEIN::default();
        let rc = unsafe { waveInOpen(Some(&mut h as *mut _), dev, &fmt, None, None, CALLBACK_NULL) };
        if rc != 0 {
            return Err(format!("打开录音设备失败（{rc}）"));
        }
        let mut w = Self { h, bufs: Vec::new() };
        for _ in 0..BUFS {
            let mut b: Box<Buf> = Box::new(unsafe { std::mem::zeroed() });
            b.hdr.lpData = PSTR(b.data.as_mut_ptr() as *mut u8);
            b.hdr.dwBufferLength = std::mem::size_of_val(&b.data) as u32;
            unsafe {
                if waveInPrepareHeader(h, &mut b.hdr, std::mem::size_of::<WAVEHDR>() as u32) != 0 || waveInAddBuffer(h, &mut b.hdr, std::mem::size_of::<WAVEHDR>() as u32) != 0 {
                    return Err("准备录音缓冲失败".into());
                }
            }
            w.bufs.push(b);
        }
        if unsafe { waveInStart(h) } != 0 {
            return Err("开始录音失败".into());
        }
        Ok(w)
    }

    /// 取走录满的缓冲（按交给驱动的顺序），转成浮点样本，再交回去
    fn drain(&mut self, out: &mut Vec<f32>) {
        for _ in 0..BUFS {
            let b = &mut self.bufs[0];
            let flags = unsafe { std::ptr::addr_of!(b.hdr.dwFlags).read_volatile() };
            if flags & WHDR_DONE == 0 {
                break;
            }
            let n = (b.hdr.dwBytesRecorded as usize / 2).min(b.data.len());
            out.extend(b.data[..n].iter().map(|s| *s as f32 / 32768.0));
            b.hdr.dwFlags = flags & !WHDR_DONE;
            b.hdr.dwBytesRecorded = 0;
            unsafe {
                waveInAddBuffer(self.h, &mut b.hdr, std::mem::size_of::<WAVEHDR>() as u32);
            }
            self.bufs.rotate_left(1);
        }
    }
}

impl Drop for WaveIn {
    fn drop(&mut self) {
        unsafe {
            waveInReset(self.h);
            for b in &mut self.bufs {
                waveInUnprepareHeader(self.h, &mut b.hdr, std::mem::size_of::<WAVEHDR>() as u32);
            }
            waveInClose(self.h);
        }
    }
}

/// 开始采集（同 audio::start）：先同步打开，返回采样率与声道数；之后在后台线程里推样本，`stop` 置位时结束并恢复混音器。
pub fn start(stop: Arc<AtomicBool>, push: impl Fn(Vec<f32>, u32, u16) + Send + 'static) -> Result<(u32, u16), String> {
    let f = find().ok_or("这台电脑的声卡没有「立体声混音」录音源，传不了声音")?;
    let (tx, rx) = std::sync::mpsc::channel::<Result<(u32, u16), String>>();
    std::thread::spawn(move || {
        let restore = match route(&f) {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        let mut w = match WaveIn::open(f.wavein) {
            Ok(w) => w,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        tracing::info!("声音采集：没有 WASAPI，录「{}」（waveIn {}）", f.name, f.wavein);
        let _ = tx.send(Ok((RATE, CHANNELS)));
        let mut buf = Vec::with_capacity(BUF_FRAMES * CHANNELS as usize * BUFS);
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(10));
            w.drain(&mut buf);
            if !buf.is_empty() {
                push(std::mem::take(&mut buf), RATE, CHANNELS);
            }
        }
        drop(w);
        drop(restore);
    });
    rx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| Err("打开声音设备超时".into()))
}
