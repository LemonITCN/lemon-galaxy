//! 用本机默认声音输出设备播放远端声音（macOS / iOS CoreAudio、Windows WASAPI、安卓 AAudio）。
//!
//! 解码出的 48 kHz 立体声先进环形缓冲，声卡回调按设备的采样率与声道数取用（线性插值重采样）。
//! 先攒 40 毫秒再出声；断流时输出静音并重新攒；积压超过 250 毫秒时丢掉旧的，延迟不累积。

use crate::audio::{PcmOutput, SAMPLE_RATE};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

const PREBUFFER: usize = SAMPLE_RATE as usize * 2 * 40 / 1000;
const MAX_BUFFER: usize = SAMPLE_RATE as usize * 2 * 250 / 1000;
const KEEP_AFTER_TRIM: usize = SAMPLE_RATE as usize * 2 * 80 / 1000;

struct Ring {
    buf: VecDeque<f32>,
    playing: bool,
}

pub struct DevicePlayer {
    ring: Arc<Mutex<Ring>>,
    gain: Arc<AtomicU32>,
    muted: Arc<AtomicBool>,
    stop: Mutex<Option<std::sync::mpsc::Sender<()>>>,
}

impl DevicePlayer {
    /// 打开默认输出设备开始播放（没有声音时输出静音）。
    pub fn start(volume: f32) -> Result<Arc<Self>, String> {
        let ring = Arc::new(Mutex::new(Ring { buf: VecDeque::with_capacity(MAX_BUFFER + 4096), playing: false }));
        let gain = Arc::new(AtomicU32::new(volume.clamp(0.0, 1.0).to_bits()));
        let muted = Arc::new(AtomicBool::new(false));
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let (r2, g2, m2) = (ring.clone(), gain.clone(), muted.clone());
        // 有的平台上声音流不能跨线程，放在自己的线程里建、持有到停止
        std::thread::Builder::new()
            .name("galaxy-audio-out".into())
            .spawn(move || {
                // 声音库在个别平台上可能 panic（如安卓没登记 Context），转成错误返回，不要等到超时
                let opened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| open_stream(r2, g2, m2)));
                let stream = match opened {
                    Ok(Ok(s)) => s,
                    Ok(Err(e)) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                    Err(_) => {
                        let _ = ready_tx.send(Err("打开声音输出设备时出错".into()));
                        return;
                    }
                };
                if let Err(e) = stream.play() {
                    let _ = ready_tx.send(Err(format!("无法开始播放：{e}")));
                    return;
                }
                let _ = ready_tx.send(Ok(()));
                // 等到停止（发送端丢弃也算）
                let _ = stop_rx.recv();
                drop(stream);
            })
            .map_err(|e| e.to_string())?;
        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Arc::new(Self { ring, gain, muted, stop: Mutex::new(Some(stop_tx)) })),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("打开声音输出设备超时".into()),
        }
    }

    pub fn set_volume(&self, v: f32) {
        self.gain.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn set_muted(&self, m: bool) {
        self.muted.store(m, Ordering::Relaxed);
    }

    pub fn stop(&self) {
        self.stop.lock().take();
    }
}

impl Drop for DevicePlayer {
    fn drop(&mut self) {
        self.stop();
    }
}

impl PcmOutput for DevicePlayer {
    fn write(&self, pcm: &[f32]) {
        let mut r = self.ring.lock();
        r.buf.extend(pcm.iter().copied());
        if r.buf.len() > MAX_BUFFER {
            let extra = r.buf.len() - KEEP_AFTER_TRIM;
            r.buf.drain(..extra & !1);
        }
    }
}

/// 从 48 kHz 立体声缓冲按设备采样率取样（线性插值）。
struct Reader {
    step: f64,
    frac: f64,
    prev: [f32; 2],
    cur: [f32; 2],
}

