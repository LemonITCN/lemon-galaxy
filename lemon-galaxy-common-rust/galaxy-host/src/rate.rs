//! 画面码率自适应（被控端）：按数据报发送队列的积压与 QUIC 连接的丢包调整编码码率。
//!
//! 主控端要求的码率是上限。QUIC 自己的拥塞控制决定实际能发多快：发得比网络快时数据报在发送队列里越积越多，
//! 这是最可靠的「超出网络能力」信号，按积压够发多久判断（连续两次采样都超过 [`QUEUE_LIMIT`]）。
//! 丢包只作辅助：Wi-Fi 上不拥塞也会丢包（Mac 用纯 UDP 实测 5%～20%，降码率也不会少丢），丢的分片有重传补回来
//! （vsend.rs）。以前按单秒 3% 判拥塞，加上 15 秒才升 25%、最低 300 kbps，码率一路被压到底，几分钟都回不来，
//! 画面满屏马赛克。现在按两秒合计、至少丢 [`LOSS_MIN_PACKETS`] 个且超过 [`LOSS_RATE`] 才算。
//! 不会补要分片的老主控端（没发过 `Nack`）丢一片整帧就废，丢包多时只能靠降码率把帧变小，门槛照旧敏感些
//! （[`LEGACY_LOSS_RATE`]、[`LEGACY_LOSS_MIN_PACKETS`]）。
//!
//! 拥塞时降到七成（之后至少 [`DOWN_GAP`] 内不再降，等效果出来）；连续 [`UP_AFTER`] 平稳升 30%，直到上限。

use std::time::{Duration, Instant};

pub const MIN_KBPS: u32 = 300;
/// 发送队列积压超过按当前码率能发这么久的量，算发不出去
const QUEUE_LIMIT: Duration = Duration::from_millis(300);
/// 两秒合计的丢包率门槛与最少丢包数
const LOSS_RATE: f32 = 0.25;
const LOSS_MIN_PACKETS: u64 = 20;
const LEGACY_LOSS_RATE: f32 = 0.08;
const LEGACY_LOSS_MIN_PACKETS: u64 = 10;
/// 降码率后多久内不再降
const DOWN_GAP: Duration = Duration::from_secs(2);
/// 平稳多久升一档
const UP_AFTER: Duration = Duration::from_secs(4);

/// 一秒的网络采样。
#[derive(Clone, Copy, Debug, Default)]
pub struct Sample {
    pub sent_packets: u64,
    pub lost_packets: u64,
    /// 数据报发送队列里还没发出去的字节数
    pub queued_bytes: u64,
}

pub struct RateCtl {
    ceiling: u32,
    current: u32,
    last: Option<Sample>,
    /// 上一秒的 (发送, 丢失, 积压是否超限)
    prev: (u64, u64, bool),
    last_down: Option<Instant>,
    stable_since: Instant,
    /// 主控端会补要丢了的分片
    retransmits: bool,
}

impl RateCtl {
    pub fn new(ceiling_kbps: u32, now: Instant) -> Self {
        let c = ceiling_kbps.max(MIN_KBPS);
        Self { ceiling: c, current: c, last: None, prev: (0, 0, false), last_down: None, stable_since: now, retransmits: false }
    }

    /// 主控端会补要丢了的分片（收到过它的 `Nack`）：零星丢包不再降码率。
    pub fn set_retransmits(&mut self, on: bool) {
        self.retransmits = on;
    }

    pub fn current(&self) -> u32 {
        self.current
    }

    /// 主控端改了码率（用户主动选择）：直接用新值作为当前码率与上限，之后照常自适应。
    pub fn set_ceiling(&mut self, kbps: u32, now: Instant) -> u32 {
        self.ceiling = kbps.max(MIN_KBPS);
        self.current = self.ceiling;
        self.stable_since = now;
        self.last_down = None;
        self.current
    }

