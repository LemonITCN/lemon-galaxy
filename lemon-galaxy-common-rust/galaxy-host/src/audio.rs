//! 远端声音（被控端）：平台层采到的系统声音统一转成 48 kHz 立体声，Opus 编码一次，分发给所有在听的会话。
//!
//! 平台层只管采集（Windows WASAPI 环回、macOS ScreenCaptureKit），格式随系统；
//! 第一个会话开始听时才让平台层开始采集，最后一个停止时关掉。

use crate::platform::HostPlatform;
use bytes::Bytes;
use galaxy_proto::peer::AudioPacketHeader;
use parking_lot::Mutex;
use std::sync::{Arc, Weak};

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;
/// 每包 20 毫秒
pub const FRAME_SAMPLES: usize = 960;

/// 平台层采到声音后推给内核：交错排列的浮点样本（-1..1），采样率与声道数随系统。
pub trait AudioSink: Send + Sync {
    fn push_pcm(&self, pcm: &[f32], sample_rate: u32, channels: u16);
}

struct Sub {
    id: u64,
    conn: quinn::Connection,
    channel: u8,
}

/// 把任意采样率、声道数的样本流转成 48 kHz 立体声（线性插值，够用于监听）。
#[derive(Default)]
pub struct Resampler {
    rate: u32,
    /// 下一个输出样本在输入流中的位置（相对 `prev` 的小数）
    pos: f64,
    prev: [f32; 2],
    primed: bool,
}

impl Resampler {
    pub fn feed(&mut self, pcm: &[f32], rate: u32, channels: u16, out: &mut Vec<f32>) {
        let ch = channels.max(1) as usize;
        if rate == 0 || pcm.len() < ch {
            return;
        }
        if rate != self.rate {
            *self = Self { rate, ..Default::default() };
        }
        let frame = |i: usize| -> [f32; 2] {
            let s = &pcm[i * ch..i * ch + ch];
            if ch == 1 {
                [s[0], s[0]]
            } else {
                [s[0], s[1]]
            }
        };
        let n = pcm.len() / ch;
        if rate == SAMPLE_RATE {
            for i in 0..n {
                out.extend_from_slice(&frame(i));
            }
            return;
        }
        let step = rate as f64 / SAMPLE_RATE as f64;
        if !self.primed {
            self.prev = frame(0);
            self.primed = true;
        }
        // 输入序列视为 prev, f(0), f(1) ... ；pos 是相对 prev 的位置
        loop {
            let i = self.pos.floor() as usize;
            if i >= n {
                break;
            }
            let a = if i == 0 { self.prev } else { frame(i - 1) };
            let b = frame(i);
            let t = (self.pos - i as f64) as f32;
            out.push(a[0] + (b[0] - a[0]) * t);
            out.push(a[1] + (b[1] - a[1]) * t);
            self.pos += step;
        }
        self.pos -= n as f64;
        self.prev = frame(n - 1);
    }
}

struct HubState {
    subs: Vec<Sub>,
    next_id: u64,
    platform: Option<Arc<dyn HostPlatform>>,
    enc: Option<opus::Encoder>,
    bitrate_kbps: u32,
    resampler: Resampler,
    pending: Vec<f32>,
    seq: u32,
    out: Vec<u8>,
}

/// 被控端的声音分发中心。
pub struct AudioHub {
    state: Mutex<HubState>,
    me: Mutex<Weak<AudioHub>>,
}

struct HubSink(Weak<AudioHub>);

impl AudioSink for HubSink {
    fn push_pcm(&self, pcm: &[f32], sample_rate: u32, channels: u16) {
        if let Some(h) = self.0.upgrade() {
            h.feed(pcm, sample_rate, channels);
        }
    }
}

