//! 采集线程的编码节奏与交给编码器的码率（Linux、Windows 被控端的采集线程共用，平台层只管截屏与编码）。
//!
//! 管两件事：
//!
//! 1. **按实际帧率补足每帧预算**。NVENC、VA-API、Windows 的 Media Foundation 编码器做 CBR 时按配置的帧率分码率：
//!    每帧平均 = 码率 ÷ 帧率，并不看两帧之间实际隔了多久。桌面画面只在变化时才出帧，实际每秒常常只有几帧到十几帧，
//!    按主控要求的 60 帧分下来，12 Mbps 时每帧只有约 25 KB，整屏一变就满屏马赛克，实际只用掉选定码率的几分之一
//!    （Ubuntu 26.04 + RTX 3080 Ti 实测：窗口每 100 毫秒变一次时约 11 帧/秒、每帧约 25 KB、实际 2.2 Mbps）。
//!    所以按最近半秒的实际帧率把交给编码器的码率放大（最多 [`MAX_SCALE`] 倍），让实际码率接近要求的码率。
//!    按时间戳分码率的编码器（openh264 的时间戳模式、macOS VideoToolbox）不用放大。
//!
//! 2. **画面停下后补几帧清晰的**。码率预算再够，变化中的帧也比静止画面糊；以前停下后只有每秒一次保活重编，
//!    要好几秒才慢慢变清楚（刚停下时编码器的缓冲是空的，第一帧保活往往只有几百字节）。现在画面停下后按
//!    [`REFRESH_AFTER`] 的节奏把最后一帧再编几次（P 帧把上一帧的量化误差补上，内容没变时几乎不占码率），
//!    之后每秒一次保活（主控端据此判断画面没有卡死，能力 video_keepalive）。这一阵变化里最大的帧不到
//!    [`REFRESH_MIN_BYTES`]（光标闪烁、敲一个字）时不补：小块区域本来就编得清楚，软件编码补一次要几十毫秒 CPU。

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// 交给编码器的码率最多放大到要求的几倍（实际帧率低于配置帧率的 1/4 时不再放大，免得画面突然连续大变时一下子冲出太多）
pub const MAX_SCALE: u32 = 4;
/// 画面停下后，距最后一次变化多久补编一次（之后按 [`KEEPALIVE`] 保活）
pub const REFRESH_AFTER: [Duration; 4] = [Duration::from_millis(80), Duration::from_millis(200), Duration::from_millis(450), Duration::from_millis(900)];
/// 画面一直不变时多久重编一帧
pub const KEEPALIVE: Duration = Duration::from_secs(1);
/// 这一阵变化里最大的帧小于这么多字节就不补编
pub const REFRESH_MIN_BYTES: usize = 4096;
/// 估算实际帧率的窗口
const WINDOW: Duration = Duration::from_millis(500);
/// 放大倍数变化不到这么多就不改编码器（改码率虽然便宜，也不必每帧都改）
const RETUNE_RATIO: f32 = 0.2;
/// 两次因帧率变化改码率至少隔多久（主控 / 自适应改了要求的码率时立即改）
const RETUNE_GAP: Duration = Duration::from_millis(250);

/// VBV 缓冲按要求码率的多少毫秒算（决定一帧最大能多大，关键帧、大面积变化、补编时用得上）。
/// 按时间算、不随放大倍数变：12 Mbps 时单帧最多约 375 KB，不至于一帧冲出几百个数据报。
pub const VBV_MS: u32 = 250;

/// 交给编码器的码率：平均码率与 VBV 缓冲大小。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncoderRate {
    pub kbps: u32,
    pub vbv_kbits: u32,
}

impl EncoderRate {
    /// 不放大时：平均码率 `kbps`，VBV 为它的 [`VBV_MS`] 毫秒
    pub fn plain(kbps: u32) -> Self {
        Self { kbps, vbv_kbits: (kbps as u64 * VBV_MS as u64 / 1000) as u32 }
    }

    /// 编码器用的 VBV（bit）：不小于配置帧率下两帧的平均量
    pub fn vbv_bits(&self, bps: u32, fps: u32) -> u32 {
        (self.vbv_kbits.saturating_mul(1000)).max(bps / fps.max(1) * 2)
    }
}

/// 这一帧为什么要编
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// 画面变了（含画面里的光标动了、要关键帧、光标显示切换）
    Change,
    /// 画面停下后的补编
    Refresh,
    /// 长时间不变时的保活
    Keepalive,
}

