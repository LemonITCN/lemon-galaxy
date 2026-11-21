//! 系统声音采集：WASAPI 环回（录下默认输出设备正在播放的声音）。在画面代理里运行（用户会话）。
//!
//! 没有声音时 WASAPI 不给数据，这时什么也不发；默认输出设备变了（插拔耳机等）自动换过去。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::core::GUID;
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, WAVEFORMATEX,
    WAVEFORMATEXTENSIBLE,
};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED};

const WAVE_FORMAT_PCM: u16 = 1;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
const SUBTYPE_PCM: GUID = GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);
const SUBTYPE_IEEE_FLOAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SampleKind {
    F32,
    I16,
    I32,
}

fn com_init() {
    // 已经初始化过（或线程是 STA）也没关系，WASAPI 两种都能用
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

fn default_render() -> windows::core::Result<IMMDevice> {
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        en.GetDefaultAudioEndpoint(eRender, eConsole)
    }
}

fn device_id(d: &IMMDevice) -> String {
    unsafe {
        match d.GetId() {
            Ok(p) => {
                let s = p.to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0 as _));
                s
            }
            Err(_) => String::new(),
        }
    }
}

/// 这台电脑有没有声音输出设备（决定是否告诉主控端能传声音）。服务进程里也能调。
pub fn has_render_device() -> bool {
    com_init();
    default_render().is_ok()
}

/// 当前默认输出设备上打开的一路环回采集。
struct Loopback {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    id: String,
    rate: u32,
    channels: u16,
    kind: SampleKind,
}

impl Loopback {
    fn open() -> Result<Self, String> {
        unsafe {
            let dev = default_render().map_err(|_| "这台电脑没有声音输出设备".to_string())?;
            let id = device_id(&dev);
            let client: IAudioClient = dev.Activate(CLSCTX_ALL, None).map_err(|e| format!("打开声音设备失败：{e}"))?;
            let fmt = client.GetMixFormat().map_err(|e| format!("读取声音格式失败：{e}"))?;
            let r = (|| {
                let wf: WAVEFORMATEX = *fmt;
                let (tag, bits) = (wf.wFormatTag, wf.wBitsPerSample);
                let sub = if tag == WAVE_FORMAT_EXTENSIBLE && wf.cbSize >= 22 {
                    let ext = std::ptr::read_unaligned(fmt as *const WAVEFORMATEXTENSIBLE);
                    Some(ext.SubFormat)
                } else {
                    None
                };
                let float = tag == WAVE_FORMAT_IEEE_FLOAT || sub == Some(SUBTYPE_IEEE_FLOAT);
                let pcm = tag == WAVE_FORMAT_PCM || sub == Some(SUBTYPE_PCM);
                let kind = match (float, pcm, bits) {
                    (true, _, 32) => SampleKind::F32,
                    (_, true, 16) => SampleKind::I16,
                    (_, true, 32) => SampleKind::I32,
                    _ => return Err(format!("不支持的声音格式（格式 {tag:#x}，{bits} 位）")),
                };
                // 200 毫秒缓冲；环回模式只能用共享模式
                client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, 2_000_000, 0, fmt, None).map_err(|e| format!("开始声音采集失败：{e}"))?;
                Ok((kind, wf.nSamplesPerSec, wf.nChannels))
            })();
            CoTaskMemFree(Some(fmt as _));
            let (kind, rate, channels) = r?;
            let capture: IAudioCaptureClient = client.GetService().map_err(|e| format!("开始声音采集失败：{e}"))?;
            client.Start().map_err(|e| format!("开始声音采集失败：{e}"))?;
            Ok(Self { client, capture, id, rate, channels, kind })
        }
    }

    /// 取走当前所有可读的数据，转成浮点样本追加到 `out`。
    fn drain(&self, out: &mut Vec<f32>) -> windows::core::Result<()> {
        unsafe {
            let mut packet = self.capture.GetNextPacketSize()?;
            while packet > 0 {
                let mut data: *mut u8 = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                self.capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                let n = frames as usize * self.channels as usize;
                if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null() {
                    out.extend(std::iter::repeat_n(0.0, n));
                } else {
                    match self.kind {
                        SampleKind::F32 => out.extend_from_slice(std::slice::from_raw_parts(data as *const f32, n)),
                        SampleKind::I16 => out.extend(std::slice::from_raw_parts(data as *const i16, n).iter().map(|s| *s as f32 / 32768.0)),
                        SampleKind::I32 => out.extend(std::slice::from_raw_parts(data as *const i32, n).iter().map(|s| *s as f32 / 2_147_483_648.0)),
                    }
                }
                self.capture.ReleaseBuffer(frames)?;
                packet = self.capture.GetNextPacketSize()?;
            }
            Ok(())
        }
    }
}

impl Drop for Loopback {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
        }
    }
}

/// 开始采集。先同步打开一次，结果（采样率、声道数或错误）立即返回；之后在后台线程里持续推样本，`stop` 置位时结束。
pub fn start(stop: Arc<AtomicBool>, push: impl Fn(Vec<f32>, u32, u16) + Send + 'static) -> Result<(u32, u16), String> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<(u32, u16), String>>();
    std::thread::spawn(move || {
        com_init();
        let mut cur = match Loopback::open() {
            Ok(l) => {
                let _ = tx.send(Ok((l.rate, l.channels)));
                Some(l)
            }
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        let mut last_check = Instant::now();
        let mut buf: Vec<f32> = Vec::with_capacity(48_000);
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(10));
            // 每秒看一次默认输出设备有没有换
            if last_check.elapsed() > Duration::from_secs(1) {
                last_check = Instant::now();
                let now = default_render().ok().map(|d| device_id(&d));
                let changed = match (&cur, &now) {
                    (Some(l), Some(id)) => &l.id != id,
                    (None, Some(_)) => true,
                    _ => false,
                };
                if changed {
                    cur = None;
                    match Loopback::open() {
                        Ok(l) => cur = Some(l),
                        Err(e) => tracing::debug!("换声音设备失败: {e}"),
                    }
                }
            }
            buf.clear();
            let (ok, rate, channels) = match cur.as_ref() {
                Some(l) => (l.drain(&mut buf).is_ok(), l.rate, l.channels),
                None => continue,
            };
            if !ok {
                // 设备失效（被拔掉等），下一轮按默认设备重新打开
                cur = None;
                last_check = Instant::now() - Duration::from_secs(2);
                continue;
            }
            if !buf.is_empty() {
                push(std::mem::take(&mut buf), rate, channels);
            }
        }
    });
    rx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| Err("打开声音设备超时".into()))
}
