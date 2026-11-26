//! Media Foundation 硬件 H.264 编码器（Intel Quick Sync、AMD AMF、高通等显卡厂商提供的异步 MFT）。
//! 输入是显卡上的 NV12 纹理（经 DXGI 设备管理器交给编码器），低延迟模式、CBR、无 B 帧，
//! 码率用 CODECAPI_AVEncCommonMeanBitRate 运行时修改，关键帧用 CODECAPI_AVEncVideoForceKeyFrame。
//!
//! mfplat.dll 的函数全部运行时加载：Windows N 版没装媒体功能包、服务器核心版没有它，静态导入的话程序都起不来。
//! 异步 MFT 的事件在单独线程里阻塞取，转给编码调用方；ProcessInput / ProcessOutput 都在调用方线程。

use super::{has_parameter_sets, strip_aud, EncodedFrame, EncoderRate, HwEncoder};
use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use windows::core::{Interface, BOOL, GUID, HRESULT, PCSTR, PCWSTR, PWSTR};
use windows::Win32::Foundation::{LUID, VARIANT_BOOL};
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D};
use windows::Win32::Media::MediaFoundation::{
    eAVEncCommonRateControlMode_CBR, eAVEncH264VProfile_Main, ICodecAPI, IMF2DBuffer, IMFActivate, IMFAttributes, IMFDXGIDeviceManager, IMFMediaBuffer, IMFMediaEventGenerator, IMFMediaType, IMFSample, IMFShutdown,
    IMFTransform, CODECAPI_AVEncCommonMeanBitRate, CODECAPI_AVEncCommonQualityVsSpeed, CODECAPI_AVEncCommonRateControlMode, CODECAPI_AVEncMPVDefaultBPictureCount, CODECAPI_AVEncMPVGOPSize,
    CODECAPI_AVEncVideoForceKeyFrame, CODECAPI_AVLowLatencyMode, MEError, METransformHaveOutput, METransformNeedInput, MFMediaType_Video, MFSampleExtension_CleanPoint, MFT_CATEGORY_VIDEO_ENCODER,
    MFT_ENUM_ADAPTER_LUID, MFT_ENUM_FLAG_HARDWARE, MFT_ENUM_FLAG_SORTANDFILTER, MFT_ENUM_HARDWARE_VENDOR_ID_Attribute, MFT_FRIENDLY_NAME_Attribute, MFT_MESSAGE_COMMAND_FLUSH, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
    MFT_MESSAGE_NOTIFY_END_OF_STREAM, MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_MESSAGE_SET_D3D_MANAGER, MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES,
    MFT_REGISTER_TYPE_INFO, MFVideoFormat_H264, MFVideoFormat_NV12, MFVideoInterlace_Progressive, MF_E_NOTACCEPTING, MF_E_TRANSFORM_STREAM_CHANGE, MF_LOW_LATENCY, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE,
    MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_MPEG2_PROFILE, MF_MT_MPEG_SEQUENCE_HEADER, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_TRANSFORM_ASYNC_UNLOCK, MF_VERSION,
};
use super::super::compat::co_task_mem_free;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::Win32::System::Variant::{VARIANT, VT_BOOL, VT_UI4};

// ---------------- mfplat.dll（运行时加载） ----------------

type Out = *mut *mut c_void;
struct MfApi {
    create_dxgi_device_manager: unsafe extern "system" fn(*mut u32, Out) -> HRESULT,
    tenum_ex: unsafe extern "system" fn(GUID, u32, *const MFT_REGISTER_TYPE_INFO, *const MFT_REGISTER_TYPE_INFO, *mut *mut *mut c_void, *mut u32) -> HRESULT,
    tenum2: Option<unsafe extern "system" fn(GUID, u32, *const MFT_REGISTER_TYPE_INFO, *const MFT_REGISTER_TYPE_INFO, *mut c_void, *mut *mut *mut c_void, *mut u32) -> HRESULT>,
    create_media_type: unsafe extern "system" fn(Out) -> HRESULT,
    create_sample: unsafe extern "system" fn(Out) -> HRESULT,
    create_dxgi_surface_buffer: unsafe extern "system" fn(*const GUID, *mut c_void, u32, BOOL, Out) -> HRESULT,
    create_memory_buffer: unsafe extern "system" fn(u32, Out) -> HRESULT,
    create_attributes: unsafe extern "system" fn(Out, u32) -> HRESULT,
}

