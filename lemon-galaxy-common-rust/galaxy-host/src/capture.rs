//! 同一块屏只采集编码一路，分发给所有在看它的会话（平台层按屏幕编号只管一份采集）。
//!
//! 第一个会话打开时让平台层开始采集；后来的会话直接挂上并请求一个关键帧；最后一个离开才停止。
//! 码率取各会话要求里最低的那个，免得网络差的一方被网络好的一方带着丢包。

use crate::platform::{CaptureStarted, FrameSink, HostPlatform};
use galaxy_proto::peer::VideoCodec;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::{Arc, Weak};

struct Sub {
    id: u64,
    sink: Arc<dyn FrameSink>,
    kbps: u32,
}

struct Shared {
    subs: Vec<Sub>,
    started: CaptureStarted,
    applied_kbps: u32,
}

pub struct CaptureHub {
    displays: Mutex<HashMap<u32, Shared>>,
    /// 开始 / 停止采集要串行（平台层的调用可能耗时、也可能同步回调）
    start_lock: Mutex<()>,
    next_id: Mutex<u64>,
    me: Weak<CaptureHub>,
}

struct FanSink {
    hub: Weak<CaptureHub>,
    display: u32,
}

impl FrameSink for FanSink {
    fn push(&self, data: Vec<u8>, pts_us: u64, keyframe: bool) {
        let Some(hub) = self.hub.upgrade() else { return };
        let sinks: Vec<Arc<dyn FrameSink>> = hub.displays.lock().get(&self.display).map(|s| s.subs.iter().map(|x| x.sink.clone()).collect()).unwrap_or_default();
        let n = sinks.len();
        let mut data = Some(data);
        for (i, s) in sinks.into_iter().enumerate() {
            // 最后一个直接拿走，前面的各复制一份
            let d = if i + 1 == n { data.take().unwrap_or_default() } else { data.as_ref().cloned().unwrap_or_default() };
            s.push(d, pts_us, keyframe);
        }
    }
    fn resized(&self, width: u32, height: u32) {
        let Some(hub) = self.hub.upgrade() else { return };
        let sinks: Vec<Arc<dyn FrameSink>> = {
            let mut g = hub.displays.lock();
            match g.get_mut(&self.display) {
                Some(s) => {
                    s.started.width = width;
                    s.started.height = height;
                    s.subs.iter().map(|x| x.sink.clone()).collect()
                }
                None => vec![],
            }
        };
        for s in sinks {
            s.resized(width, height);
        }
    }
}

impl CaptureHub {
    pub fn new() -> Arc<Self> {
        Arc::new_cyclic(|me| Self { displays: Mutex::new(HashMap::new()), start_lock: Mutex::new(()), next_id: Mutex::new(1), me: me.clone() })
    }

    /// 开始看某块屏。返回订阅号与当前的分辨率、编码。
    #[allow(clippy::too_many_arguments)]
    pub fn subscribe(&self, platform: &Arc<dyn HostPlatform>, display: u32, max_fps: u32, kbps: u32, codecs: Vec<VideoCodec>, sink: Arc<dyn FrameSink>) -> Result<(u64, CaptureStarted), String> {
        let _g = self.start_lock.lock();
        let id = {
            let mut n = self.next_id.lock();
            *n += 1;
            *n
        };
        let joined = {
            let mut g = self.displays.lock();
            match g.get_mut(&display) {
                Some(s) => {
                    if !codecs.is_empty() && !codecs.contains(&s.started.codec) {
                        return Err("这块屏正被别的会话以对方不支持的编码查看".into());
                    }
                    s.subs.push(Sub { id, sink: sink.clone(), kbps });
                    Some(s.started.clone())
                }
                None => None,
            }
        };
        if let Some(started) = joined {
            // 新来的会话要从关键帧开始解码
            platform.request_keyframe(display);
            self.apply_bitrate(platform, display);
            return Ok((id, started));
        }
        let fan: Arc<dyn FrameSink> = Arc::new(FanSink { hub: self.me.clone(), display });
        // 先登记再开始：平台层可能在 start_capture 返回前就推第一帧
        self.displays.lock().insert(display, Shared { subs: vec![Sub { id, sink, kbps }], started: CaptureStarted { width: 0, height: 0, codec: VideoCodec::H264 }, applied_kbps: kbps });
        match platform.start_capture(display, max_fps, kbps, codecs, fan) {
            Ok(started) => {
                if let Some(s) = self.displays.lock().get_mut(&display) {
                    s.started = started.clone();
                }
                Ok((id, started))
            }
            Err(e) => {
                self.displays.lock().remove(&display);
                Err(e)
            }
        }
    }

    /// 某个会话不看了；没人看了就停止采集。
    pub fn unsubscribe(&self, platform: &Arc<dyn HostPlatform>, display: u32, id: u64) {
        let _g = self.start_lock.lock();
        let stop = {
            let mut g = self.displays.lock();
            let Some(s) = g.get_mut(&display) else { return };
            s.subs.retain(|x| x.id != id);
            if s.subs.is_empty() {
                g.remove(&display);
                true
            } else {
                false
            }
        };
        if stop {
            platform.stop_capture(display);
        } else {
            self.apply_bitrate(platform, display);
        }
    }

