//! 会话录像（主控端）：把收到的 H.264 画面与 Opus 声音直接封装成分片 MP4（不重新编码）。
//!
//! 边录边写，每 2 秒一个分片，内存占用固定；程序意外退出时已写入的部分照样能播放。
//! 被控端分辨率变了（换屏、改分辨率）时自动分段：结束当前文件，下一个关键帧开始写 `名字-2.mp4`。

use muxide::api::AudioCodec;
use muxide::fragmented::{FragmentAudioConfig, FragmentConfig, FragmentedMuxer};
use parking_lot::Mutex;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

const VIDEO_TIMESCALE: u64 = 90_000;
const AUDIO_RATE: u64 = 48_000;
const OPUS_FRAME: u64 = 960;

/// 录完的结果。
#[derive(Clone, Debug, Default)]
pub struct RecordingSummary {
    /// 写出的文件（分辨率变化时不止一个）
    pub paths: Vec<PathBuf>,
    pub duration_ms: u64,
    pub bytes: u64,
    pub frames: u64,
}

struct Part {
    file: std::io::BufWriter<std::fs::File>,
    mux: FragmentedMuxer,
    width: u32,
    height: u32,
    last_video: Option<u64>,
    audio_next: Option<u64>,
}

struct State {
    base: PathBuf,
    with_audio: bool,
    start: Instant,
    dims: (u32, u32),
    part: Option<Part>,
    parts: u32,
    summary: RecordingSummary,
    error: Option<String>,
    finished: bool,
}

pub struct Recorder {
    st: Mutex<State>,
}

/// 按起始码拆 Annex B 码流，返回各个 NAL（不含起始码）。
pub fn split_annexb(b: &[u8]) -> Vec<&[u8]> {
    let mut out = vec![];
    let mut i = 0;
    let mut start: Option<usize> = None;
    while i + 3 <= b.len() {
        let sc3 = b[i] == 0 && b[i + 1] == 0 && b[i + 2] == 1;
        let sc4 = i + 4 <= b.len() && b[i] == 0 && b[i + 1] == 0 && b[i + 2] == 0 && b[i + 3] == 1;
        if sc3 || sc4 {
            if let Some(s) = start {
                out.push(&b[s..i]);
            }
            i += if sc4 { 4 } else { 3 };
            start = Some(i);
        } else {
            i += 1;
        }
    }
    if let Some(s) = start {
        if s < b.len() {
            out.push(&b[s..]);
        }
    }
    // 去掉 NAL 末尾多余的 0（下一个四字节起始码的第一个字节）
    out.into_iter()
        .map(|n| {
            let mut e = n.len();
            while e > 1 && n[e - 1] == 0 {
                e -= 1;
            }
            &n[..e]
        })
        .filter(|n| !n.is_empty())
        .collect()
}

