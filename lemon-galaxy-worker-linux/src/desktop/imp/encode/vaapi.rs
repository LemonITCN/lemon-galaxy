//! VA-API 硬件编码（Intel 核显 / 独显、AMD，以及其他装了 VA 编码驱动的显卡）。运行时加载 libva.so.2 + libva-drm.so.2。
//!
//! 参数缓冲区按 libva 2.x 头文件的字段偏移逐个写（偏移、尺寸、位域取值都在 Ubuntu 24.04 的 libva 2.20 头文件上
//! 用 offsetof / sizeof 打印核对过，见下面各常量）。这些结构体是 libva 的 ABI，2.x 内不变。
//!
//! 用法：H.264 Main（CABAC），没有 Main 时 Constrained Baseline；CBR（不支持时 VBR）；一帧一个条带；
//! 只参考上一帧（两张重建面轮换），无 B 帧；按需 IDR（frame_num 归零、idr_pic_id 加一）；POC 类型 2。
//! SPS / PPS / 条带头由驱动生成（不传打包头）。编码面宽高按 16 对齐，SPS 里裁掉多出来的部分。
//! 画面在 CPU 上转 NV12（convert.rs）后经 vaDeriveImage（不行就 vaPutImage）传上去。

use super::convert::bgra_to_nv12;
use super::dl::Lib;
use super::{EncodedFrame, HwEncoder};
use std::ffi::{c_char, c_int, c_void};
use std::os::fd::{AsRawFd, OwnedFd};

type Dpy = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct ConfigAttrib {
    ty: c_int,
    value: u32,
}

crate::dl_fns! {
    struct Va {
        vaInitialize: fn(Dpy, *mut c_int, *mut c_int) -> c_int;
        vaTerminate: fn(Dpy) -> c_int;
        vaQueryVendorString: fn(Dpy) -> *const c_char;
        vaErrorStr: fn(c_int) -> *const c_char;
        vaMaxNumEntrypoints: fn(Dpy) -> c_int;
        vaQueryConfigEntrypoints: fn(Dpy, c_int, *mut c_int, *mut c_int) -> c_int;
        vaGetConfigAttributes: fn(Dpy, c_int, c_int, *mut ConfigAttrib, c_int) -> c_int;
        vaCreateConfig: fn(Dpy, c_int, c_int, *mut ConfigAttrib, c_int, *mut u32) -> c_int;
        vaDestroyConfig: fn(Dpy, u32) -> c_int;
        vaCreateSurfaces: fn(Dpy, u32, u32, u32, *mut u32, u32, *mut c_void, u32) -> c_int;
        vaDestroySurfaces: fn(Dpy, *mut u32, c_int) -> c_int;
        vaCreateContext: fn(Dpy, u32, c_int, c_int, c_int, *mut u32, c_int, *mut u32) -> c_int;
        vaDestroyContext: fn(Dpy, u32) -> c_int;
        vaCreateBuffer: fn(Dpy, u32, c_int, u32, u32, *mut c_void, *mut u32) -> c_int;
        vaDestroyBuffer: fn(Dpy, u32) -> c_int;
        vaMapBuffer: fn(Dpy, u32, *mut *mut c_void) -> c_int;
        vaUnmapBuffer: fn(Dpy, u32) -> c_int;
        vaBeginPicture: fn(Dpy, u32, u32) -> c_int;
        vaRenderPicture: fn(Dpy, u32, *mut u32, c_int) -> c_int;
        vaEndPicture: fn(Dpy, u32) -> c_int;
        vaSyncSurface: fn(Dpy, u32) -> c_int;
        vaDeriveImage: fn(Dpy, u32, *mut u8) -> c_int;
        vaCreateImage: fn(Dpy, *mut u8, c_int, c_int, *mut u8) -> c_int;
        vaPutImage: fn(Dpy, u32, u32, c_int, c_int, u32, u32, c_int, c_int, u32, u32) -> c_int;
        vaDestroyImage: fn(Dpy, u32) -> c_int;
    }
}

