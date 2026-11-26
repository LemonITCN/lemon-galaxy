//! 画面分片重组（丢片重传版，被控端有能力 video_nack 时用；docs/protocol.md 8.3）。
//!
//! 老办法（session.rs 的 `Reassembler`）丢一片整帧作废、之后等关键帧。Wi-Fi 上不拥塞也会成片丢包
//! （Mac 用纯 UDP 实测 5%～20%），一帧几十上百片几乎帧帧都缺，于是不停要关键帧、画面一直停着，
//! 被控端的码率自适应又把丢包当拥塞一路降码率，最后满屏马赛克。
//!
//! 这里改成：
//! - **按帧号顺序交给解码器**：后面的帧先收齐也要等前面的（P 帧依赖上一帧）。
//! - **缺了就要**：一帧里后面的分片到了、前面的没到，或者后一帧的分片到了、这一帧还缺（被控端按顺序匀速发），
//!   立即发 `Nack` 要缺的那几片；帧号跳过的整帧要整帧。最后几片丢了又没有后续数据时，安静 [`tail_wait`] 后再要。
//!   要过还没到的，隔 [`renack_wait`] 再要，每帧最多 [`MAX_NACKS`] 轮。
//! - **补不回来才算丢帧**：挡在最前面的帧等了 [`give_up_after`] 还不齐，就按老办法从下一个关键帧开始
//!   （调用方要关键帧）。局域网往返几毫秒，通常补一次十几毫秒就齐了。

use bytes::Bytes;
use galaxy_proto::peer::{FragNack, VideoFragHeader};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// 每帧最多要几轮
pub const MAX_NACKS: u8 = 4;
/// 帧号一次跳过超过这么多帧就不逐个要了，直接等关键帧
const MAX_GAP: u32 = 64;

/// 最后一片之后安静多久认为剩下的丢了（被控端匀速发，分片间隔远小于这个）
pub fn tail_wait(rtt: Duration) -> Duration {
    (rtt * 3 / 2).max(Duration::from_millis(12))
}

/// 要过的分片多久还没到就再要
pub fn renack_wait(rtt: Duration) -> Duration {
    (rtt * 2).max(Duration::from_millis(25))
}

/// 挡在最前面的帧等多久还不齐就放弃（等关键帧）
pub fn give_up_after(rtt: Duration) -> Duration {
    (rtt * 8).max(Duration::from_millis(400))
}

/// 帧号 `a` 早于 `b`（回绕也对）
fn before(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) < 0
}

struct Slot {
    /// None：一片都没收到（帧号跳过时补的占位），不知道分了几片
    parts: Option<Vec<Option<Bytes>>>,
    have: usize,
    pts: u64,
    keyframe: bool,
    /// 发现这一帧的时刻（收到第一片，或发现它整帧缺）
    first_seen: Instant,
    /// 最近一次有分片到达
    last_seen: Instant,
    /// 收到的最大分片号
    max_idx: Option<u16>,
    /// 每片最近一次要的时刻（parts 为 None 时用 whole_req）
    req: Vec<Option<Instant>>,
    whole_req: Option<Instant>,
    rounds: u8,
}

impl Slot {
    fn empty(now: Instant) -> Self {
        Self { parts: None, have: 0, pts: 0, keyframe: false, first_seen: now, last_seen: now, max_idx: None, req: vec![], whole_req: None, rounds: 0 }
    }

    fn complete(&self) -> bool {
        self.parts.as_ref().is_some_and(|p| self.have == p.len())
    }

    /// 缺的分片里该要的：`upto` 之前（不含）缺的，且没要过或要过已超过 `renack`
    fn missing(&mut self, upto: usize, now: Instant, renack: Duration) -> Vec<u16> {
        let Some(parts) = self.parts.as_ref() else { return vec![] };
        let mut v = Vec::new();
        for i in 0..upto.min(parts.len()) {
            if parts[i].is_none() && self.req[i].is_none_or(|t| now.duration_since(t) >= renack) {
                v.push(i as u16);
            }
        }
        for &i in &v {
            self.req[i as usize] = Some(now);
        }
        v
    }