pub struct Pacer {
    nominal_fps: u32,
    target_kbps: u32,
    applied_kbps: u32,
    /// 编码器能运行时改码率、而且按配置帧率分码率（要放大）
    scale: bool,
    /// 要求的码率变了，还没交给编码器
    target_dirty: bool,
    /// 最近 [`WINDOW`] 内「画面变了」的编码时刻
    changes: VecDeque<Instant>,
    last_change: Option<Instant>,
    /// 这次停下后已经补编了几次
    refreshed: usize,
    /// 这一阵变化（上次补编完或停了一秒以后）里最大的帧
    burst_max: usize,
    last_encode: Option<Instant>,
    last_retune: Option<Instant>,
}

impl Pacer {
    /// `nominal_fps` 是编码器配置的帧率，`target_kbps` 是内核要求的码率（主控上限与自适应的结果）。
    pub fn new(nominal_fps: u32, target_kbps: u32) -> Self {
        Self {
            nominal_fps: nominal_fps.max(1),
            target_kbps,
            applied_kbps: target_kbps,
            scale: false,
            target_dirty: false,
            changes: VecDeque::new(),
            last_change: None,
            refreshed: REFRESH_AFTER.len(),
            burst_max: 0,
            last_encode: None,
            last_retune: None,
        }
    }

    pub fn target(&self) -> u32 {
        self.target_kbps
    }

    /// 内核要求的码率变了。
    pub fn set_target(&mut self, kbps: u32) {
        if kbps != self.target_kbps {
            self.target_kbps = kbps;
            self.target_dirty = true;
        }
    }

    /// 编码器（重新）建好了：它现在按 `applied_kbps` 编码；`scale` 为真表示它能运行时改码率并且按配置帧率分码率。
    pub fn encoder_ready(&mut self, applied_kbps: u32, scale: bool) {
        self.applied_kbps = applied_kbps;
        self.scale = scale;
        self.target_dirty = applied_kbps != self.target_kbps;
        self.last_retune = None;
    }

    /// 现在该交给编码器的码率。
    pub fn wanted_kbps(&self, now: Instant) -> u32 {
        if !self.scale {
            return self.target_kbps;
        }
        let recent = self.changes.iter().filter(|t| now.duration_since(**t) < WINDOW).count() as u32;
        // 半秒内的帧数 × 2 ≈ 每秒帧数；放大倍数 = 配置帧率 ÷ 实际帧率，限在 1..=MAX_SCALE
        let fps = (recent * 2).max(1);
        let scale = (self.nominal_fps as f32 / fps as f32).clamp(1.0, MAX_SCALE as f32);
        ((self.target_kbps as f32 * scale) as u32).min(500_000)
    }

    /// 编码器该改成的码率；不用改时返回 None。调用方改成功后码率即生效（改不了时调用方重建编码器，再调 [`Self::encoder_ready`]）。
    /// VBV 按要求的码率算，不随放大倍数变。
    pub fn retune(&mut self, now: Instant) -> Option<EncoderRate> {
        let want = self.wanted_kbps(now);
        if want == self.applied_kbps {
            self.target_dirty = false;
            return None;
        }
        if !self.target_dirty {
            let diff = (want as f32 - self.applied_kbps as f32).abs() / self.applied_kbps.max(1) as f32;
            if diff < RETUNE_RATIO || self.last_retune.is_some_and(|t| now.duration_since(t) < RETUNE_GAP) {
                return None;
            }
        }
        self.target_dirty = false;
        self.applied_kbps = want;
        self.last_retune = Some(now);
        Some(EncoderRate { kbps: want, vbv_kbits: EncoderRate::plain(self.target_kbps).vbv_kbits })
    }

    /// 这一轮要不要编码。`changed` 为真表示画面有变化、必须立即编码。
    pub fn decide(&self, changed: bool, now: Instant) -> Option<Why> {
        if changed {
            return Some(Why::Change);
        }
        if let (Some(t), Some(after)) = (self.last_change, REFRESH_AFTER.get(self.refreshed)) {
            if now.duration_since(t) >= *after && self.burst_max >= REFRESH_MIN_BYTES {
                return Some(Why::Refresh);
            }
        }
        match self.last_encode {
            Some(t) if now.duration_since(t) < KEEPALIVE => None,
            _ => Some(Why::Keepalive),
        }
    }