crate::dl_fns! {
    struct VaDrm {
        vaGetDisplayDRM: fn(c_int) -> Dpy;
    }
}

// ---------------- 常量（libva 2.20 头文件实测值） ----------------

const PROFILE_H264_CB: c_int = 13;
const PROFILE_H264_MAIN: c_int = 6;
const ENTRY_ENC_SLICE: c_int = 6;
const ENTRY_ENC_SLICE_LP: c_int = 8;
const ATTR_RT_FORMAT: c_int = 0;
const ATTR_RATE_CONTROL: c_int = 5;
const RT_YUV420: u32 = 1;
const RC_CBR: u32 = 2;
const RC_VBR: u32 = 4;
const ATTRIB_NOT_SUPPORTED: u32 = 0x8000_0000;
const BUF_CODED: c_int = 21;
const BUF_SEQ: c_int = 22;
const BUF_PIC: c_int = 23;
const BUF_SLICE: c_int = 24;
const BUF_MISC: c_int = 27;
const MISC_FRAMERATE: u32 = 0;
const MISC_RATE_CONTROL: u32 = 1;
const MISC_HRD: u32 = 5;
const FOURCC_NV12: u32 = 0x3231_564E;
const PIC_INVALID: u32 = 1;
const PIC_SHORT_TERM_REF: u32 = 8;
const PROGRESSIVE: c_int = 1;
const INVALID_ID: u32 = 0xffff_ffff;

// 结构体尺寸与偏移
const SEQ_SIZE: usize = 1132;
const PIC_SIZE: usize = 648;
const SLICE_SIZE: usize = 3140;
const PICTURE_H264_SIZE: usize = 36;
const IMAGE_SIZE: usize = 120;
const IMAGE_FORMAT_SIZE: usize = 48;
const RC_SIZE: usize = 60;
const FRAMERATE_SIZE: usize = 24;
const HRD_SIZE: usize = 24;

// 位域取值
const SEQ_CHROMA_420: u32 = 1;
const SEQ_FRAME_MBS_ONLY: u32 = 4;
const SEQ_DIRECT_8X8: u32 = 32;
const SEQ_LOG2_MAX_FRAME_NUM_MINUS4_UNIT: u32 = 64;
const SEQ_POC_TYPE_UNIT: u32 = 1024;
const SEQ_LOG2_MAX_POC_LSB_MINUS4_UNIT: u32 = 4096;
const PIC_IDR: u32 = 1;
const PIC_REFERENCE_UNIT: u32 = 2;
const PIC_CABAC: u32 = 8;
const PIC_DEBLOCK_PRESENT: u32 = 512;
const RC_FLAG_RESET: u32 = 1;
/// frame_num 用 8 位（log2_max_frame_num_minus4 = 4）
const MAX_FRAME_NUM: u32 = 256;

/// 按偏移写字段的小缓冲
struct Raw(Vec<u8>);
impl Raw {
    fn new(n: usize) -> Self {
        Self(vec![0; n])
    }
    fn u8(&mut self, o: usize, v: u8) {
        self.0[o] = v;
    }
    fn u16(&mut self, o: usize, v: u16) {
        self.0[o..o + 2].copy_from_slice(&v.to_ne_bytes());
    }
    fn u32(&mut self, o: usize, v: u32) {
        self.0[o..o + 4].copy_from_slice(&v.to_ne_bytes());
    }
    fn get_u32(&self, o: usize) -> u32 {
        u32::from_ne_bytes(self.0[o..o + 4].try_into().unwrap())
    }
    /// VAPictureH264：picture_id、frame_idx、flags、TopFieldOrderCnt、BottomFieldOrderCnt
    fn picture(&mut self, o: usize, surface: u32, frame_idx: u32, flags: u32, poc: i32) {
        self.u32(o, surface);
        self.u32(o + 4, frame_idx);
        self.u32(o + 8, flags);
        self.u32(o + 12, poc as u32);
        self.u32(o + 16, poc as u32);
    }
    fn invalid_pictures(&mut self, o: usize, n: usize) {
        for i in 0..n {
            self.picture(o + i * PICTURE_H264_SIZE, INVALID_ID, 0, PIC_INVALID, 0);
        }
    }
}