    fn assemble(self) -> (Vec<u8>, u64, bool) {
        let parts = self.parts.unwrap_or_default();
        let mut data = Vec::with_capacity(parts.iter().map(|x| x.as_ref().map_or(0, |b| b.len())).sum());
        for p in parts.into_iter().flatten() {
            data.extend_from_slice(&p);
        }
        (data, self.pts, self.keyframe)
    }
}

#[derive(Default, Debug)]
pub struct Out {
    /// 按顺序可以交给解码器的帧：(数据, pts, 是否关键帧)
    pub frames: Vec<(Vec<u8>, u64, bool)>,
    /// 要被控端补发的
    pub nacks: Vec<FragNack>,
    /// 这一步刚放弃（补不回来），开始等关键帧
    pub new_loss: bool,
}

pub struct OrderedAssembler {
    slots: HashMap<u32, Slot>,
    /// 下一个要交的帧号；None 表示还在等第一个（或丢帧后的）关键帧
    next: Option<u32>,
    /// 见过的最大帧号
    newest: Option<u32>,
}

impl Default for OrderedAssembler {
    fn default() -> Self {
        Self { slots: HashMap::new(), next: None, newest: None }
    }
}

impl OrderedAssembler {
    pub fn waiting_keyframe(&self) -> bool {
        self.next.is_none()
    }

    /// 还有没收齐的帧（调用方据此缩短检查间隔）
    pub fn incomplete(&self) -> bool {
        self.slots.values().any(|s| !s.complete())
    }

    pub fn push(&mut self, h: VideoFragHeader, payload: Bytes, now: Instant, rtt: Duration) -> Out {
        let mut out = Out::default();
        let id = h.frame_id;
        if self.next.is_some_and(|n| before(id, n)) {
            // 已经交过或放弃了的帧（补发迟到、重复）
            return out;
        }
        // 帧号跳过的整帧：补占位并整帧要（只在正常交付时；等关键帧时只关心关键帧）
        if let Some(newest) = self.newest {
            if before(newest, id) {
                let gap = id.wrapping_sub(newest) - 1;
                if gap > MAX_GAP && self.next.is_some() {
                    self.lose(&mut out);
                } else if self.next.is_some() {
                    for k in 1..=gap {
                        let mid = newest.wrapping_add(k);
                        let s = self.slots.entry(mid).or_insert_with(|| Slot::empty(now));
                        if s.parts.is_none() && s.whole_req.is_none() {
                            s.whole_req = Some(now);
                            s.rounds = 1;
                            out.nacks.push(FragNack { frame_id: mid, frags: vec![] });
                        }
                    }
                }
                self.newest = Some(id);
            }
        } else {
            self.newest = Some(id);
        }
        let renack = renack_wait(rtt);
        let s = self.slots.entry(id).or_insert_with(|| Slot::empty(now));
        if s.parts.is_none() {
            let n = h.frag_count.max(1) as usize;
            s.parts = Some(vec![None; n]);
            s.req = vec![None; n];
            if s.whole_req.is_some() {
                // 整帧要过：各片都算刚要过，不再立刻按片重复要
                s.req = vec![s.whole_req; n];
            }
        }
        s.last_seen = now;
        s.keyframe |= h.keyframe;
        if h.frag_idx == 0 {
            s.pts = h.pts_us;
        }
        let idx = h.frag_idx as usize;
        if let Some(parts) = s.parts.as_mut() {
            if idx < parts.len() && parts[idx].is_none() {
                parts[idx] = Some(payload);
                s.have += 1;
            }
        }
        s.max_idx = Some(s.max_idx.map_or(h.frag_idx, |m| m.max(h.frag_idx)));
        // 同一帧里后面的到了、前面的没到：前面的丢了
        let want_this = self.next.is_some() || s.keyframe;
        if want_this {
            let miss = s.missing(idx, now, renack);
            if !miss.is_empty() {
                s.rounds = s.rounds.max(1);
                out.nacks.push(FragNack { frame_id: id, frags: miss });
            }
        }
        // 后一帧的分片到了：前面没收齐的帧剩下的都丢了
        let earlier: Vec<u32> = self.slots.keys().copied().filter(|k| before(*k, id)).collect();
        for k in earlier {
            let normal = self.next.is_some();
            let s = self.slots.get_mut(&k).expect("刚列出");
            if s.complete() || !(normal || s.keyframe) {
                continue;
            }
            let n = s.parts.as_ref().map_or(0, |p| p.len());
            let miss = s.missing(n, now, renack);
            if !miss.is_empty() {
                s.rounds = s.rounds.max(1);
                out.nacks.push(FragNack { frame_id: k, frags: miss });
            }
        }
        self.deliver(&mut out);
        out
    }