impl AudioHub {
    pub fn new() -> Arc<Self> {
        let me = Arc::new(Self {
            state: Mutex::new(HubState { subs: vec![], next_id: 1, platform: None, enc: None, bitrate_kbps: 96, resampler: Resampler::default(), pending: vec![], seq: 0, out: vec![0; 4000] }),
            me: Mutex::new(Weak::new()),
        });
        *me.me.lock() = Arc::downgrade(&me);
        me
    }

    /// 开始给 `conn` 的 `channel` 发声音。第一个订阅者到来时让平台层开始采集。
    pub fn subscribe(&self, platform: &Arc<dyn HostPlatform>, conn: quinn::Connection, channel: u8, bitrate_kbps: u32) -> Result<u64, String> {
        let mut st = self.state.lock();
        if st.platform.is_none() {
            let sink: Arc<dyn AudioSink> = Arc::new(HubSink(self.me.lock().clone()));
            let enc = new_encoder(bitrate_kbps)?;
            // 平台层可能同步回调 push_pcm，先放锁
            drop(st);
            platform.start_audio(sink)?;
            st = self.state.lock();
            st.platform = Some(platform.clone());
            st.enc = Some(enc);
            st.bitrate_kbps = bitrate_kbps;
            st.pending.clear();
            st.resampler = Resampler::default();
        }
        let id = st.next_id;
        st.next_id += 1;
        st.subs.push(Sub { id, conn, channel });
        Ok(id)
    }

    pub fn unsubscribe(&self, id: u64) {
        let stop = {
            let mut st = self.state.lock();
            st.subs.retain(|s| s.id != id);
            if st.subs.is_empty() {
                st.enc = None;
                st.platform.take()
            } else {
                None
            }
        };
        if let Some(p) = stop {
            p.stop_audio();
        }
    }

    pub fn listeners(&self) -> usize {
        self.state.lock().subs.len()
    }

    fn feed(&self, pcm: &[f32], sample_rate: u32, channels: u16) {
        let mut guard = self.state.lock();
        let st = &mut *guard;
        if st.subs.is_empty() || st.enc.is_none() {
            return;
        }
        let mut stereo = std::mem::take(&mut st.pending);
        st.resampler.feed(pcm, sample_rate, channels, &mut stereo);
        let frame_len = FRAME_SAMPLES * CHANNELS;
        let mut off = 0;
        while stereo.len() - off >= frame_len {
            let frame = &stereo[off..off + frame_len];
            off += frame_len;
            let Some(enc) = st.enc.as_mut() else { break };
            let n = match enc.encode_float(frame, &mut st.out) {
                Ok(n) => n,
                Err(e) => {
                    tracing::debug!("Opus 编码失败: {e}");
                    continue;
                }
            };
            let seq = st.seq;
            st.seq = st.seq.wrapping_add(1);
            for s in &st.subs {
                let mut b = Vec::with_capacity(n + AudioPacketHeader::LEN);
                AudioPacketHeader { channel: s.channel, seq }.encode(&mut b);
                b.extend_from_slice(&st.out[..n]);
                let _ = s.conn.send_datagram(Bytes::from(b));
            }
        }
        stereo.drain(..off);
        // 积压太多（平台层一次推了很长一段）只留最近 200 毫秒
        let cap = SAMPLE_RATE as usize / 5 * CHANNELS;
        if stereo.len() > cap {
            let extra = stereo.len() - cap;
            stereo.drain(..extra);
        }
        st.pending = stereo;
    }
}

fn new_encoder(bitrate_kbps: u32) -> Result<opus::Encoder, String> {
    let mut e = opus::Encoder::new(SAMPLE_RATE, opus::Channels::Stereo, opus::Application::Audio).map_err(|e| format!("Opus 编码器创建失败：{e}"))?;
    let _ = e.set_bitrate(opus::Bitrate::Bits((bitrate_kbps.clamp(16, 256) * 1000) as i32));
    Ok(e)
}