fn libs() -> Result<(&'static Va, &'static VaDrm), String> {
    use std::sync::OnceLock;
    static L: OnceLock<Result<(Va, VaDrm), String>> = OnceLock::new();
    let r = L.get_or_init(|| {
        let va: &'static Lib = Box::leak(Box::new(Lib::open(&["libva.so.2", "libva.so"])?));
        let drm: &'static Lib = Box::leak(Box::new(Lib::open(&["libva-drm.so.2", "libva-drm.so"])?));
        Ok((Va::load(va)?, VaDrm::load(drm)?))
    });
    match r {
        Ok((a, b)) => Ok((a, b)),
        Err(e) => Err(format!("没有 VA-API 运行库（{e}）")),
    }
}

pub struct Vaapi {
    va: &'static Va,
    dpy: Dpy,
    _fd: OwnedFd,
    config: u32,
    ctx: u32,
    /// 0 是输入面，1、2 是轮换的重建面
    surfaces: [u32; 3],
    coded: u32,
    width: u32,
    height: u32,
    aligned: (u32, u32),
    fps: u32,
    bps: u32,
    cabac: bool,
    rc_mode: u32,
    frame_num: u32,
    idr_id: u16,
    cur: usize,
    have_ref: bool,
    rate_dirty: bool,
    use_derive: bool,
    name: String,
}

// 只在采集线程里用
unsafe impl Send for Vaapi {}

fn err(va: &Va, st: c_int, what: &str) -> String {
    let s = unsafe {
        let p = (va.vaErrorStr)(st);
        if p.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    };
    format!("VA-API {what}失败（{st} {s}）")
}

fn check(va: &Va, st: c_int, what: &str) -> anyhow::Result<()> {
    if st == 0 {
        Ok(())
    } else {
        Err(anyhow::anyhow!(err(va, st, what)))
    }
}

/// 候选的渲染节点
fn render_nodes() -> Vec<String> {
    if let Ok(d) = std::env::var("LEMON_GALAXY_VAAPI_DEVICE") {
        return vec![d];
    }
    (128..136).map(|n| format!("/dev/dri/renderD{n}")).filter(|p| std::path::Path::new(p).exists()).collect()
}

impl Vaapi {
    pub fn new(width: u32, height: u32, fps: u32, kbps: u32) -> anyhow::Result<Self> {
        let (va, drm) = libs().map_err(anyhow::Error::msg)?;
        let nodes = render_nodes();
        anyhow::ensure!(!nodes.is_empty(), "没有显卡渲染节点（/dev/dri/renderD*）");
        let mut why = vec![];
        for node in nodes {
            match Self::open(va, drm, &node, width, height, fps, kbps) {
                Ok(e) => return Ok(e),
                Err(e) => why.push(format!("{node}：{e}")),
            }
        }
        anyhow::bail!("{}", why.join("；"))
    }