impl Recorder {
    /// 准备录到 `path`（.mp4）。第一个关键帧到来时才建文件。
    pub fn new(path: &Path, with_audio: bool) -> Result<std::sync::Arc<Self>, String> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir).map_err(|e| format!("无法创建目录 {}：{e}", dir.display()))?;
            }
        }
        Ok(std::sync::Arc::new(Self {
            st: Mutex::new(State {
                base: path.to_path_buf(),
                with_audio,
                start: Instant::now(),
                dims: (0, 0),
                part: None,
                parts: 0,
                summary: RecordingSummary::default(),
                error: None,
                finished: false,
            }),
        }))
    }

    /// 画面尺寸（变了会在下一个关键帧开新文件）。
    pub fn on_config(&self, width: u32, height: u32) {
        let mut st = self.st.lock();
        if st.finished {
            return;
        }
        st.dims = (width, height);
        let changed = st.part.as_ref().map(|p| p.width != width || p.height != height).unwrap_or(false);
        if changed {
            close_part(&mut st);
        }
    }

    /// 一帧画面（Annex B，关键帧带参数集）。
    pub fn on_video(&self, annexb: &[u8], keyframe: bool) {
        let mut st = self.st.lock();
        if st.finished || st.error.is_some() {
            return;
        }
        if st.part.is_none() {
            if !keyframe {
                return;
            }
            if let Err(e) = open_part(&mut st, annexb) {
                st.error = Some(e);
                return;
            }
        }
        let now = st.start.elapsed();
        let mut sample = Vec::with_capacity(annexb.len() + 16);
        for n in split_annexb(annexb) {
            // 参数集放在文件头里；分隔符不要
            if matches!(n[0] & 0x1f, 7 | 8 | 9) {
                continue;
            }
            sample.extend_from_slice(&(n.len() as u32).to_be_bytes());
            sample.extend_from_slice(n);
        }
        if sample.is_empty() {
            return;
        }
        let Some(part) = st.part.as_mut() else { return };
        let mut ts = now.as_micros() as u64 * VIDEO_TIMESCALE / 1_000_000;
        if let Some(last) = part.last_video {
            ts = ts.max(last + 1);
        }
        part.last_video = Some(ts);
        let r = part.mux.write_video(ts, ts, &sample, keyframe);
        st.summary.frames += 1;
        if let Err(e) = r {
            st.error = Some(format!("写入画面失败：{e}"));
            return;
        }
        flush_if_ready(&mut st);
    }

    /// 一个 Opus 包（20 毫秒）。画面开始之前的声音不录。
    pub fn on_audio(&self, opus: &[u8]) {
        let mut st = self.st.lock();
        if st.finished || st.error.is_some() || !st.with_audio || opus.is_empty() {
            return;
        }
        let now = st.start.elapsed().as_micros() as u64 * AUDIO_RATE / 1_000_000;
        let Some(part) = st.part.as_mut() else { return };
        // 连续的包按 20 毫秒一包排；隔得太远（被控端静音时不发包）就按到达时间对齐
        let pts = match part.audio_next {
            Some(n) if now.abs_diff(n) < AUDIO_RATE / 10 => n,
            _ => now,
        };
        part.audio_next = Some(pts + OPUS_FRAME);
        if let Err(e) = part.mux.write_audio(pts, pts, opus) {
            st.error = Some(format!("写入声音失败：{e}"));
            return;
        }
        flush_if_ready(&mut st);
    }

    pub fn error(&self) -> Option<String> {
        self.st.lock().error.clone()
    }

    pub fn summary(&self) -> RecordingSummary {
        let st = self.st.lock();
        let mut s = st.summary.clone();
        s.duration_ms = st.start.elapsed().as_millis() as u64;
        s
    }

    /// 结束录像，写完剩下的数据。没收到过关键帧（什么都没录到）时返回错误。
    pub fn finish(&self) -> Result<RecordingSummary, String> {
        let mut st = self.st.lock();
        if !st.finished {
            st.finished = true;
            close_part(&mut st);
            st.summary.duration_ms = st.start.elapsed().as_millis() as u64;
        }
        if let Some(e) = st.error.clone() {
            return Err(e);
        }
        if st.summary.paths.is_empty() {
            return Err("没有录到画面（还没收到关键帧）".into());
        }
        Ok(st.summary.clone())
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

fn part_path(base: &Path, n: u32) -> PathBuf {
    if n <= 1 {
        return base.to_path_buf();
    }
    let stem = base.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "recording".into());
    let ext = base.extension().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "mp4".into());
    base.with_file_name(format!("{stem}-{n}.{ext}"))
}

fn open_part(st: &mut State, keyframe: &[u8]) -> Result<(), String> {
    let nals = split_annexb(keyframe);
    let sps = nals.iter().find(|n| n[0] & 0x1f == 7).ok_or("关键帧里没有 SPS")?.to_vec();
    let pps = nals.iter().find(|n| n[0] & 0x1f == 8).ok_or("关键帧里没有 PPS")?.to_vec();
    let (w, h) = st.dims;
    if w == 0 || h == 0 {
        return Err("还不知道画面尺寸".into());
    }
    let mut cfg = FragmentConfig::new(w, h, VIDEO_TIMESCALE as u32).with_fragment_duration_ms(2000);
    cfg.sps = sps;
    cfg.pps = pps;
    let mux = if st.with_audio {
        FragmentedMuxer::new_with_audio(cfg, FragmentAudioConfig { codec: AudioCodec::Opus, sample_rate: AUDIO_RATE as u32, channels: 2 })
    } else {
        FragmentedMuxer::new(cfg)
    };
    let mut mux = mux.map_err(|e| format!("无法开始录像：{e}"))?;
    st.parts += 1;
    let path = part_path(&st.base, st.parts);
    let f = std::fs::File::create(&path).map_err(|e| format!("无法创建 {}：{e}", path.display()))?;
    let mut file = std::io::BufWriter::new(f);
    let init = mux.init_segment();
    file.write_all(&init).map_err(|e| format!("写入失败：{e}"))?;
    st.summary.bytes += init.len() as u64;
    st.summary.paths.push(path);
    st.part = Some(Part { file, mux, width: w, height: h, last_video: None, audio_next: None });
    Ok(())
}