fn api() -> Option<&'static MfApi> {
    static API: OnceLock<Option<MfApi>> = OnceLock::new();
    API.get_or_init(|| unsafe {
        let mut buf = [0u16; 260];
        let n = GetSystemDirectoryW(Some(&mut buf)) as usize;
        if n == 0 || n >= buf.len() {
            return None;
        }
        let path: Vec<u16> = String::from_utf16_lossy(&buf[..n]).encode_utf16().chain("\\mfplat.dll".encode_utf16()).chain(Some(0)).collect();
        let m = LoadLibraryW(PCWSTR(path.as_ptr())).ok()?;
        let p = |name: &[u8]| GetProcAddress(m, PCSTR(name.as_ptr()));
        let startup: unsafe extern "system" fn(u32, u32) -> HRESULT = std::mem::transmute(p(b"MFStartup\0")?);
        let api = MfApi {
            create_dxgi_device_manager: std::mem::transmute(p(b"MFCreateDXGIDeviceManager\0")?),
            tenum_ex: std::mem::transmute(p(b"MFTEnumEx\0")?),
            tenum2: p(b"MFTEnum2\0").map(|f| std::mem::transmute(f)),
            create_media_type: std::mem::transmute(p(b"MFCreateMediaType\0")?),
            create_sample: std::mem::transmute(p(b"MFCreateSample\0")?),
            create_dxgi_surface_buffer: std::mem::transmute(p(b"MFCreateDXGISurfaceBuffer\0")?),
            create_memory_buffer: std::mem::transmute(p(b"MFCreateMemoryBuffer\0")?),
            create_attributes: std::mem::transmute(p(b"MFCreateAttributes\0")?),
        };
        // MFSTARTUP_LITE：不需要网络。进程里只启动一次，不关
        if startup(MF_VERSION, 1).is_err() {
            return None;
        }
        Some(api)
    })
    .as_ref()
}

/// Media Foundation 能用吗（Windows N 版没装媒体功能包时不能）。
pub fn available() -> bool {
    api().is_some()
}

unsafe fn made<T: Interface>(hr: HRESULT, p: *mut c_void) -> anyhow::Result<T> {
    hr.ok()?;
    anyhow::ensure!(!p.is_null(), "Media Foundation 返回空对象");
    Ok(T::from_raw(p))
}

fn variant_u32(v: u32) -> VARIANT {
    let mut x = VARIANT::default();
    unsafe {
        (*x.Anonymous.Anonymous).vt = VT_UI4;
        (*x.Anonymous.Anonymous).Anonymous.ulVal = v;
    }
    x
}

fn variant_bool(v: bool) -> VARIANT {
    let mut x = VARIANT::default();
    unsafe {
        (*x.Anonymous.Anonymous).vt = VT_BOOL;
        (*x.Anonymous.Anonymous).Anonymous.boolVal = VARIANT_BOOL(if v { -1 } else { 0 });
    }
    x
}

// ---------------- 找编码器 ----------------

/// 一个硬件编码器的候选。
struct Candidate {
    activate: IMFActivate,
    name: String,
    vendor: String,
}

fn attr_string(a: &IMFAttributes, key: &GUID) -> String {
    unsafe {
        let mut p = PWSTR::null();
        let mut n = 0u32;
        if a.GetAllocatedString(key, &mut p, &mut n).is_err() || p.is_null() {
            return String::new();
        }
        let s = p.to_string().unwrap_or_default();
        co_task_mem_free(p.0 as _);
        s
    }
}