    fn open(va: &'static Va, drm: &'static VaDrm, node: &str, width: u32, height: u32, fps: u32, kbps: u32) -> anyhow::Result<Self> {
        let f = std::fs::OpenOptions::new().read(true).write(true).open(node).map_err(|e| anyhow::anyhow!("打不开（{e}）"))?;
        let fd: OwnedFd = f.into();
        unsafe {
            let dpy = (drm.vaGetDisplayDRM)(fd.as_raw_fd());
            anyhow::ensure!(!dpy.is_null(), "vaGetDisplayDRM 失败");
            let (mut maj, mut min) = (0, 0);
            let st = (va.vaInitialize)(dpy, &mut maj, &mut min);
            if st != 0 {
                anyhow::bail!(err(va, st, "初始化"));
            }
            let vendor = {
                let p = (va.vaQueryVendorString)(dpy);
                if p.is_null() {
                    String::new()
                } else {
                    std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
                }
            };
            let fail = |e: anyhow::Error| {
                (va.vaTerminate)(dpy);
                e
            };
            // 找支持编码的 H.264 档次与入口
            let n = (va.vaMaxNumEntrypoints)(dpy).max(1);
            let mut found = None;
            for profile in [PROFILE_H264_MAIN, PROFILE_H264_CB] {
                let mut eps = vec![0 as c_int; n as usize];
                let mut cnt = 0;
                if (va.vaQueryConfigEntrypoints)(dpy, profile, eps.as_mut_ptr(), &mut cnt) != 0 {
                    continue;
                }
                let eps = &eps[..cnt.max(0) as usize];
                if let Some(e) = [ENTRY_ENC_SLICE, ENTRY_ENC_SLICE_LP].into_iter().find(|e| eps.contains(e)) {
                    found = Some((profile, e));
                    break;
                }
            }
            let Some((profile, entry)) = found else { return Err(fail(anyhow::anyhow!("这块显卡（{vendor}）不支持 H.264 编码"))) };
            let mut attrs = [ConfigAttrib { ty: ATTR_RT_FORMAT, value: 0 }, ConfigAttrib { ty: ATTR_RATE_CONTROL, value: 0 }];
            if let Err(e) = check(va, (va.vaGetConfigAttributes)(dpy, profile, entry, attrs.as_mut_ptr(), 2), "查询属性") {
                return Err(fail(e));
            }
            if attrs[0].value == ATTRIB_NOT_SUPPORTED || attrs[0].value & RT_YUV420 == 0 {
                return Err(fail(anyhow::anyhow!("不支持 YUV 4:2:0")));
            }
            let rc_supported = if attrs[1].value == ATTRIB_NOT_SUPPORTED { 0 } else { attrs[1].value };
            let rc_mode = if rc_supported & RC_CBR != 0 {
                RC_CBR
            } else if rc_supported & RC_VBR != 0 {
                RC_VBR
            } else {
                return Err(fail(anyhow::anyhow!("不支持按码率控制（只有固定 QP）")));
            };
            let mut cfg_attrs = [ConfigAttrib { ty: ATTR_RT_FORMAT, value: RT_YUV420 }, ConfigAttrib { ty: ATTR_RATE_CONTROL, value: rc_mode }];
            let mut config = 0u32;
            if let Err(e) = check(va, (va.vaCreateConfig)(dpy, profile, entry, cfg_attrs.as_mut_ptr(), 2, &mut config), "建配置") {
                return Err(fail(e));
            }
            let aligned = ((width + 15) & !15, (height + 15) & !15);
            let mut surfaces = [0u32; 3];
            if let Err(e) = check(va, (va.vaCreateSurfaces)(dpy, RT_YUV420, aligned.0, aligned.1, surfaces.as_mut_ptr(), 3, std::ptr::null_mut(), 0), "建编码面") {
                (va.vaDestroyConfig)(dpy, config);
                return Err(fail(e));
            }
            let mut ctx = 0u32;
            if let Err(e) = check(va, (va.vaCreateContext)(dpy, config, aligned.0 as c_int, aligned.1 as c_int, PROGRESSIVE, surfaces.as_mut_ptr(), 3, &mut ctx), "建上下文") {
                (va.vaDestroySurfaces)(dpy, surfaces.as_mut_ptr(), 3);
                (va.vaDestroyConfig)(dpy, config);
                return Err(fail(e));
            }
            let coded_size = (aligned.0 * aligned.1 * 3 / 2).max(1 << 20);
            let mut coded = 0u32;
            if let Err(e) = check(va, (va.vaCreateBuffer)(dpy, ctx, BUF_CODED, coded_size, 1, std::ptr::null_mut(), &mut coded), "建输出缓冲") {
                (va.vaDestroyContext)(dpy, ctx);
                (va.vaDestroySurfaces)(dpy, surfaces.as_mut_ptr(), 3);
                (va.vaDestroyConfig)(dpy, config);
                return Err(fail(e));
            }
            let short = node.rsplit('/').next().unwrap_or(node);
            let vendor_short: String = vendor.chars().take(60).collect();
            Ok(Self {
                va,
                dpy,
                _fd: fd,
                config,
                ctx,
                surfaces,
                coded,
                width,
                height,
                aligned,
                fps: fps.max(1),
                bps: kbps.clamp(200, 500_000) * 1000,
                cabac: profile == PROFILE_H264_MAIN,
                rc_mode,
                frame_num: 0,
                idr_id: 0,
                cur: 1,
                have_ref: false,
                rate_dirty: false,
                use_derive: true,
                name: format!("VA-API H.264{}（{vendor_short}，{short}）", if entry == ENTRY_ENC_SLICE_LP { " 低功耗" } else { "" }),
            })
        }
    }