    /// 编完一帧（编码器有没有出数据都算），`bytes` 是这次编出来的字节数。
    pub fn encoded(&mut self, why: Why, now: Instant, bytes: usize) {
        self.last_encode = Some(now);
        match why {
            Why::Change => {
                // 上一阵的补编已经补完（或者停了一秒以上）：新的一阵
                let quiet = self.last_change.is_none_or(|t| now.duration_since(t) >= KEEPALIVE);
                if quiet || self.refreshed >= REFRESH_AFTER.len() {
                    self.burst_max = 0;
                }
                self.burst_max = self.burst_max.max(bytes);
                self.last_change = Some(now);
                self.refreshed = 0;
                self.changes.push_back(now);
                while self.changes.front().is_some_and(|t| now.duration_since(*t) >= WINDOW) {
                    self.changes.pop_front();
                }
            }
            Why::Refresh => {
                // 跳过已经过时的补编点（编码慢或线程被耽搁时不连着补好几次）
                let since = self.last_change.map(|t| now.duration_since(t)).unwrap_or_default();
                self.refreshed += 1;
                while REFRESH_AFTER.get(self.refreshed).is_some_and(|a| since >= *a) {
                    self.refreshed += 1;
                }
            }
            Why::Keepalive => {
                // 异步编码器的输出可能晚一次才出来：变化帧的大小这时才知道
                if self.refreshed < REFRESH_AFTER.len() {
                    self.burst_max = self.burst_max.max(bytes);
                }
            }
        }
    }

