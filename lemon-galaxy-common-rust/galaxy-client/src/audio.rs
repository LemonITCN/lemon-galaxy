//! 远端声音（主控端）：收 Opus 数据报、解码（丢包时做补偿）、交给输出（本机声卡播放或测试统计）。

use crate::error::{ClientError, Result};
use crate::session::PeerSession;
use bytes::Bytes;
use galaxy_net::frame;
use galaxy_proto::peer::*;
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::sync::mpsc;

pub const SAMPLE_RATE: u32 = 48_000;
/// 解码后的输出：48 kHz 立体声交错浮点样本，每段 20 毫秒。
pub trait PcmOutput: Send + Sync {
    fn write(&self, pcm: &[f32]);
}

/// 正在收听的一路声音。
pub struct AudioChannel {
    send: tokio::sync::Mutex<quinn::SendStream>,
    pub channel: u8,
    session: Weak<PeerSession>,
    task: parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl AudioChannel {
    pub async fn stop(&self) {
        if let Some(t) = self.task.lock().take() {
            t.abort();
        }
        if let Some(s) = self.session.upgrade() {
            s.unregister_datagrams(self.channel);
        }
        let mut s = self.send.lock().await;
        let _ = frame::write(&mut *s, &AudioIn::Stop).await;
        let _ = s.finish();
    }
}

impl PeerSession {
    /// 开始收听被控端的系统声音。被控端能力列表里要有 "audio"（有授权且平台支持）。
    pub async fn open_audio(self: &Arc<Self>, bitrate_kbps: u32, out: Arc<dyn PcmOutput>) -> Result<Arc<AudioChannel>> {
        if !self.ack.capabilities.iter().any(|c| c == "audio") {
            return Err(ClientError::Other(if self.ack.permissions.audio { "对方设备不支持传声音".into() } else { "没有听声音的权限".into() }));
        }
        let (send, mut recv) = self.open_stream(StreamOpen::Audio(AudioRequest { codecs: vec![AudioCodec::Opus], bitrate_kbps })).await?;
        let first: AudioOut = tokio::time::timeout(Duration::from_secs(15), frame::read(&mut recv))
            .await
            .map_err(|_| ClientError::Network("打开声音超时".into()))?
            .map_err(|e| ClientError::Network(e.to_string()))?
            .ok_or_else(|| ClientError::Network("对方关闭了声音（可能版本太老）".into()))?;
        let (sample_rate, channels, channel) = match first {
            AudioOut::Started { codec: AudioCodec::Opus, sample_rate, channels, channel } => (sample_rate, channels, channel),
            AudioOut::Error(e) => return Err(crate::session::peer_err(e)),
        };
        if sample_rate != SAMPLE_RATE || channels != 2 {
            return Err(ClientError::Protocol(format!("不支持的声音格式 {sample_rate} Hz × {channels}")));
        }
        let (tx, mut rx) = mpsc::channel::<Bytes>(256);
        self.register_datagrams(channel, tx);
        let task = tokio::spawn(async move {
            let Ok(mut dec) = opus::Decoder::new(SAMPLE_RATE, opus::Channels::Stereo) else { return };
            let mut expected: Option<u32> = None;
            // 最长一包 120 毫秒
            let mut buf = vec![0f32; 5760 * 2];
            // 流关闭时 recv 返回 None，任务结束
            let mut recv = recv;
            let closed = async move { while let Ok(Some(_)) = frame::read::<AudioOut>(&mut recv).await {} };
            tokio::pin!(closed);
            loop {
                let d = tokio::select! {
                    d = rx.recv() => match d { Some(d) => d, None => break },
                    _ = &mut closed => break,
                };
                let Some((h, pos)) = AudioPacketHeader::decode(&d) else { continue };
                if let Some(e) = expected {
                    let gap = h.seq.wrapping_sub(e);
                    if gap > u32::MAX / 2 {
                        // 迟到的旧包
                        continue;
                    }
                    // 丢了几包：用解码器的丢包补偿填上（最多 60 毫秒，再多就直接跳过）
                    for _ in 0..gap.min(3) {
                        if let Ok(n) = dec.decode_float(&[], &mut buf[..960 * 2], false) {
                            out.write(&buf[..n * 2]);
                        }
                    }
                }
                expected = Some(h.seq.wrapping_add(1));
                if let Ok(n) = dec.decode_float(&d[pos..], &mut buf, false) {
                    out.write(&buf[..n * 2]);
                }
            }
        });
        Ok(Arc::new(AudioChannel { send: tokio::sync::Mutex::new(send), channel, session: Arc::downgrade(self), task: parking_lot::Mutex::new(Some(task)) }))
    }
}

/// 测试用输出：统计收到的样本数、能量与过零次数（估算主频）。
#[derive(Default)]
pub struct PcmStats {
    inner: parking_lot::Mutex<(u64, f64, u64, f32)>,
}

impl PcmStats {
    /// (样本帧数, 均方根, 估算频率 Hz)
    pub fn summary(&self) -> (u64, f64, f64) {
        let g = self.inner.lock();
        let frames = g.0;
        let rms = if frames > 0 { (g.1 / frames as f64).sqrt() } else { 0.0 };
        let secs = frames as f64 / SAMPLE_RATE as f64;
        let freq = if secs > 0.0 { g.2 as f64 / 2.0 / secs } else { 0.0 };
        (frames, rms, freq)
    }
}

impl PcmOutput for PcmStats {
    fn write(&self, pcm: &[f32]) {
        let mut g = self.inner.lock();
        for f in pcm.chunks_exact(2) {
            let l = f[0];
            g.0 += 1;
            g.1 += (l as f64) * (l as f64);
            if (g.3 < 0.0) != (l < 0.0) {
                g.2 += 1;
            }
            g.3 = l;
        }
    }
}