    fn level_idc(&self) -> u8 {
        let mbs = (self.aligned.0 / 16) * (self.aligned.1 / 16);
        let mbps = mbs * self.fps;
        // 4.1: 8192 宏块 / 245760 每秒；4.2: 8704 / 522240；5.1: 36864 / 983040；5.2: 36864 / 2073600
        if mbs <= 8192 && mbps <= 245_760 {
            41
        } else if mbs <= 8704 && mbps <= 522_240 {
            42
        } else if mbs <= 36_864 && mbps <= 983_040 {
            51
        } else {
            52
        }
    }

    fn buffer(&self, ty: c_int, data: &mut [u8]) -> anyhow::Result<u32> {
        let mut id = 0u32;
        check(self.va, unsafe { (self.va.vaCreateBuffer)(self.dpy, self.ctx, ty, data.len() as u32, 1, data.as_mut_ptr() as *mut c_void, &mut id) }, "建参数缓冲")?;
        Ok(id)
    }

    fn misc(&self, kind: u32, body: Raw) -> anyhow::Result<u32> {
        let mut b = Raw::new(4 + body.0.len());
        b.u32(0, kind);
        b.0[4..].copy_from_slice(&body.0);
        self.buffer(BUF_MISC, &mut b.0)
    }

    fn rate_buffers(&self, reset: bool) -> anyhow::Result<Vec<u32>> {
        let mut rc = Raw::new(RC_SIZE);
        rc.u32(0, self.bps);
        rc.u32(4, if self.rc_mode == RC_CBR { 100 } else { 90 });
        rc.u32(8, 1000);
        rc.u32(24, if reset { RC_FLAG_RESET } else { 0 });
        let mut fr = Raw::new(FRAMERATE_SIZE);
        fr.u32(0, self.fps);
        let vbv = self.bps / self.fps * 8;
        let mut hrd = Raw::new(HRD_SIZE);
        hrd.u32(0, vbv / 4 * 3);
        hrd.u32(4, vbv);
        Ok(vec![self.misc(MISC_RATE_CONTROL, rc)?, self.misc(MISC_FRAMERATE, fr)?, self.misc(MISC_HRD, hrd)?])
    }

    fn sequence(&self) -> anyhow::Result<u32> {
        let mut s = Raw::new(SEQ_SIZE);
        let (aw, ah) = self.aligned;
        s.u8(0, 0);
        s.u8(1, self.level_idc());
        // 关键帧只按需出：周期设得很长
        s.u32(4, self.fps * 3600);
        s.u32(8, self.fps * 3600);
        s.u32(12, 1);
        s.u32(16, self.bps);
        s.u32(20, 1);
        s.u16(24, (aw / 16) as u16);
        s.u16(26, (ah / 16) as u16);
        s.u32(28, SEQ_CHROMA_420 | SEQ_FRAME_MBS_ONLY | SEQ_DIRECT_8X8 | 4 * SEQ_LOG2_MAX_FRAME_NUM_MINUS4_UNIT | 2 * SEQ_POC_TYPE_UNIT | 4 * SEQ_LOG2_MAX_POC_LSB_MINUS4_UNIT);
        if (aw, ah) != (self.width, self.height) {
            // 4:2:0 帧编码时裁剪单位是 2 像素
            s.u8(1068, 1);
            s.u32(1076, (aw - self.width) / 2);
            s.u32(1084, (ah - self.height) / 2);
        }
        // VUI：只写时间信息（有的驱动码率控制要用）
        s.u8(1088, 1);
        s.u32(1092, 2);
        s.u32(1108, 1);
        s.u32(1112, self.fps * 2);
        let mut b = s.0;
        self.buffer(BUF_SEQ, &mut b)
    }