/// 这块显卡（按 LUID）上的硬件 H.264 编码器；系统太旧不能按显卡筛时列出全部，再按厂商号筛。
fn find(luid: &LUID, vendor: u32) -> anyhow::Result<Vec<Candidate>> {
    let mf = api().ok_or_else(|| anyhow::anyhow!("系统没有 Media Foundation（Windows N 版需要装媒体功能包）"))?;
    let input = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: MFVideoFormat_NV12 };
    let output = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: MFVideoFormat_H264 };
    let flags = (MFT_ENUM_FLAG_HARDWARE.0 | MFT_ENUM_FLAG_SORTANDFILTER.0) as u32;
    let mut list: *mut *mut c_void = std::ptr::null_mut();
    let mut count = 0u32;
    let mut by_luid = false;
    unsafe {
        if let Some(e2) = mf.tenum2 {
            let mut p = std::ptr::null_mut();
            if let Ok(attrs) = made::<IMFAttributes>((mf.create_attributes)(&mut p, 1), p) {
                let bytes = std::slice::from_raw_parts(luid as *const LUID as *const u8, std::mem::size_of::<LUID>());
                if attrs.SetBlob(&MFT_ENUM_ADAPTER_LUID, bytes).is_ok() && e2(MFT_CATEGORY_VIDEO_ENCODER, flags, &input, &output, attrs.as_raw(), &mut list, &mut count).is_ok() {
                    by_luid = true;
                }
            }
        }
        if !by_luid {
            (mf.tenum_ex)(MFT_CATEGORY_VIDEO_ENCODER, flags, &input, &output, &mut list, &mut count).ok()?;
        }
        let mut v = Vec::new();
        let want = format!("VEN_{vendor:04X}");
        for i in 0..count as usize {
            let raw = *list.add(i);
            if raw.is_null() {
                continue;
            }
            let a = IMFActivate::from_raw(raw);
            let attrs: IMFAttributes = a.cast()?;
            let name = attr_string(&attrs, &MFT_FRIENDLY_NAME_Attribute);
            let vendor_id = attr_string(&attrs, &MFT_ENUM_HARDWARE_VENDOR_ID_Attribute);
            if !by_luid && !vendor_id.to_ascii_uppercase().contains(&want) {
                continue;
            }
            v.push(Candidate { activate: a, name, vendor: vendor_id });
        }
        if !list.is_null() {
            co_task_mem_free(list as _);
        }
        Ok(v)
    }
}

// ---------------- 编码器 ----------------

enum Ev {
    NeedInput,
    HaveOutput,
    Error(HRESULT),
    Closed,
}

/// 事件线程拿着的生成器（异步 MFT 是自由线程的）
struct SendGen(IMFMediaEventGenerator);
unsafe impl Send for SendGen {}

pub struct Mft {
    transform: IMFTransform,
    codec: Option<ICodecAPI>,
    _manager: IMFDXGIDeviceManager,
    events: Receiver<Ev>,
    in_id: u32,
    out_id: u32,
    provides_samples: bool,
    out_size: u32,
    need_input: u32,
    /// 已经送进去还没出来的帧数
    pending: u32,
    key_pending: bool,
    seq_header: Vec<u8>,
    fps: u32,
    last_time: i64,
    /// 送帧后等输出的时长
    wait_ms: u64,
    name: String,
}

// 只在采集线程里用（事件在另一个线程取）
unsafe impl Send for Mft {}

impl Mft {
    /// 在 `device` 所在显卡上找硬件 H.264 编码器。`allow_nvidia`：NVIDIA 默认走 NVENC 直连，这里跳过它的 MFT。
    pub fn new(device: &ID3D11Device, adapter: &super::gpu::Adapter, width: u32, height: u32, fps: u32, kbps: u32, allow_nvidia: bool) -> anyhow::Result<Self> {
        let list = find(&adapter.luid, adapter.vendor)?;
        anyhow::ensure!(!list.is_empty(), "这块显卡没有硬件 H.264 编码器");
        let mut errors = Vec::new();
        for c in list {
            if !allow_nvidia && c.vendor.to_ascii_uppercase().contains("VEN_10DE") {
                errors.push(format!("{}：NVIDIA 显卡走 NVENC 直连", c.name));
                continue;
            }
            match Self::open(&c, device, width, height, fps, kbps) {
                Ok(mut m) => {
                    m.name = format!("Media Foundation {}（{}）", c.name, adapter.name);
                    return Ok(m);
                }
                Err(e) => errors.push(format!("{}：{e}", c.name)),
            }
            unsafe {
                let _ = c.activate.ShutdownObject();
            }
        }
        anyhow::bail!("{}", errors.join("；"))
    }

    fn open(c: &Candidate, device: &ID3D11Device, width: u32, height: u32, fps: u32, kbps: u32) -> anyhow::Result<Self> {
        let mf = api().ok_or_else(|| anyhow::anyhow!("没有 Media Foundation"))?;
        let fps = fps.max(1);
        unsafe {
            let transform: IMFTransform = c.activate.ActivateObject()?;
            let attrs = transform.GetAttributes()?;
            // 硬件编码器是异步的，先解锁才能用
            attrs.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)?;
            let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);
            let gen: IMFMediaEventGenerator = transform.cast().map_err(|_| anyhow::anyhow!("不是异步编码器"))?;