/// 测试用的声源：440 Hz 正弦波，44.1 kHz 单声道（顺带覆盖重采样与声道转换）。端到端测试里给 Linux 被控端用。
pub struct ToneSource {
    stop: Mutex<Option<std::sync::mpsc::Sender<()>>>,
}

impl Default for ToneSource {
    fn default() -> Self {
        Self { stop: Mutex::new(None) }
    }
}

impl ToneSource {
    pub fn start(&self, sink: Arc<dyn AudioSink>) {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        *self.stop.lock() = Some(tx);
        std::thread::spawn(move || {
            let rate = 44_100u32;
            let chunk = (rate / 100) as usize; // 10 毫秒
            let mut phase = 0f64;
            let step = 2.0 * std::f64::consts::PI * 440.0 / rate as f64;
            let start = std::time::Instant::now();
            let mut sent = 0u64;
            loop {
                match rx.try_recv() {
                    Ok(()) | Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                }
                let mut buf = Vec::with_capacity(chunk);
                for _ in 0..chunk {
                    buf.push((phase.sin() * 0.5) as f32);
                    phase += step;
                }
                sink.push_pcm(&buf, rate, 1);
                sent += 1;
                // 按真实时间节奏推
                let due = std::time::Duration::from_millis(sent * 10);
                if let Some(wait) = due.checked_sub(start.elapsed()) {
                    std::thread::sleep(wait);
                }
            }
        });
    }

    pub fn stop(&self) {
        self.stop.lock().take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_44k_mono_to_48k_stereo() {
        let mut r = Resampler::default();
        let mut out = vec![];
        // 1 秒 44.1k 单声道，分 100 次喂
        for k in 0..100 {
            let chunk: Vec<f32> = (0..441).map(|i| (k * 441 + i) as f32).collect();
            r.feed(&chunk, 44_100, 1, &mut out);
        }
        let frames = out.len() / 2;
        assert!((47_990..=48_010).contains(&frames), "{frames}");
        // 左右相同、单调递增（线性插值保持顺序）
        for f in out.chunks(2) {
            assert_eq!(f[0], f[1]);
        }
        assert!(out.chunks(2).map(|f| f[0]).collect::<Vec<_>>().windows(2).all(|w| w[1] >= w[0]));
    }

    #[test]
    fn passthrough_48k_and_multichannel() {
        let mut r = Resampler::default();
        let mut out = vec![];
        r.feed(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 48_000, 3, &mut out);
        assert_eq!(out, vec![1.0, 2.0, 4.0, 5.0]);
    }

    #[test]
    fn opus_roundtrip_keeps_tone() {
        let mut enc = new_encoder(96).unwrap();
        let mut dec = opus::Decoder::new(SAMPLE_RATE, opus::Channels::Stereo).unwrap();
        let mut phase = 0f64;
        let step = 2.0 * std::f64::consts::PI * 440.0 / SAMPLE_RATE as f64;
        let mut energy = 0f64;
        let mut n = 0usize;
        for k in 0..50 {
            let mut frame = Vec::with_capacity(FRAME_SAMPLES * 2);
            for _ in 0..FRAME_SAMPLES {
                let s = (phase.sin() * 0.5) as f32;
                frame.push(s);
                frame.push(s);
                phase += step;
            }
            let mut pkt = vec![0u8; 4000];
            let len = enc.encode_float(&frame, &mut pkt).unwrap();
            let mut out = vec![0f32; FRAME_SAMPLES * 2];
            let got = dec.decode_float(&pkt[..len], &mut out, false).unwrap();
            assert_eq!(got, FRAME_SAMPLES);
            if k > 5 {
                energy += out.iter().map(|x| (*x as f64).powi(2)).sum::<f64>();
                n += out.len();
            }
        }
        let rms = (energy / n as f64).sqrt();
        // 0.5 幅度正弦波 RMS ≈ 0.354
        assert!((0.3..0.4).contains(&rms), "rms {rms}");
    }
}