fn flush_if_ready(st: &mut State) {
    let Some(part) = st.part.as_mut() else { return };
    if !part.mux.ready_to_flush() {
        return;
    }
    if let Some(seg) = part.mux.flush_segment() {
        let r = part.file.write_all(&seg).and_then(|_| part.file.flush());
        st.summary.bytes += seg.len() as u64;
        if let Err(e) = r {
            st.error = Some(format!("写入失败（磁盘满了？）：{e}"));
        }
    }
}

fn close_part(st: &mut State) {
    if let Some(mut part) = st.part.take() {
        if let Some(seg) = part.mux.flush_segment() {
            st.summary.bytes += seg.len() as u64;
            if let Err(e) = part.file.write_all(&seg) {
                st.error = Some(format!("写入失败：{e}"));
            }
        }
        let _ = part.file.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annexb_split() {
        let b = [0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 0, 1, 0x65, 9, 9, 0];
        let n = split_annexb(&b);
        assert_eq!(n, vec![&[0x67, 1, 2][..], &[0x68, 3][..], &[0x65, 9, 9][..]]);
    }

    #[test]
    fn part_names() {
        let p = Path::new("/tmp/a/rec.mp4");
        assert_eq!(part_path(p, 1), PathBuf::from("/tmp/a/rec.mp4"));
        assert_eq!(part_path(p, 3), PathBuf::from("/tmp/a/rec-3.mp4"));
    }

    /// 用 openh264 编一段合成画面加 Opus 声音录成 MP4，再用 mp4 读回来核对轨道与帧数；中途改分辨率应分成两个文件。
    #[test]
    fn records_playable_fragmented_mp4() {
        use openh264::encoder::{Encoder, EncoderConfig};
        use openh264::formats::YUVBuffer;
        let dir = std::env::temp_dir().join(format!("lg-rec-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rec = Recorder::new(&dir.join("rec.mp4"), true).unwrap();
        let mut opus = opus::Encoder::new(48_000, opus::Channels::Stereo, opus::Application::Audio).unwrap();
        let tone: Vec<f32> = (0..960 * 2).map(|i| ((i / 2) as f32 * 0.0575).sin() * 0.3).collect();
        let mut pkt = vec![0u8; 4000];
        // 编码器码率控制可能跳过个别帧（输出为空），按实际有数据的帧计数
        let mut per_part = vec![];
        for (w, h, frames) in [(320u32, 240u32, 30usize), (640, 360, 10)] {
            let mut got = 0u32;
            let mut enc = Encoder::with_api_config(openh264::OpenH264API::from_source(), EncoderConfig::new()).unwrap();
            rec.on_config(w, h);
            for i in 0..frames {
                // I420：亮度每帧不同，保证有实际内容
                let n = (w * h) as usize;
                let mut raw = vec![128u8; n + n / 2];
                for (k, v) in raw[..n].iter_mut().enumerate() {
                    *v = ((k + i * 7) % 255) as u8;
                }
                let yuv = YUVBuffer::from_vec(raw, w as usize, h as usize);
                if i == 0 {
                    enc.force_intra_frame();
                }
                let bs = enc.encode(&yuv).unwrap();
                let data = bs.to_vec();
                let kf = matches!(bs.frame_type(), openh264::encoder::FrameType::IDR | openh264::encoder::FrameType::I);
                if !data.is_empty() {
                    got += 1;
                }
                rec.on_video(&data, kf);
                let n = opus.encode_float(&tone, &mut pkt).unwrap();
                rec.on_audio(&pkt[..n]);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            per_part.push(got);
        }
        let s = rec.finish().unwrap();
        assert_eq!(s.paths.len(), 2, "{:?}", s.paths);
        assert_eq!(s.frames, per_part.iter().map(|x| *x as u64).sum::<u64>());
        assert!(per_part[0] >= 25 && per_part[1] >= 8, "{per_part:?}");
        for (p, want) in s.paths.iter().zip(per_part) {
            let f = std::fs::File::open(p).unwrap();
            let size = f.metadata().unwrap().len();
            let r = mp4::Mp4Reader::read_header(std::io::BufReader::new(f), size).unwrap();
            let kinds: Vec<_> = r.tracks().values().map(|t| t.track_type().unwrap()).collect();
            assert!(kinds.contains(&mp4::TrackType::Video) && kinds.contains(&mp4::TrackType::Audio), "{kinds:?}");
            let v = r.tracks().values().find(|t| t.track_type().unwrap() == mp4::TrackType::Video).unwrap();
            assert_eq!(v.sample_count(), want, "{}", p.display());
        }
        if std::env::var("LG_KEEP_REC").is_err() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