            let mut token = 0u32;
            let mut p = std::ptr::null_mut();
            let manager: IMFDXGIDeviceManager = made((mf.create_dxgi_device_manager)(&mut token, &mut p), p)?;
            manager.ResetDevice(device, token)?;
            transform.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize).map_err(|e| anyhow::anyhow!("编码器不收显卡上的画面：{e}"))?;

            let (mut in_id, mut out_id) = ([0u32], [0u32]);
            if transform.GetStreamIDs(&mut in_id, &mut out_id).is_err() {
                in_id = [0];
                out_id = [0];
            }
            let codec: Option<ICodecAPI> = transform.cast().ok();
            let bps = kbps.clamp(200, 500_000) * 1000;
            if let Some(api) = &codec {
                let _ = api.SetValue(&CODECAPI_AVLowLatencyMode, &variant_bool(true));
                let _ = api.SetValue(&CODECAPI_AVEncCommonRateControlMode, &variant_u32(eAVEncCommonRateControlMode_CBR.0 as u32));
                let _ = api.SetValue(&CODECAPI_AVEncCommonMeanBitRate, &variant_u32(bps));
                let _ = api.SetValue(&CODECAPI_AVEncMPVDefaultBPictureCount, &variant_u32(0));
                // 关键帧只在需要时出：GOP 放到一小时
                let _ = api.SetValue(&CODECAPI_AVEncMPVGOPSize, &variant_u32(fps * 3600));
                let _ = api.SetValue(&CODECAPI_AVEncCommonQualityVsSpeed, &variant_u32(0));
            }

            // 先设输出格式，编码器据此给出可用的输入格式
            let mut p = std::ptr::null_mut();
            let ot: IMFMediaType = made((mf.create_media_type)(&mut p), p)?;
            ot.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            ot.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
            ot.SetUINT32(&MF_MT_AVG_BITRATE, bps)?;
            ot.SetUINT64(&MF_MT_FRAME_SIZE, ((width as u64) << 32) | height as u64)?;
            ot.SetUINT64(&MF_MT_FRAME_RATE, ((fps as u64) << 32) | 1)?;
            ot.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
            ot.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            ot.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_Main.0 as u32)?;
            transform.SetOutputType(out_id[0], &ot, 0).map_err(|e| anyhow::anyhow!("编码器不接受输出格式：{e}"))?;

            let mut it: Option<IMFMediaType> = None;
            let mut i = 0;
            while let Ok(t) = transform.GetInputAvailableType(in_id[0], i) {
                i += 1;
                if t.GetGUID(&MF_MT_SUBTYPE).map(|g| g == MFVideoFormat_NV12).unwrap_or(false) {
                    it = Some(t);
                    break;
                }
            }
            let it = match it {
                Some(t) => t,
                None => {
                    let mut p = std::ptr::null_mut();
                    let t: IMFMediaType = made((mf.create_media_type)(&mut p), p)?;
                    t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                    t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
                    t
                }
            };
            it.SetUINT64(&MF_MT_FRAME_SIZE, ((width as u64) << 32) | height as u64)?;
            it.SetUINT64(&MF_MT_FRAME_RATE, ((fps as u64) << 32) | 1)?;
            it.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
            it.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            transform.SetInputType(in_id[0], &it, 0).map_err(|e| anyhow::anyhow!("编码器不接受 NV12 输入：{e}"))?;

            let info = transform.GetOutputStreamInfo(out_id[0])?;
            let provides_samples = info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0) as u32 != 0;

            let (tx, rx) = mpsc::channel();
            let g = SendGen(gen);
            std::thread::Builder::new().name("lg-mft-events".into()).spawn(move || {
                let g = g;
                loop {
                    let ev = match g.0.GetEvent(Default::default()) {
                        Ok(e) => match e.GetType() {
                            Ok(t) if t == METransformNeedInput.0 as u32 => Ev::NeedInput,
                            Ok(t) if t == METransformHaveOutput.0 as u32 => Ev::HaveOutput,
                            // Intel 的编码器开头会发一个状态为成功的 MEError，不算错
                            Ok(t) if t == MEError.0 as u32 => match e.GetStatus() {
                                Ok(hr) if hr.is_ok() => continue,
                                Ok(hr) => Ev::Error(hr),
                                Err(err) => Ev::Error(err.code()),
                            },
                            _ => continue,
                        },
                        Err(_) => Ev::Closed,
                    };
                    let closed = matches!(ev, Ev::Closed);
                    if tx.send(ev).is_err() || closed {
                        break;
                    }
                }
            })?;

            transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0)?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
            let mut me = Self {
                transform,
                codec,
                _manager: manager,
                events: rx,
                in_id: in_id[0],
                out_id: out_id[0],
                provides_samples,
                out_size: info.cbSize,
                need_input: 0,
                pending: 0,
                key_pending: false,
                seq_header: Vec::new(),
                fps,
                last_time: -1,
                wait_ms: 100,
                name: String::new(),
            };
            me.read_sequence_header();
            Ok(me)
        }
    }

    fn read_sequence_header(&mut self) {
        unsafe {
            let Ok(t) = self.transform.GetOutputCurrentType(self.out_id) else { return };
            let Ok(n) = t.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER) else { return };
            let mut v = vec![0u8; n as usize];
            if n > 0 && t.GetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut v, None).is_ok() {
                self.seq_header = v;
            }
        }
    }

    /// 取一帧输出。
    fn output(&mut self) -> anyhow::Result<Option<EncodedFrame>> {
        let mf = api().ok_or_else(|| anyhow::anyhow!("没有 Media Foundation"))?;
        unsafe {
            let mut mine: Option<IMFSample> = None;
            if !self.provides_samples {
                let mut p = std::ptr::null_mut();
                let buf: IMFMediaBuffer = made((mf.create_memory_buffer)(self.out_size.max(4 << 20), &mut p), p)?;
                let mut p = std::ptr::null_mut();
                let s: IMFSample = made((mf.create_sample)(&mut p), p)?;
                s.AddBuffer(&buf)?;
                mine = Some(s);
            }
            let mut ob = [MFT_OUTPUT_DATA_BUFFER { dwStreamID: self.out_id, pSample: ManuallyDrop::new(mine.clone()), dwStatus: 0, pEvents: ManuallyDrop::new(None) }];
            let mut status = 0u32;
            let r = self.transform.ProcessOutput(0, &mut ob, &mut status);
            let [mut ob] = ob;
            let sample = ManuallyDrop::take(&mut ob.pSample);
            drop(ManuallyDrop::take(&mut ob.pEvents));
            if let Err(e) = r {
                if e.code() == MF_E_TRANSFORM_STREAM_CHANGE {
                    // 编码器定下了输出格式：照它的来
                    if let Ok(t) = self.transform.GetOutputAvailableType(self.out_id, 0) {
                        let _ = self.transform.SetOutputType(self.out_id, &t, 0);
                    }
                    self.read_sequence_header();
                    return Ok(None);
                }
                return Err(anyhow::anyhow!("编码器出帧失败：{e}"));
            }
            let Some(sample) = sample else { return Ok(None) };
            self.pending = self.pending.saturating_sub(1);
            let buf = sample.ConvertToContiguousBuffer()?;
            let mut ptr = std::ptr::null_mut();
            let mut len = 0u32;
            buf.Lock(&mut ptr, None, Some(&mut len))?;
            let mut data = std::slice::from_raw_parts(ptr, len as usize).to_vec();
            let _ = buf.Unlock();
            strip_aud(&mut data);
            let key = sample.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(0) != 0;
            if key && !has_parameter_sets(&data) && !self.seq_header.is_empty() {
                let mut v = self.seq_header.clone();
                v.extend_from_slice(&data);
                data = v;
            }
            if data.is_empty() {
                return Ok(None);
            }
            Ok(Some(EncodedFrame { keyframe: key && has_parameter_sets(&data), data }))
        }
    }

    /// 处理一个事件；出了帧放进 `out`。
    fn handle(&mut self, ev: Ev, out: &mut Vec<EncodedFrame>) -> anyhow::Result<()> {
        match ev {
            Ev::NeedInput => self.need_input += 1,
            Ev::HaveOutput => {
                if let Some(f) = self.output()? {
                    out.push(f);
                }
            }
            Ev::Error(hr) => anyhow::bail!("编码器出错：{hr:?}"),
            Ev::Closed => anyhow::bail!("编码器已关闭"),
        }
        Ok(())
    }

    /// 最多等到 `until`，直到 `done` 成立。
    fn wait(&mut self, until: Instant, out: &mut Vec<EncodedFrame>, done: impl Fn(&Self, &[EncodedFrame]) -> bool) -> anyhow::Result<bool> {
        loop {
            if done(self, out) {
                return Ok(true);
            }
            let left = until.saturating_duration_since(Instant::now());
            match self.events.recv_timeout(left) {
                Ok(ev) => self.handle(ev, out)?,
                Err(RecvTimeoutError::Timeout) => return Ok(done(self, out)),
                Err(RecvTimeoutError::Disconnected) => anyhow::bail!("编码器已关闭"),
            }
        }
    }
}