    fn upload(&mut self, bgra: &[u8], src_w: u32) -> anyhow::Result<()> {
        let (w, h) = (self.width as usize, self.height as usize);
        let va = self.va;
        unsafe {
            let mut img = Raw::new(IMAGE_SIZE);
            let st = if self.use_derive { (va.vaDeriveImage)(self.dpy, self.surfaces[0], img.0.as_mut_ptr()) } else { -1 };
            let derived = st == 0 && img.get_u32(4) == FOURCC_NV12;
            if !derived {
                // 派生出来的不是 NV12（或者驱动不支持派生）：以后都走 vaPutImage
                if st == 0 {
                    (va.vaDestroyImage)(self.dpy, img.get_u32(0));
                }
                self.use_derive = false;
                let mut fmt = Raw::new(IMAGE_FORMAT_SIZE);
                fmt.u32(0, FOURCC_NV12);
                fmt.u32(8, 12);
                img = Raw::new(IMAGE_SIZE);
                check(va, (va.vaCreateImage)(self.dpy, fmt.0.as_mut_ptr(), self.aligned.0 as c_int, self.aligned.1 as c_int, img.0.as_mut_ptr()), "建图像")?;
            }
            let (image_id, buf) = (img.get_u32(0), img.get_u32(52));
            let (p0, p1, o0, o1) = (img.get_u32(68) as usize, img.get_u32(72) as usize, img.get_u32(80) as usize, img.get_u32(84) as usize);
            let data_size = img.get_u32(60) as usize;
            let mut ptr: *mut c_void = std::ptr::null_mut();
            let st = (va.vaMapBuffer)(self.dpy, buf, &mut ptr);
            if st != 0 || ptr.is_null() || o1 + p1 * (h / 2) > data_size || o0 + p0 * h > data_size {
                (va.vaDestroyImage)(self.dpy, image_id);
                anyhow::bail!(err(va, st, "映射图像"));
            }
            let mem = std::slice::from_raw_parts_mut(ptr as *mut u8, data_size);
            let (y, uv) = mem.split_at_mut(o1);
            bgra_to_nv12(bgra, src_w as usize, w, h, &mut y[o0..], p0, uv, p1);
            (va.vaUnmapBuffer)(self.dpy, buf);
            let r = if derived { 0 } else { (va.vaPutImage)(self.dpy, self.surfaces[0], image_id, 0, 0, w as u32, h as u32, 0, 0, w as u32, h as u32) };
            (va.vaDestroyImage)(self.dpy, image_id);
            check(va, r, "上传画面")
        }
    }
}