    /// 没有新分片时调用（约每 10 毫秒）：尾部丢片再要、要过没到的再要、补不回来的放弃。
    pub fn tick(&mut self, now: Instant, rtt: Duration) -> Out {
        let mut out = Out::default();
        let (tail, renack) = (tail_wait(rtt), renack_wait(rtt));
        let normal = self.next.is_some();
        let mut ids: Vec<u32> = self.slots.keys().copied().collect();
        ids.sort_by(|a, b| if before(*a, *b) { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater });
        for id in ids {
            let s = self.slots.get_mut(&id).expect("刚列出");
            if s.complete() || !(normal || s.keyframe) {
                continue;
            }
            if s.rounds >= MAX_NACKS {
                continue;
            }
            match s.parts.as_ref().map(|p| p.len()) {
                None => {
                    if s.whole_req.is_none_or(|t| now.duration_since(t) >= renack) {
                        s.whole_req = Some(now);
                        s.rounds += 1;
                        out.nacks.push(FragNack { frame_id: id, frags: vec![] });
                    }
                }
                Some(n) => {
                    if now.duration_since(s.last_seen) < tail {
                        continue;
                    }
                    let miss = s.missing(n, now, renack);
                    if !miss.is_empty() {
                        s.rounds += 1;
                        out.nacks.push(FragNack { frame_id: id, frags: miss });
                    }
                }
            }
        }
        // 挡在最前面的帧补不回来：放弃，等关键帧
        if let Some(n) = self.next {
            if let Some(s) = self.slots.get(&n) {
                if !s.complete() && now.duration_since(s.first_seen) >= give_up_after(rtt) {
                    self.lose(&mut out);
                }
            }
        } else {
            // 等关键帧时，太旧的非关键帧不用留
            let limit = give_up_after(rtt) * 2;
            self.slots.retain(|_, s| s.keyframe || now.duration_since(s.last_seen) < limit);
        }
        self.deliver(&mut out);
        out
    }

    fn lose(&mut self, out: &mut Out) {
        if self.next.is_some() {
            self.next = None;
            out.new_loss = true;
            out.nacks.clear();
            // 非关键帧都用不上了
            self.slots.retain(|_, s| s.keyframe);
        }
    }