impl HwEncoder for Mft {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn encode(&mut self, nv12: &ID3D11Texture2D, keyframe: bool, pts_us: u64) -> anyhow::Result<Vec<EncodedFrame>> {
        let mf = api().ok_or_else(|| anyhow::anyhow!("没有 Media Foundation"))?;
        let mut out = Vec::new();
        self.key_pending |= keyframe;
        // 编码器还没要输入：稍等；等不到就丢掉这一帧（关键帧请求留给下一帧）
        while let Ok(ev) = self.events.try_recv() {
            self.handle(ev, &mut out)?;
        }
        if !self.wait(Instant::now() + Duration::from_millis(100), &mut out, |m, _| m.need_input > 0)? {
            return Ok(out);
        }
        unsafe {
            let mut p = std::ptr::null_mut();
            let buf: IMFMediaBuffer = made((mf.create_dxgi_surface_buffer)(&ID3D11Texture2D::IID, nv12.as_raw(), 0, false.into(), &mut p), p)?;
            if let Ok(b2) = buf.cast::<IMF2DBuffer>() {
                if let Ok(n) = b2.GetContiguousLength() {
                    let _ = buf.SetCurrentLength(n);
                }
            }
            let mut p = std::ptr::null_mut();
            let sample: IMFSample = made((mf.create_sample)(&mut p), p)?;
            sample.AddBuffer(&buf)?;
            // 时间戳单位 100 ns，必须递增
            let t = (pts_us as i64 * 10).max(self.last_time + 1);
            self.last_time = t;
            sample.SetSampleTime(t)?;
            sample.SetSampleDuration(10_000_000 / self.fps as i64)?;
            if self.key_pending {
                if let Some(api) = &self.codec {
                    let _ = api.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &variant_u32(1));
                }
            }
            if let Err(e) = self.transform.ProcessInput(self.in_id, &sample, 0) {
                if e.code() == MF_E_NOTACCEPTING {
                    // 编码器手上满了：这一帧不送，等它要
                    self.need_input = 0;
                    return Ok(out);
                }
                anyhow::bail!("送帧失败：{e}");
            }
        }
        self.key_pending = false;
        self.need_input -= 1;
        self.pending += 1;
        // 低延迟模式下一帧进一帧出；等这一帧出来（最多 100 ms，没出来的下次再取）
        self.wait(Instant::now() + Duration::from_millis(self.wait_ms), &mut out, |m, _| m.pending == 0)?;
        while let Ok(ev) = self.events.try_recv() {
            self.handle(ev, &mut out)?;
        }
        Ok(out)
    }

    fn probe(&mut self, nv12: &ID3D11Texture2D) -> anyhow::Result<()> {
        // 有的编码器建得起来却编不出帧（例如本机 Intel 核显没接显示器时，收一帧后报错）：先试编一帧
        self.wait_ms = 1000;
        let r = self.encode(nv12, true, 0);
        self.wait_ms = 100;
        anyhow::ensure!(!r?.is_empty(), "试编码一秒内没有出帧");
        Ok(())
    }

    /// 只改平均码率（Media Foundation 编码器开始编码后不一定允许改 VBV）
    fn set_bitrate(&mut self, rate: EncoderRate) -> bool {
        let Some(api) = &self.codec else { return false };
        unsafe { api.SetValue(&CODECAPI_AVEncCommonMeanBitRate, &variant_u32(rate.kbps.clamp(200, 500_000) * 1000)).is_ok() }
    }
}

impl Drop for Mft {
    fn drop(&mut self) {
        unsafe {
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
            // 关掉后事件线程的 GetEvent 返回错误，线程自己退出
            if let Ok(s) = self.transform.cast::<IMFShutdown>() {
                let _ = s.Shutdown();
            }
        }
    }
}