impl HwEncoder for Vaapi {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn encode(&mut self, bgra: &[u8], src_w: u32, keyframe: bool, _pts_us: u64) -> anyhow::Result<Vec<EncodedFrame>> {
        anyhow::ensure!(src_w >= self.width && bgra.len() >= src_w as usize * self.height as usize * 4, "画面尺寸不对");
        self.upload(bgra, src_w)?;
        let key = keyframe || !self.have_ref;
        if key {
            self.frame_num = 0;
            self.idr_id = self.idr_id.wrapping_add(1);
        }
        let recon = self.surfaces[self.cur];
        let prev = self.surfaces[3 - self.cur];
        let prev_num = (self.frame_num + MAX_FRAME_NUM - 1) % MAX_FRAME_NUM;
        let mut bufs = vec![];
        let result = (|| -> anyhow::Result<Vec<u8>> {
            if key {
                bufs.push(self.sequence()?);
                bufs.extend(self.rate_buffers(false)?);
                self.rate_dirty = false;
            } else if self.rate_dirty {
                bufs.extend(self.rate_buffers(true)?);
                self.rate_dirty = false;
            }
            // 图像参数
            let mut p = Raw::new(PIC_SIZE);
            p.picture(0, recon, self.frame_num, 0, (self.frame_num * 2) as i32);
            p.invalid_pictures(36, 16);
            if !key {
                p.picture(36, prev, prev_num, PIC_SHORT_TERM_REF, (prev_num * 2) as i32);
            }
            p.u32(612, self.coded);
            p.u16(620, self.frame_num as u16);
            p.u8(622, 26);
            p.u32(628, if key { PIC_IDR } else { 0 } | PIC_REFERENCE_UNIT | if self.cabac { PIC_CABAC } else { 0 } | PIC_DEBLOCK_PRESENT);
            bufs.push(self.buffer(BUF_PIC, &mut p.0)?);
            // 条带参数：一帧一个条带
            let mut s = Raw::new(SLICE_SIZE);
            let mbs = (self.aligned.0 / 16) * (self.aligned.1 / 16);
            s.u32(4, mbs);
            s.u32(8, INVALID_ID);
            s.u8(12, if key { 2 } else { 0 });
            s.u16(14, self.idr_id);
            s.u16(16, ((self.frame_num * 2) % 256) as u16);
            s.invalid_pictures(36, 32);
            s.invalid_pictures(1188, 32);
            if !key {
                s.picture(36, prev, prev_num, PIC_SHORT_TERM_REF, (prev_num * 2) as i32);
            }
            bufs.push(self.buffer(BUF_SLICE, &mut s.0)?);
            let va = self.va;
            unsafe {
                check(va, (va.vaBeginPicture)(self.dpy, self.ctx, self.surfaces[0]), "开始编码")?;
                check(va, (va.vaRenderPicture)(self.dpy, self.ctx, bufs.as_mut_ptr(), bufs.len() as c_int), "提交参数")?;
                check(va, (va.vaEndPicture)(self.dpy, self.ctx), "结束编码")?;
                check(va, (va.vaSyncSurface)(self.dpy, self.surfaces[0]), "等编码完成")?;
                let mut seg: *mut c_void = std::ptr::null_mut();
                check(va, (va.vaMapBuffer)(self.dpy, self.coded, &mut seg), "取输出")?;
                let mut out = Vec::new();
                // VACodedBufferSegment：size@0、buf@16、next@24
                let mut cur = seg as *const u8;
                let mut guard = 0;
                while !cur.is_null() && guard < 64 {
                    let size = u32::from_ne_bytes(std::slice::from_raw_parts(cur, 4).try_into().unwrap()) as usize;
                    let data = *(cur.add(16) as *const *const u8);
                    if !data.is_null() && size > 0 {
                        out.extend_from_slice(std::slice::from_raw_parts(data, size));
                    }
                    cur = *(cur.add(24) as *const *const u8);
                    guard += 1;
                }
                (va.vaUnmapBuffer)(self.dpy, self.coded);
                Ok(out)
            }
        })();
        for b in bufs {
            unsafe { (self.va.vaDestroyBuffer)(self.dpy, b) };
        }
        let data = result?;
        self.have_ref = true;
        self.cur = 3 - self.cur;
        self.frame_num = (self.frame_num + 1) % MAX_FRAME_NUM;
        if data.is_empty() {
            return Ok(vec![]);
        }
        Ok(vec![EncodedFrame { data, keyframe: key }])
    }

    fn set_bitrate(&mut self, kbps: u32) -> bool {
        self.bps = kbps.clamp(200, 500_000) * 1000;
        self.rate_dirty = true;
        true
    }
}

impl Drop for Vaapi {
    fn drop(&mut self) {
        unsafe {
            (self.va.vaDestroyBuffer)(self.dpy, self.coded);
            (self.va.vaDestroyContext)(self.dpy, self.ctx);
            (self.va.vaDestroySurfaces)(self.dpy, self.surfaces.as_mut_ptr(), 3);
            (self.va.vaDestroyConfig)(self.dpy, self.config);
            (self.va.vaTerminate)(self.dpy);
        }
    }
}
