//! 画面帧的发送：切成数据报，按节奏发出（Linux、Windows、macOS 被控端共用）。
//!
//! 以前平台层每编好一帧就在它的线程里把所有分片一口气交给 QUIC。局域网往返只有几毫秒，QUIC 的拥塞窗口很快涨得很大，
//! 一帧几百 KB 的关键帧或大面积变化就按有线网卡的线速（千兆）冲出去；主控在 Wi-Fi 上时路由器 / 无线这一段接不住，
//! 成片丢包（Ubuntu 有线 → Mac Wi-Fi 实测：帧大到一两百 KB 后丢包 6%～10%，两端网卡与套接字都没丢，丢在中间），
//! 丢一片整帧作废、主控要关键帧，关键帧更大更容易丢，越丢越糊。
//!
//! 现在每路画面一个发送任务：平台层只把帧放进队列；任务按令牌桶匀速发（速率见 [`pace_bps`]，远高于画面码率，
//! 平时不添延迟，只削掉线速突发）。发不过来积压时，新的关键帧到了就把排在它前面还没发的帧丢掉（关键帧不依赖它们），
//! 帧号只按真正发出的帧递增（主控端按帧号跳号判丢帧）。
//!
//! 丢片重传（能力 video_nack）：Wi-Fi 上即使不拥塞也会成片丢包（同一台 Mac 用纯 UDP 实测 10 Mbps 时丢 5%～20%，
//! 隔空投送等功能周期性切走信道），一帧几十上百片，丢一片整帧就废。发过的分片留 [`HISTORY_AGE`]，
//! 主控端发 `Nack` 时优先补发（同样匀速），每片最多补 [`MAX_RESEND`] 次。

use bytes::Bytes;
use galaxy_proto::peer::{FragNack, VideoFragHeader};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// 匀速发送的最低速率（bit/s）
const PACE_MIN_BPS: u64 = 100_000_000;
/// 匀速发送速率至少是画面码率的几倍（码率很高时跟着提高）
const PACE_MULT: u64 = 6;
/// 令牌桶容量（字节）：允许连着发出的最大量
const BURST_BYTES: f64 = 32.0 * 1024.0;
/// 发过的分片留多久、最多留多少字节（供补发）
const HISTORY_AGE: Duration = Duration::from_secs(1);
const HISTORY_BYTES: usize = 16 << 20;
/// 同一片最多补发几次
const MAX_RESEND: u8 = 4;

/// 画面码率为 `kbps` 时的发送速率（bit/s）。环境变量 LEMON_GALAXY_VIDEO_PACE_MBPS 可指定（0 为不限速，测试用）。
pub fn pace_bps(kbps: u32) -> Option<u64> {
    if let Some(v) = std::env::var("LEMON_GALAXY_VIDEO_PACE_MBPS").ok().and_then(|s| s.trim().parse::<u64>().ok()) {
        return (v > 0).then_some(v * 1_000_000);
    }
    Some((kbps as u64 * 1000 * PACE_MULT).max(PACE_MIN_BPS))
}

pub struct Frame {
    pub data: Vec<u8>,
    pub pts_us: u64,
    pub keyframe: bool,
}

/// 平台层这边：把帧放进队列；主控端要补发的分片也从这里交给发送任务。
pub struct Queue {
    tx: mpsc::UnboundedSender<Frame>,
    nack_tx: mpsc::UnboundedSender<Vec<FragNack>>,
    /// 队列里还没发出的字节（码率自适应把它算进积压）
    pub queued: Arc<AtomicU64>,
}

impl Queue {
    pub fn push(&self, f: Frame) {
        self.queued.fetch_add(f.data.len() as u64, Ordering::Relaxed);
        if self.tx.send(f).is_err() {
            // 发送任务已结束（画面在关）
        }
    }

    pub fn nack(&self, v: Vec<FragNack>) {
        let _ = self.nack_tx.send(v);
    }
}

/// 开一路画面的发送任务。`kbps` 是这一路现在的画面码率（码率自适应随时更新）。任务在队列关闭（`Queue` 被丢弃）后结束。
pub fn start(conn: quinn::Connection, channel: u8, kbps: Arc<AtomicU32>) -> Queue {
    let (tx, rx) = mpsc::unbounded_channel();
    let (nack_tx, nack_rx) = mpsc::unbounded_channel();
    let queued = Arc::new(AtomicU64::new(0));
    tokio::spawn(run(conn, channel, kbps, rx, nack_rx, queued.clone()));
    Queue { tx, nack_tx, queued }
}

struct Sent {
    id: u32,
    at: Instant,
    frags: Vec<Bytes>,
    resent: Vec<u8>,
}

/// 最近发过的帧（按帧号顺序）
#[derive(Default)]
struct History {
    frames: VecDeque<Sent>,
    bytes: usize,
}