    /// 喂一次采样（约每秒一次，发送与丢失是累计值）。返回需要改成的新码率。
    pub fn on_sample(&mut self, s: Sample, now: Instant) -> Option<u32> {
        let Some(prev) = self.last.replace(s) else { return None };
        let sent = s.sent_packets.saturating_sub(prev.sent_packets);
        let lost = s.lost_packets.saturating_sub(prev.lost_packets);
        // 按当前码率，积压的数据要发多久
        let queued = Duration::from_secs_f64(s.queued_bytes as f64 * 8.0 / (self.current.max(1) as f64 * 1000.0));
        let backlog = queued > QUEUE_LIMIT;
        let (sent2, lost2) = (sent + self.prev.0, lost + self.prev.1);
        let backlog2 = backlog && self.prev.2;
        self.prev = (sent, lost, backlog);
        let (rate, min) = if self.retransmits { (LOSS_RATE, LOSS_MIN_PACKETS) } else { (LEGACY_LOSS_RATE, LEGACY_LOSS_MIN_PACKETS) };
        // 两秒发了不到 60 个包时丢包率没有意义
        let lossy = sent2 >= 60 && lost2 >= min && lost2 as f32 > sent2 as f32 * rate;
        if backlog2 || lossy {
            self.stable_since = now;
            let recently = self.last_down.is_some_and(|t| now.duration_since(t) < DOWN_GAP);
            if recently || self.current <= MIN_KBPS {
                return None;
            }
            let next = ((self.current as f32 * 0.7) as u32).max(MIN_KBPS);
            self.current = next;
            self.last_down = Some(now);
            return Some(next);
        }
        if backlog || lost > 0 && lost as f32 > sent as f32 * rate {
            // 有苗头但还不够：先不升
            self.stable_since = now;
            return None;
        }
        if self.current < self.ceiling && now.duration_since(self.stable_since) >= UP_AFTER {
            let next = ((self.current as f32 * 1.3) as u32).min(self.ceiling);
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

    fn s(sent: u64, lost: u64, queued: u64) -> Sample {
        Sample { sent_packets: sent, lost_packets: lost, queued_bytes: queued }
    }

    fn sec(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn sporadic_wifi_loss_does_not_back_off() {
        let t0 = Instant::now();
        let mut r = RateCtl::new(12_000, t0);
        r.on_sample(s(0, 0, 0), t0);
        // 每秒 40 个包丢 1~2 个（3%~5%）：以前每次都降，一路降到 300 kbps
        let mut sent = 0;
        let mut lost = 0;
        for i in 1..=60 {
            sent += 40;
            lost += 1 + i % 2;
            assert_eq!(r.on_sample(s(sent, lost, 0), t0 + sec(i)), None, "第 {i} 秒");
        }
        assert_eq!(r.current(), 12_000);
    }

    #[test]
    fn backlog_backs_off_then_recovers() {
        let t0 = Instant::now();
        let mut r = RateCtl::new(12_000, t0);
        r.on_sample(s(0, 0, 0), t0);
        // 12 Mbps 时 300 毫秒约 450 KB；只有一秒积压（关键帧刚发出去）不算
        assert_eq!(r.on_sample(s(1000, 0, 600_000), t0 + sec(1)), None);
        assert_eq!(r.on_sample(s(2000, 0, 0), t0 + sec(2)), None);
        // 连续两秒积压：降到七成
        assert_eq!(r.on_sample(s(3000, 0, 600_000), t0 + sec(3)), None);
        assert_eq!(r.on_sample(s(4000, 0, 600_000), t0 + sec(4)), Some(8_400));
        // 两秒内不再降
        assert_eq!(r.on_sample(s(5000, 0, 600_000), t0 + sec(5)), None);
        assert_eq!(r.on_sample(s(6000, 0, 600_000), t0 + sec(6)), Some(5_880));
        // 平稳 4 秒升 30%，直到上限
        assert_eq!(r.on_sample(s(7000, 0, 0), t0 + sec(7)), None);
        assert_eq!(r.on_sample(s(8000, 0, 0), t0 + sec(10)), Some(7_643));
        assert_eq!(r.on_sample(s(9000, 0, 0), t0 + sec(14)), Some(9_935));
        assert_eq!(r.on_sample(s(10000, 0, 0), t0 + sec(18)), Some(12_000));
        assert_eq!(r.on_sample(s(11000, 0, 0), t0 + sec(30)), None);
    }

    #[test]
    fn heavy_loss_backs_off() {
        let t0 = Instant::now();
        let mut r = RateCtl::new(8_000, t0);
        r.set_retransmits(true);
        r.on_sample(s(0, 0, 0), t0);
        // 两秒合计 300 个包丢 90 个（30%）
        assert_eq!(r.on_sample(s(150, 30, 0), t0 + sec(1)), None);
        assert_eq!(r.on_sample(s(300, 90, 0), t0 + sec(2)), Some(5_600));
        // 10%（Wi-Fi 上常见，重传补得回来）不降
        let mut w = RateCtl::new(8_000, t0);
        w.set_retransmits(true);
        w.on_sample(s(0, 0, 0), t0);
        for i in 1..=10u64 {
            assert_eq!(w.on_sample(s(300 * i, 30 * i, 0), t0 + sec(i)), None);
        }
        // 不会补要的老主控端：10% 丢包要降（帧变小才收得齐）
        let mut l = RateCtl::new(8_000, t0);
        l.on_sample(s(0, 0, 0), t0);
        assert_eq!(l.on_sample(s(300, 30, 0), t0 + sec(1)), Some(5_600));
        // 包太少不算丢包率
        let mut q = RateCtl::new(8_000, t0);
        q.on_sample(s(0, 0, 0), t0);
        assert_eq!(q.on_sample(s(20, 9, 0), t0 + sec(1)), None);
        assert_eq!(q.on_sample(s(40, 19, 0), t0 + sec(2)), None);
    }

    #[test]
    fn floor_and_ceiling_changes() {
        let t0 = Instant::now();
        let mut r = RateCtl::new(400, t0);
        r.on_sample(s(0, 0, 0), t0);
        assert_eq!(r.on_sample(s(200, 100, 0), t0 + sec(1)), Some(MIN_KBPS));
        assert_eq!(r.on_sample(s(400, 200, 0), t0 + sec(4)), None);
        assert_eq!(r.set_ceiling(4000, t0), 4000);
        assert_eq!(r.set_ceiling(10000, t0), 10000);
        assert_eq!(r.set_ceiling(10, t0), MIN_KBPS);
    }
}
