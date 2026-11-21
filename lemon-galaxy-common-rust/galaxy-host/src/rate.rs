//! 画面码率自适应（被控端）：按 QUIC 连接的丢包、拥塞事件与数据报发送缓冲占用调整编码码率。
//!
//! 主控端要求的码率是上限。网络拥塞时立即降到七成（之后至少 3 秒内不再降，等效果出来）；
//! 连续 15 秒平稳再升 25%，直到上限。有的编码器改码率要重建（Windows openh264 会出关键帧），所以调整不宜频繁。

use std::time::{Duration, Instant};

pub const MIN_KBPS: u32 = 300;

/// 一秒的网络采样。
#[derive(Clone, Copy, Debug, Default)]
pub struct Sample {
    pub sent_packets: u64,
    pub lost_packets: u64,
    pub congestion_events: u64,
    /// 数据报发送缓冲剩余比例（0..1），低说明发得比网络快
    pub buffer_free: f32,
}

pub struct RateCtl {
    ceiling: u32,
    current: u32,
    last: Option<Sample>,
    last_down: Option<Instant>,
    stable_since: Instant,
}

impl RateCtl {
    pub fn new(ceiling_kbps: u32, now: Instant) -> Self {
        let c = ceiling_kbps.max(MIN_KBPS);
        Self { ceiling: c, current: c, last: None, last_down: None, stable_since: now }
    }

    pub fn current(&self) -> u32 {
        self.current
    }

    /// 主控端改了码率（用户主动选择）：直接用新值作为当前码率与上限，之后照常自适应。
    pub fn set_ceiling(&mut self, kbps: u32, now: Instant) -> u32 {
        self.ceiling = kbps.max(MIN_KBPS);
        self.current = self.ceiling;
        self.stable_since = now;
        self.current
    }

    /// 喂一次采样（约每秒一次，计数是累计值）。返回需要改成的新码率。
    pub fn on_sample(&mut self, s: Sample, now: Instant) -> Option<u32> {
        let Some(prev) = self.last.replace(s) else { return None };
        let sent = s.sent_packets.saturating_sub(prev.sent_packets);
        let lost = s.lost_packets.saturating_sub(prev.lost_packets);
        let events = s.congestion_events.saturating_sub(prev.congestion_events);
        let loss = if sent >= 30 { lost as f32 / sent as f32 } else { 0.0 };
        let congested = loss > 0.03 || (events > 0 && loss > 0.01) || s.buffer_free < 0.25;
        if congested {
            self.stable_since = now;
            let recently = self.last_down.map(|t| now.duration_since(t) < Duration::from_secs(3)).unwrap_or(false);
            if recently || self.current <= MIN_KBPS {
                return None;
            }
            let next = ((self.current as f32 * 0.7) as u32).max(MIN_KBPS);
            self.current = next;
            self.last_down = Some(now);
            return Some(next);
        }
        if self.current < self.ceiling && now.duration_since(self.stable_since) >= Duration::from_secs(15) {
            let next = ((self.current as f32 * 1.25) as u32).min(self.ceiling);
            self.current = next;
            self.stable_since = now;
            return Some(next);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(sent: u64, lost: u64, free: f32) -> Sample {
        Sample { sent_packets: sent, lost_packets: lost, congestion_events: 0, buffer_free: free }
    }

    #[test]
    fn backs_off_on_loss_then_recovers_slowly() {
        let t0 = Instant::now();
        let mut r = RateCtl::new(8000, t0);
        assert_eq!(r.on_sample(s(0, 0, 1.0), t0), None);
        // 10% 丢包：降到 5600
        assert_eq!(r.on_sample(s(1000, 100, 1.0), t0 + Duration::from_secs(1)), Some(5600));
        // 3 秒内再丢包不再降
        assert_eq!(r.on_sample(s(2000, 200, 1.0), t0 + Duration::from_secs(2)), None);
        // 之后再拥塞（发送缓冲快满）继续降
        assert_eq!(r.on_sample(s(3000, 200, 0.1), t0 + Duration::from_secs(5)), Some(3920));
        // 平稳不到 15 秒不升
        assert_eq!(r.on_sample(s(4000, 200, 1.0), t0 + Duration::from_secs(10)), None);
        // 平稳 15 秒后升 25%
        assert_eq!(r.on_sample(s(5000, 200, 1.0), t0 + Duration::from_secs(20)), Some(4900));
        assert_eq!(r.on_sample(s(6000, 200, 1.0), t0 + Duration::from_secs(36)), Some(6125));
        assert_eq!(r.on_sample(s(7000, 200, 1.0), t0 + Duration::from_secs(52)), Some(7656));
        // 不超过上限
        assert_eq!(r.on_sample(s(8000, 200, 1.0), t0 + Duration::from_secs(68)), Some(8000));
        assert_eq!(r.on_sample(s(9000, 200, 1.0), t0 + Duration::from_secs(90)), None);
    }

    #[test]
    fn small_samples_and_floor() {
        let t0 = Instant::now();
        let mut r = RateCtl::new(400, t0);
        r.on_sample(s(0, 0, 1.0), t0);
        // 包太少不算丢包率
        assert_eq!(r.on_sample(s(10, 5, 1.0), t0 + Duration::from_secs(1)), None);
        assert_eq!(r.on_sample(s(200, 100, 1.0), t0 + Duration::from_secs(2)), Some(MIN_KBPS));
        assert_eq!(r.on_sample(s(400, 200, 1.0), t0 + Duration::from_secs(6)), None);
    }

    #[test]
    fn ceiling_changes() {
        let t0 = Instant::now();
        let mut r = RateCtl::new(8000, t0);
        assert_eq!(r.set_ceiling(4000, t0), 4000);
        assert_eq!(r.set_ceiling(10000, t0), 10000);
        assert_eq!(r.set_ceiling(10, t0), MIN_KBPS);
    }
}