impl History {
    fn push(&mut self, id: u32, frags: Vec<Bytes>, now: Instant) {
        self.bytes += frags.iter().map(|b| b.len()).sum::<usize>();
        let n = frags.len();
        self.frames.push_back(Sent { id, at: now, frags, resent: vec![0; n] });
        while let Some(f) = self.frames.front() {
            if now.duration_since(f.at) <= HISTORY_AGE && self.bytes <= HISTORY_BYTES {
                break;
            }
            self.bytes -= f.frags.iter().map(|b| b.len()).sum::<usize>();
            self.frames.pop_front();
        }
    }

    /// 要补发的数据报（已扣掉补发次数用完的）
    fn take(&mut self, nacks: &[FragNack]) -> Vec<Bytes> {
        let mut out = Vec::new();
        for n in nacks {
            let Some(f) = self.frames.iter_mut().find(|f| f.id == n.frame_id) else { continue };
            let all: Vec<u16> = if n.frags.is_empty() { (0..f.frags.len() as u16).collect() } else { n.frags.clone() };
            for i in all {
                let i = i as usize;
                if i < f.frags.len() && f.resent[i] < MAX_RESEND {
                    f.resent[i] += 1;
                    out.push(f.frags[i].clone());
                }
            }
        }
        out
    }
}

/// 积压的帧里有关键帧时，它前面的帧都不用发了。返回丢掉的字节数。
fn skip_before_last_keyframe(pending: &mut Vec<Frame>) -> u64 {
    match pending.iter().rposition(|f| f.keyframe) {
        Some(k) if k > 0 => pending.drain(..k).map(|f| f.data.len() as u64).sum(),
        _ => 0,
    }
}

struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Bucket {
    /// 发 `n` 字节前要等多久（按 `bps` 补充令牌）；返回 0 表示可以立即发（已扣掉）。
    fn take(&mut self, n: usize, bps: Option<u64>, now: Instant) -> Duration {
        let Some(bps) = bps else { return Duration::ZERO };
        let rate = bps as f64 / 8.0;
        self.tokens = (self.tokens + now.duration_since(self.at).as_secs_f64() * rate).min(BURST_BYTES);
        self.at = now;
        if self.tokens >= n as f64 {
            self.tokens -= n as f64;
            return Duration::ZERO;
        }
        Duration::from_secs_f64((n as f64 - self.tokens) / rate)
    }
}

/// 匀速发一个数据报（按令牌桶等）。返回 false 表示连接已断。
async fn paced_send(conn: &quinn::Connection, bucket: &mut Bucket, pace: Option<u64>, d: Bytes) -> bool {
    let wait = bucket.take(d.len(), pace, Instant::now());
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
        if !bucket.take(d.len(), pace, Instant::now()).is_zero() {
            // 计时器比要求的早醒（粒度粗的系统）：先欠着，下一片补回来
            bucket.tokens -= d.len() as f64;
        }
    }
    conn.send_datagram(d).is_ok()
}

/// 补发主控端要的分片。返回 false 表示连接已断。
async fn resend(conn: &quinn::Connection, bucket: &mut Bucket, pace: Option<u64>, hist: &mut History, nacks: Vec<FragNack>) -> bool {
    let list = hist.take(&nacks);
    if !list.is_empty() {
        tracing::trace!("补发 {} 片", list.len());
    }
    for d in list {
        if !paced_send(conn, bucket, pace, d).await {
            return false;
        }
    }
    true
}