    fn deliver(&mut self, out: &mut Out) {
        if self.next.is_none() {
            // 等关键帧：最新的一个收齐的关键帧，交出去，从它往后接着交
            let key = self.slots.iter().filter(|(_, s)| s.keyframe && s.complete()).map(|(k, _)| *k).reduce(|a, b| if before(a, b) { b } else { a });
            let Some(k) = key else { return };
            self.slots.retain(|id, _| !before(*id, k));
            let s = self.slots.remove(&k).expect("刚找到");
            out.frames.push(s.assemble());
            self.next = Some(k.wrapping_add(1));
        }
        while let Some(n) = self.next {
            if !self.slots.get(&n).is_some_and(|s| s.complete()) {
                break;
            }
            let s = self.slots.remove(&n).expect("刚看过");
            out.frames.push(s.assemble());
            self.next = Some(n.wrapping_add(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RTT: Duration = Duration::from_millis(5);

    fn frag(id: u32, idx: u16, count: u16, key: bool) -> (VideoFragHeader, Bytes) {
        (VideoFragHeader { channel: 1, frame_id: id, frag_idx: idx, frag_count: count, keyframe: key, pts_us: id as u64 }, Bytes::from(vec![id as u8, idx as u8]))
    }

    fn push(a: &mut OrderedAssembler, id: u32, idx: u16, count: u16, key: bool, now: Instant) -> Out {
        let (h, p) = frag(id, idx, count, key);
        a.push(h, p, now, RTT)
    }

    fn ids(o: &Out) -> Vec<u32> {
        o.frames.iter().map(|f| f.1 as u32).collect()
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn in_order_without_loss() {
        let mut a = OrderedAssembler::default();
        let t = Instant::now();
        assert!(a.waiting_keyframe());
        // 第一个关键帧之前的帧不交
        assert!(ids(&push(&mut a, 0, 0, 1, false, t)).is_empty());
        push(&mut a, 1, 0, 2, true, t);
        assert_eq!(ids(&push(&mut a, 1, 1, 2, true, t)), vec![1]);
        assert_eq!(ids(&push(&mut a, 2, 0, 1, false, t)), vec![2]);
        assert!(!a.waiting_keyframe());
        assert!(!a.incomplete());
    }

    #[test]
    fn gap_in_frame_is_nacked_and_later_frames_wait() {
        let mut a = OrderedAssembler::default();
        let t = Instant::now();
        push(&mut a, 0, 0, 1, true, t);
        // 帧 1 共 4 片，第 1 片丢了：第 2 片一到就要第 1 片
        push(&mut a, 1, 0, 4, false, t);
        let o = push(&mut a, 1, 2, 4, false, t);
        assert_eq!(o.nacks, vec![FragNack { frame_id: 1, frags: vec![1] }]);
        push(&mut a, 1, 3, 4, false, t);
        // 帧 2 先收齐也要等帧 1
        assert!(ids(&push(&mut a, 2, 0, 1, false, t)).is_empty());
        // 补发到了：1、2 一起交
        let o = push(&mut a, 1, 1, 4, false, t + ms(6));
        assert_eq!(ids(&o), vec![1, 2]);
        assert!(!o.new_loss);
    }

    #[test]
    fn whole_missing_frame_is_nacked() {
        let mut a = OrderedAssembler::default();
        let t = Instant::now();
        push(&mut a, 0, 0, 1, true, t);
        // 帧 1、2 一片都没到，帧 3 到了
        let o = push(&mut a, 3, 0, 1, false, t);
        assert_eq!(o.nacks, vec![FragNack { frame_id: 1, frags: vec![] }, FragNack { frame_id: 2, frags: vec![] }]);
        assert!(o.frames.is_empty());
        push(&mut a, 1, 0, 2, false, t + ms(5));
        assert_eq!(ids(&push(&mut a, 1, 1, 2, false, t + ms(5))), vec![1]);
        assert_eq!(ids(&push(&mut a, 2, 0, 1, false, t + ms(6))), vec![2, 3]);
    }

    #[test]
    fn tail_loss_nacked_after_quiet() {
        let mut a = OrderedAssembler::default();
        let t = Instant::now();
        push(&mut a, 0, 0, 1, true, t);
        // 帧 1 共 3 片，最后一片丢了，之后画面不动没有后续数据
        push(&mut a, 1, 0, 3, false, t);
        push(&mut a, 1, 1, 3, false, t);
        assert!(a.incomplete());
        assert!(a.tick(t + ms(5), RTT).nacks.is_empty());
        let o = a.tick(t + ms(12), RTT);
        assert_eq!(o.nacks, vec![FragNack { frame_id: 1, frags: vec![2] }]);
        // 刚要过不重复要；隔 25 毫秒再要
        assert!(a.tick(t + ms(20), RTT).nacks.is_empty());
        assert_eq!(a.tick(t + ms(40), RTT).nacks, vec![FragNack { frame_id: 1, frags: vec![2] }]);
        assert_eq!(ids(&push(&mut a, 1, 2, 3, false, t + ms(41))), vec![1]);
    }

    #[test]
    fn later_frame_reveals_tail_loss() {
        let mut a = OrderedAssembler::default();
        let t = Instant::now();
        push(&mut a, 0, 0, 1, true, t);
        push(&mut a, 1, 0, 3, false, t);
        // 帧 1 的第 1、2 片没到，帧 2 的分片到了：立即要
        let o = push(&mut a, 2, 0, 2, false, t);
        assert_eq!(o.nacks, vec![FragNack { frame_id: 1, frags: vec![1, 2] }]);
    }

    #[test]
    fn gives_up_then_waits_for_keyframe() {
        let mut a = OrderedAssembler::default();
        let t = Instant::now();
        push(&mut a, 0, 0, 1, true, t);
        push(&mut a, 1, 0, 2, false, t);
        push(&mut a, 2, 0, 1, false, t);
        let mut lost = false;
        let mut rounds = 0;
        let mut now = t;
        while now < t + ms(450) {
            now += ms(10);
            let o = a.tick(now, RTT);
            rounds += o.nacks.len();
            lost |= o.new_loss;
        }
        assert!(lost && a.waiting_keyframe());
        // 每帧最多要 MAX_NACKS 轮（第一轮在帧 2 到达时）
        assert_eq!(rounds, MAX_NACKS as usize - 1);
        // 非关键帧不交，关键帧来了接着交
        assert!(ids(&push(&mut a, 3, 0, 1, false, now)).is_empty());
        assert_eq!(ids(&push(&mut a, 4, 0, 1, true, now)), vec![4]);
        assert_eq!(ids(&push(&mut a, 5, 0, 1, false, now)), vec![5]);
    }

    #[test]
    fn keyframe_gaps_nacked_while_waiting() {
        let mut a = OrderedAssembler::default();
        let t = Instant::now();
        // 刚打开：关键帧丢了中间一片也要补，不必再要新关键帧
        push(&mut a, 0, 0, 3, true, t);
        let o = push(&mut a, 0, 2, 3, true, t);
        assert_eq!(o.nacks, vec![FragNack { frame_id: 0, frags: vec![1] }]);
        assert_eq!(ids(&push(&mut a, 0, 1, 3, true, t + ms(5))), vec![0]);
        // 等关键帧期间非关键帧缺片不要
        let mut b = OrderedAssembler::default();
        push(&mut b, 7, 0, 3, false, t);
        assert!(push(&mut b, 7, 2, 3, false, t).nacks.is_empty());
    }

    #[test]
    fn large_gap_waits_for_keyframe() {
        let mut a = OrderedAssembler::default();
        let t = Instant::now();
        push(&mut a, 0, 0, 1, true, t);
        let o = push(&mut a, 200, 0, 1, false, t);
        assert!(o.new_loss && o.nacks.is_empty());
        assert!(a.waiting_keyframe());
    }

    #[test]
    fn duplicates_and_late_frames_ignored() {
        let mut a = OrderedAssembler::default();
        let t = Instant::now();
        push(&mut a, 0, 0, 1, true, t);
        assert!(push(&mut a, 0, 0, 1, true, t).frames.is_empty());
        push(&mut a, 1, 0, 2, false, t);
        push(&mut a, 1, 0, 2, false, t);
        assert_eq!(ids(&push(&mut a, 1, 1, 2, false, t)), vec![1]);
    }

    #[test]
    fn frame_id_wraps() {
        let mut a = OrderedAssembler::default();
        let t = Instant::now();
        assert_eq!(ids(&push(&mut a, u32::MAX, 0, 1, true, t)), vec![u32::MAX]);
        let o = push(&mut a, 1, 0, 1, false, t);
        assert_eq!(o.nacks, vec![FragNack { frame_id: 0, frags: vec![] }]);
        assert_eq!(ids(&push(&mut a, 0, 0, 1, false, t)), vec![0, 1]);
    }
}