    /// 下一次需要编码（补编或保活）的时刻；采集线程据此决定最多等多久新画面。
    pub fn next_due(&self, now: Instant) -> Instant {
        let keep = self.last_encode.map(|t| t + KEEPALIVE).unwrap_or(now);
        match (self.last_change, REFRESH_AFTER.get(self.refreshed)) {
            (Some(t), Some(after)) if self.burst_max >= REFRESH_MIN_BYTES => keep.min(t + *after),
            _ => keep,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn refresh_after_change_then_keepalive() {
        let t0 = Instant::now();
        let mut p = Pacer::new(60, 12_000);
        // 刚开始没有编过：先编一帧（保活）
        assert_eq!(p.decide(false, t0), Some(Why::Keepalive));
        p.encoded(Why::Change, t0, 50_000);
        assert_eq!(p.decide(false, t0 + ms(50)), None);
        assert_eq!(p.decide(false, t0 + ms(80)), Some(Why::Refresh));
        p.encoded(Why::Refresh, t0 + ms(80), 20_000);
        assert_eq!(p.decide(false, t0 + ms(150)), None);
        assert_eq!(p.next_due(t0 + ms(150)), t0 + ms(200));
        p.encoded(Why::Refresh, t0 + ms(200), 20_000);
        p.encoded(Why::Refresh, t0 + ms(450), 20_000);
        p.encoded(Why::Refresh, t0 + ms(900), 20_000);
        // 补完了：之后只剩每秒保活
        assert_eq!(p.decide(false, t0 + ms(1500)), None);
        assert_eq!(p.next_due(t0 + ms(1500)), t0 + ms(1900));
        assert_eq!(p.decide(false, t0 + ms(1900)), Some(Why::Keepalive));
        p.encoded(Why::Keepalive, t0 + ms(1900), 100);
        // 新的变化重新开始补编
        assert_eq!(p.decide(true, t0 + ms(2000)), Some(Why::Change));
        p.encoded(Why::Change, t0 + ms(2000), 50_000);
        assert_eq!(p.decide(false, t0 + ms(2080)), Some(Why::Refresh));
    }

    #[test]
    fn tiny_changes_are_not_refreshed() {
        let t0 = Instant::now();
        let mut p = Pacer::new(60, 12_000);
        // 光标闪烁：变化帧只有几百字节，不补编，只保活
        p.encoded(Why::Change, t0, 300);
        assert_eq!(p.decide(false, t0 + ms(100)), None);
        assert_eq!(p.decide(false, t0 + ms(500)), None);
        assert_eq!(p.next_due(t0 + ms(500)), t0 + ms(1000));
        // 同一阵里有一帧大的：补编
        p.encoded(Why::Change, t0 + ms(600), 300);
        p.encoded(Why::Change, t0 + ms(650), 80_000);
        p.encoded(Why::Change, t0 + ms(700), 500);
        assert_eq!(p.decide(false, t0 + ms(780)), Some(Why::Refresh));
        for d in [780, 900, 1150, 1600] {
            p.encoded(Why::Refresh, t0 + ms(d), 10_000);
        }
        // 补完以后再来的小变化是新的一阵，不补
        p.encoded(Why::Change, t0 + ms(1700), 200);
        assert_eq!(p.decide(false, t0 + ms(1800)), None);
    }

    #[test]
    fn late_refresh_skips_stale_points() {
        let t0 = Instant::now();
        let mut p = Pacer::new(60, 12_000);
        p.encoded(Why::Change, t0, 50_000);
        // 线程被耽搁到 500 毫秒才补：80、200、450 都过了，只补这一次，下一次在 900
        assert_eq!(p.decide(false, t0 + ms(500)), Some(Why::Refresh));
        p.encoded(Why::Refresh, t0 + ms(500), 20_000);
        assert_eq!(p.decide(false, t0 + ms(600)), None);
        assert_eq!(p.next_due(t0 + ms(600)), t0 + ms(900));
    }

    #[test]
    fn bitrate_scales_with_actual_frame_rate() {
        let t0 = Instant::now();
        let mut p = Pacer::new(60, 12_000);
        // 按配置帧率分码率的硬件编码器
        p.encoder_ready(12_000, true);
        // 静止（半秒内没有变化帧）：放大到上限 4 倍
        assert_eq!(p.retune(t0), Some(EncoderRate { kbps: 48_000, vbv_kbits: 3_000 }));
        assert_eq!(p.retune(t0 + ms(10)), None);
        // 每 100 毫秒变一次（约 10 帧/秒）：仍是 4 倍
        let mut t = t0;
        for _ in 0..10 {
            t += ms(100);
            p.encoded(Why::Change, t, 50_000);
        }
        assert_eq!(p.wanted_kbps(t), 48_000);
        // 每 34 毫秒一帧（约 30 帧/秒）：2 倍
        for _ in 0..30 {
            t += ms(34);
            p.encoded(Why::Change, t, 50_000);
        }
        assert_eq!(p.wanted_kbps(t), 24_000);
        assert_eq!(p.retune(t).map(|r| r.kbps), Some(24_000));
        // 60 帧/秒：不放大；距上次改不到 250 毫秒先不改
        for _ in 0..4 {
            t += ms(16);
            p.encoded(Why::Change, t, 50_000);
        }
        let _ = p.retune(t);
        for _ in 0..40 {
            t += ms(16);
            p.encoded(Why::Change, t, 50_000);
        }
        assert!(p.wanted_kbps(t) <= 12_500, "{}", p.wanted_kbps(t));
        assert_eq!(p.retune(t).map(|r| r.kbps <= 12_500), Some(true));
    }

    #[test]
    fn target_change_applies_immediately() {
        let t0 = Instant::now();
        let mut p = Pacer::new(60, 12_000);
        p.encoder_ready(12_000, true);
        assert_eq!(p.retune(t0).map(|r| r.kbps), Some(48_000));
        // 自适应降码率：不受 250 毫秒间隔限制
        p.set_target(8_400);
        assert_eq!(p.retune(t0 + ms(1)), Some(EncoderRate { kbps: 33_600, vbv_kbits: 2_100 }));
        // 按时间戳分码率的编码器（或不能运行时改码率）：只跟随要求的码率
        let mut q = Pacer::new(60, 6_000);
        q.encoder_ready(6_000, false);
        assert_eq!(q.retune(t0), None);
        q.set_target(4_200);
        assert_eq!(q.retune(t0), Some(EncoderRate::plain(4_200)));
        assert_eq!(q.retune(t0 + ms(500)), None);
    }

    #[test]
    fn vbv_floor_is_two_frames() {
        let r = EncoderRate::plain(1_000);
        assert_eq!(r.vbv_kbits, 250);
        // 码率放大 4 倍、配置 30 帧：两帧平均 = 4000k / 30 × 2 ≈ 266 kbit > 250 kbit
        assert_eq!(r.vbv_bits(4_000_000, 30), 266_666);
        assert_eq!(r.vbv_bits(1_000_000, 60), 250_000);
    }

    #[test]
    fn rebuilt_encoder_gets_current_target() {
        let t0 = Instant::now();
        let mut p = Pacer::new(30, 4_000);
        p.set_target(5_000);
        // 重建时按要求的码率建，建好后再放大
        p.encoder_ready(5_000, true);
        assert_eq!(p.retune(t0), Some(EncoderRate { kbps: 20_000, vbv_kbits: 1_250 }));
    }
}