async fn run(conn: quinn::Connection, channel: u8, kbps: Arc<AtomicU32>, mut rx: mpsc::UnboundedReceiver<Frame>, mut nack_rx: mpsc::UnboundedReceiver<Vec<FragNack>>, queued: Arc<AtomicU64>) {
    let mut frame_id: u32 = 0;
    let mut bucket = Bucket { tokens: BURST_BYTES, at: Instant::now() };
    let mut pending: Vec<Frame> = Vec::new();
    let mut hist = History::default();
    loop {
        let pace = pace_bps(kbps.load(Ordering::Relaxed));
        while let Ok(v) = nack_rx.try_recv() {
            if !resend(&conn, &mut bucket, pace, &mut hist, v).await {
                return;
            }
        }
        if pending.is_empty() {
            tokio::select! {
                f = rx.recv() => match f {
                    Some(f) => pending.push(f),
                    None => return,
                },
                v = nack_rx.recv() => match v {
                    Some(v) => {
                        if !resend(&conn, &mut bucket, pace, &mut hist, v).await {
                            return;
                        }
                        continue;
                    }
                    None => return,
                },
            }
        }
        while let Ok(f) = rx.try_recv() {
            pending.push(f);
        }
        let skipped = skip_before_last_keyframe(&mut pending);
        if skipped > 0 {
            queued.fetch_sub(skipped, Ordering::Relaxed);
            tracing::debug!("画面发送积压，跳过关键帧之前的 {skipped} 字节");
        }
        let f = pending.remove(0);
        let len = f.data.len() as u64;
        let max = conn.max_datagram_size().unwrap_or(1100).saturating_sub(VideoFragHeader::BASE_LEN + 8).max(256);
        let count = f.data.len().div_ceil(max).max(1);
        if count <= u16::MAX as usize {
            let id = frame_id;
            frame_id = frame_id.wrapping_add(1);
            let frags: Vec<Bytes> = f
                .data
                .chunks(max)
                .enumerate()
                .map(|(i, chunk)| {
                    let mut b = Vec::with_capacity(chunk.len() + 20);
                    VideoFragHeader { channel, frame_id: id, frag_idx: i as u16, frag_count: count as u16, keyframe: f.keyframe, pts_us: f.pts_us }.encode(&mut b);
                    b.extend_from_slice(chunk);
                    Bytes::from(b)
                })
                .collect();
            hist.push(id, frags.clone(), Instant::now());
            for d in frags {
                // 补发优先：主控端在等它们才能往下解码
                while let Ok(v) = nack_rx.try_recv() {
                    if !resend(&conn, &mut bucket, pace, &mut hist, v).await {
                        return;
                    }
                }
                if !paced_send(&conn, &mut bucket, pace, d).await {
                    return;
                }
            }
        }
        queued.fetch_sub(len, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fr(n: usize, key: bool) -> Frame {
        Frame { data: vec![0; n], pts_us: 0, keyframe: key }
    }

    #[test]
    fn keyframe_supersedes_older_pending() {
        let mut v = vec![fr(10, false), fr(20, false), fr(30, true), fr(5, false)];
        assert_eq!(skip_before_last_keyframe(&mut v), 30);
        assert_eq!(v.len(), 2);
        assert!(v[0].keyframe);
        // 关键帧已在最前面或没有关键帧：不动
        assert_eq!(skip_before_last_keyframe(&mut v), 0);
        let mut w = vec![fr(10, false), fr(20, false)];
        assert_eq!(skip_before_last_keyframe(&mut w), 0);
        assert_eq!(w.len(), 2);
        // 多个关键帧：留最后一个
        let mut x = vec![fr(1, true), fr(2, false), fr(3, true)];
        assert_eq!(skip_before_last_keyframe(&mut x), 3);
        assert_eq!(x.len(), 1);
    }

    #[test]
    fn bucket_paces_to_rate() {
        let t0 = Instant::now();
        let mut b = Bucket { tokens: BURST_BYTES, at: t0 };
        // 100 Mbit/s = 12.5 MB/s；先用掉桶里的 32 KB
        let mut sent = 0usize;
        while b.take(1200, Some(100_000_000), t0).is_zero() {
            sent += 1200;
        }
        assert!(sent >= 32 * 1024 - 1200 && sent <= 32 * 1024, "{sent}");
        // 再发 1200 字节要等约 1200 / 12.5 MB/s ≈ 0.1 毫秒以内
        let w = b.take(1200, Some(100_000_000), t0);
        assert!(w > Duration::ZERO && w < Duration::from_micros(100), "{w:?}");
        // 1 毫秒后补了约 12.5 KB
        let t1 = t0 + Duration::from_millis(1);
        let mut more = 0;
        while b.take(1200, Some(100_000_000), t1).is_zero() {
            more += 1200;
        }
        assert!((10_000..=13_000).contains(&more), "{more}");
        // 不限速
        assert_eq!(b.take(1_000_000, None, t1), Duration::ZERO);
    }

    #[test]
    fn history_resends_and_limits() {
        let t0 = Instant::now();
        let mut h = History::default();
        let fr = |n: usize| (0..n).map(|i| Bytes::from(vec![i as u8; 100])).collect::<Vec<_>>();
        h.push(7, fr(3), t0);
        h.push(8, fr(2), t0);
        // 指定分片；越界的忽略；不认识的帧忽略
        assert_eq!(h.take(&[FragNack { frame_id: 7, frags: vec![1, 5] }, FragNack { frame_id: 99, frags: vec![0] }]).len(), 1);
        // 整帧
        assert_eq!(h.take(&[FragNack { frame_id: 8, frags: vec![] }]).len(), 2);
        // 每片最多补 MAX_RESEND 次
        for _ in 1..MAX_RESEND {
            assert_eq!(h.take(&[FragNack { frame_id: 7, frags: vec![1] }]).len(), 1);
        }
        assert_eq!(h.take(&[FragNack { frame_id: 7, frags: vec![1] }]).len(), 0);
        // 过期的帧清掉
        h.push(9, fr(1), t0 + HISTORY_AGE + Duration::from_millis(1));
        assert_eq!(h.frames.len(), 1);
        assert_eq!(h.bytes, 100);
    }

    #[test]
    fn pace_follows_bitrate() {
        assert_eq!(pace_bps(4_000), Some(PACE_MIN_BPS));
        assert_eq!(pace_bps(50_000), Some(300_000_000));
    }
}