    /// 某个会话要求的码率变了（自适应或用户选择）。实际用所有会话里最低的。
    pub fn set_bitrate(&self, platform: &Arc<dyn HostPlatform>, display: u32, id: u64, kbps: u32) {
        if let Some(s) = self.displays.lock().get_mut(&display) {
            if let Some(x) = s.subs.iter_mut().find(|x| x.id == id) {
                x.kbps = kbps;
            }
        }
        self.apply_bitrate(platform, display);
    }

    fn apply_bitrate(&self, platform: &Arc<dyn HostPlatform>, display: u32) {
        let want = {
            let mut g = self.displays.lock();
            let Some(s) = g.get_mut(&display) else { return };
            let m = s.subs.iter().map(|x| x.kbps).min().unwrap_or(s.applied_kbps);
            if m == s.applied_kbps {
                return;
            }
            s.applied_kbps = m;
            m
        };
        platform.set_bitrate(display, want);
    }

    pub fn viewers(&self, display: u32) -> usize {
        self.displays.lock().get(&display).map(|s| s.subs.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[derive(Default)]
    struct FakePlatform {
        starts: AtomicU32,
        stops: AtomicU32,
        keyframes: AtomicU32,
        bitrate: AtomicU32,
        sink: Mutex<Option<Arc<dyn FrameSink>>>,
    }
    impl HostPlatform for FakePlatform {
        fn start_capture(&self, _d: u32, _f: u32, kbps: u32, _c: Vec<VideoCodec>, sink: Arc<dyn FrameSink>) -> Result<CaptureStarted, String> {
            self.starts.fetch_add(1, Ordering::Relaxed);
            self.bitrate.store(kbps, Ordering::Relaxed);
            *self.sink.lock() = Some(sink);
            Ok(CaptureStarted { width: 1920, height: 1080, codec: VideoCodec::H264 })
        }
        fn stop_capture(&self, _d: u32) {
            self.stops.fetch_add(1, Ordering::Relaxed);
        }
        fn request_keyframe(&self, _d: u32) {
            self.keyframes.fetch_add(1, Ordering::Relaxed);
        }
        fn set_bitrate(&self, _d: u32, kbps: u32) {
            self.bitrate.store(kbps, Ordering::Relaxed);
        }
    }

    #[derive(Default)]
    struct Counter(AtomicU32, AtomicU32);
    impl FrameSink for Counter {
        fn push(&self, d: Vec<u8>, _p: u64, _k: bool) {
            assert_eq!(d, vec![1, 2, 3]);
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        fn resized(&self, _w: u32, _h: u32) {
            self.1.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn two_viewers_share_one_capture() {
        let fp = Arc::new(FakePlatform::default());
        let p: Arc<dyn HostPlatform> = fp.clone();
        let hub = CaptureHub::new();
        let a = Arc::new(Counter::default());
        let b = Arc::new(Counter::default());
        let (ia, sa) = hub.subscribe(&p, 0, 60, 8000, vec![VideoCodec::H264], a.clone()).unwrap();
        assert_eq!((sa.width, sa.height), (1920, 1080));
        let (ib, sb) = hub.subscribe(&p, 0, 60, 3000, vec![], b.clone()).unwrap();
        assert_eq!(sb.width, 1920);
        assert_eq!(fp.starts.load(Ordering::Relaxed), 1);
        assert_eq!(fp.keyframes.load(Ordering::Relaxed), 1);
        // 码率取最低
        assert_eq!(fp.bitrate.load(Ordering::Relaxed), 3000);
        let sink = fp.sink.lock().clone().unwrap();
        sink.push(vec![1, 2, 3], 0, true);
        sink.resized(1280, 720);
        assert_eq!((a.0.load(Ordering::Relaxed), b.0.load(Ordering::Relaxed)), (1, 1));
        assert_eq!((a.1.load(Ordering::Relaxed), b.1.load(Ordering::Relaxed)), (1, 1));
        // 一个人走了，另一个照常收，码率回到剩下的那个
        hub.unsubscribe(&p, 0, ib);
        assert_eq!(fp.stops.load(Ordering::Relaxed), 0);
        assert_eq!(fp.bitrate.load(Ordering::Relaxed), 8000);
        sink.push(vec![1, 2, 3], 0, false);
        assert_eq!((a.0.load(Ordering::Relaxed), b.0.load(Ordering::Relaxed)), (2, 1));
        // 新来的拿到的是当前分辨率
        let (ic, sc) = hub.subscribe(&p, 0, 60, 8000, vec![], Arc::new(Counter::default())).unwrap();
        assert_eq!((sc.width, sc.height), (1280, 720));
        hub.unsubscribe(&p, 0, ia);
        hub.unsubscribe(&p, 0, ic);
        assert_eq!(fp.stops.load(Ordering::Relaxed), 1);
        assert_eq!(hub.viewers(0), 0);
    }
}