impl Reader {
    fn next(&mut self, r: &mut Ring) -> [f32; 2] {
        if !r.playing {
            if r.buf.len() >= PREBUFFER {
                r.playing = true;
            } else {
                return [0.0, 0.0];
            }
        }
        while self.frac >= 1.0 {
            self.prev = self.cur;
            match (r.buf.pop_front(), r.buf.pop_front()) {
                (Some(a), Some(b)) => self.cur = [a, b],
                _ => {
                    // 断流：静音，重新攒缓冲
                    r.playing = false;
                    self.frac = 1.0;
                    self.prev = [0.0, 0.0];
                    self.cur = [0.0, 0.0];
                    return [0.0, 0.0];
                }
            }
            self.frac -= 1.0;
        }
        let t = self.frac as f32;
        self.frac += self.step;
        [self.prev[0] + (self.cur[0] - self.prev[0]) * t, self.prev[1] + (self.cur[1] - self.prev[1]) * t]
    }
}

fn open_stream(ring: Arc<Mutex<Ring>>, gain: Arc<AtomicU32>, muted: Arc<AtomicBool>) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let dev = host.default_output_device().ok_or("没有可用的声音输出设备")?;
    let supported = dev.default_output_config().map_err(|e| format!("读取声音输出设备配置失败：{e}"))?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let rate = config.sample_rate;
    let channels = config.channels as usize;
    let err = |e| tracing::debug!("声音输出出错: {e}");
    macro_rules! build {
        ($t:ty) => {{
            let mut rd = Reader { step: SAMPLE_RATE as f64 / rate as f64, frac: 1.0, prev: [0.0; 2], cur: [0.0; 2] };
            dev.build_output_stream::<$t, _, _>(
                config.clone(),
                move |data: &mut [$t], _| {
                    let g = if muted.load(Ordering::Relaxed) { 0.0 } else { f32::from_bits(gain.load(Ordering::Relaxed)) };
                    let mut r = ring.lock();
                    for f in data.chunks_mut(channels.max(1)) {
                        let [l, rr] = rd.next(&mut r);
                        let (l, rr) = (l * g, rr * g);
                        if f.len() == 1 {
                            f[0] = <$t as cpal::FromSample<f32>>::from_sample_((l + rr) * 0.5);
                        } else {
                            for (i, s) in f.iter_mut().enumerate() {
                                let v = match i {
                                    0 => l,
                                    1 => rr,
                                    _ => 0.0,
                                };
                                *s = <$t as cpal::FromSample<f32>>::from_sample_(v);
                            }
                        }
                    }
                },
                err,
                None,
            )
        }};
    }
    let stream = match format {
        cpal::SampleFormat::F32 => build!(f32),
        cpal::SampleFormat::I16 => build!(i16),
        cpal::SampleFormat::U16 => build!(u16),
        cpal::SampleFormat::I32 => build!(i32),
        other => return Err(format!("不支持的声音输出格式 {other:?}")),
    };
    stream.map_err(|e| format!("无法打开声音输出设备：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_prebuffers_resamples_and_recovers() {
        let mut ring = Ring { buf: VecDeque::new(), playing: false };
        let mut rd = Reader { step: 48_000.0 / 44_100.0, frac: 1.0, prev: [0.0; 2], cur: [0.0; 2] };
        // 不够预缓冲：静音
        ring.buf.extend([0.5f32; 100]);
        assert_eq!(rd.next(&mut ring), [0.0, 0.0]);
        assert!(!ring.playing);
        // 攒够 1 秒的常量 0.5
        ring.buf.extend(std::iter::repeat_n(0.5f32, 48_000 * 2));
        let mut n = 0;
        let mut first = None;
        while ring.buf.len() >= 2 || ring.playing {
            let s = rd.next(&mut ring);
            if !ring.playing {
                break;
            }
            if first.is_none() && s[0] > 0.49 {
                first = Some(n);
            }
            n += 1;
        }
        // 44.1k 设备取 1 秒 48k 数据，大约 44100 个样本帧
        assert!((44_000..=44_200).contains(&n), "{n}");
        assert!(first.unwrap() <= 2);
        // 断流后又是静音，需要重新攒
        assert_eq!(rd.next(&mut ring), [0.0, 0.0]);
    }
}
