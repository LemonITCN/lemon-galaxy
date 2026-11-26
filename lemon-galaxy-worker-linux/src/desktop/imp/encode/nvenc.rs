//! NVIDIA NVENC（Linux）：运行时加载驱动自带的 libnvidia-encode.so.1 与 libcuda.so.1（没装 NVIDIA 驱动时加载失败，不影响启动）。
//! GeForce（如 RTX 3080）、Quadro、数据中心卡都支持；NVIDIA 驱动对 GeForce 限制同时编码路数（新驱动 8 路），超了会建会话失败、退回下一个编码器。
//!
//! 结构体与常量和 Windows 被控端（lemon-galaxy-worker-windows/src/win/encode/nvenc.rs）是同一份，按 Video Codec SDK 12.0 的
//! nvEncodeAPI.h 手写、编译期断言核对过尺寸与偏移（x86_64 与 ARM64 的 Linux 布局相同：只有 32 位整数、64 位整数与指针）。
//!
//! 输入：CUDA 显存（cuMemAllocPitch）登记成 NVENC 资源，每帧把内存里的 BGRA 拷上去（cuMemcpy2D），格式 ARGB（字序 A8R8G8B8，
//! 内存里就是 B G R A），显卡自己转 YUV。用法与 Windows 相同：同步编码、超低延迟、无 B 帧、按需 IDR、码率运行时改。

#![allow(dead_code)]

use super::{EncodedFrame, EncoderRate, HwEncoder};
use super::dl::Lib;
use std::ffi::{c_char, c_void};
use std::mem::{offset_of, size_of};

/// 与 Windows 的 GUID 同布局
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct GUID {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

impl GUID {
    pub const fn from_u128(v: u128) -> Self {
        Self { data1: (v >> 96) as u32, data2: (v >> 80 & 0xffff) as u16, data3: (v >> 64 & 0xffff) as u16, data4: (v as u64).to_be_bytes() }
    }
}
const _: () = assert!(size_of::<GUID>() == 16);

const DEVICE_TYPE_CUDA: u32 = 1;
const RESOURCE_TYPE_CUDADEVICEPTR: u32 = 1;
const BUFFER_FORMAT_ARGB: u32 = 0x0100_0000;

// ---------------- 常量 ----------------

const API_MAJOR: u32 = 12;
const API_MINOR: u32 = 0;
const API_VERSION: u32 = API_MAJOR | (API_MINOR << 24);
const fn ver(v: u32) -> u32 {
    API_VERSION | (v << 16) | (0x7 << 28)
}
const fn ver_ext(v: u32) -> u32 {
    ver(v) | (1 << 31)
}
const FUNCTION_LIST_VER: u32 = ver(2);
const OPEN_SESSION_VER: u32 = ver(1);
const PRESET_CONFIG_VER: u32 = ver_ext(4);
const CONFIG_VER: u32 = ver_ext(8);
const INIT_PARAMS_VER: u32 = ver_ext(5);
const RECONFIGURE_VER: u32 = ver_ext(1);
const PIC_PARAMS_VER: u32 = ver_ext(6);
const LOCK_BITSTREAM_VER: u32 = ver(2);
const MAP_INPUT_VER: u32 = ver(4);
const REGISTER_VER: u32 = ver(4);
const BITSTREAM_BUFFER_VER: u32 = ver(1);
// 与头文件宏展开的值核对
const _: () = assert!(FUNCTION_LIST_VER == 1879179276 && CONFIG_VER == (-267911156i32) as u32 && INIT_PARAMS_VER == (-268107764i32) as u32);
const _: () = assert!(PIC_PARAMS_VER == (-268042228i32) as u32 && PRESET_CONFIG_VER == (-268173300i32) as u32 && RECONFIGURE_VER == (-268369908i32) as u32);
const _: () = assert!(LOCK_BITSTREAM_VER == 1879179276 && MAP_INPUT_VER == 1879310348 && REGISTER_VER == 1879310348 && OPEN_SESSION_VER == 1879113740 && BITSTREAM_BUFFER_VER == 1879113740);

const CODEC_H264: GUID = GUID::from_u128(0x6bc82762_4e63_4ca4_aa85_1e50f321f6bf);
const PROFILE_H264_MAIN: GUID = GUID::from_u128(0x60b5c1d4_67fe_4790_94d5_c4726d7b6e6d);
const PRESET_P1: GUID = GUID::from_u128(0xfc0a8d3e_45f8_4cf8_80c7_298871590ebf);
const PRESET_P2: GUID = GUID::from_u128(0xf581cfb8_88d6_4381_93f0_df13f9c27dab);
const PRESET_P3: GUID = GUID::from_u128(0x36850110_3a07_441f_94d5_3670631f91f6);

const SUCCESS: i32 = 0;
const DEVICE_TYPE_DIRECTX: u32 = 0;
const TUNING_ULTRA_LOW_LATENCY: u32 = 3;
const RC_CBR: u32 = 2;
const INFINITE_GOP: u32 = 0xffff_ffff;
const BUFFER_FORMAT_NV12: u32 = 1;
const RESOURCE_TYPE_DIRECTX: u32 = 0;
const BUFFER_USAGE_INPUT_IMAGE: u32 = 0;
const PIC_STRUCT_FRAME: u32 = 1;
const PIC_FLAG_FORCEIDR: u32 = 0x2;
const PIC_FLAG_OUTPUT_SPSPPS: u32 = 0x4;
const PIC_TYPE_I: u32 = 2;
const PIC_TYPE_IDR: u32 = 3;
// NV_ENC_CONFIG_H264 第一个字里的位域
const H264_OUTPUT_AUD: u32 = 1 << 6;
const H264_ENABLE_INTRA_REFRESH: u32 = 1 << 10;
const H264_REPEAT_SPSPPS: u32 = 1 << 12;
const H264_ENABLE_LTR: u32 = 1 << 14;
const H264_FILLER_DATA: u32 = 1 << 17;

// ---------------- 结构体 ----------------

#[repr(C)]
#[derive(Clone, Copy)]
struct Qp {
    inter_p: u32,
    inter_b: u32,
    intra: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RcParams {
    version: u32,
    rate_control_mode: u32,
    const_qp: Qp,
    average_bit_rate: u32,
    max_bit_rate: u32,
    vbv_buffer_size: u32,
    vbv_initial_delay: u32,
    /// enableMinQP:1 enableMaxQP:1 enableInitialRCQP:1 enableAQ:1 … zeroReorderDelay（第 9 位）…
    flags: u32,
    min_qp: Qp,
    max_qp: Qp,
    initial_rc_qp: Qp,
    temporal_layer_idx_mask: u32,
    temporal_layer_qp: [u8; 8],
    target_quality: u8,
    target_quality_lsb: u8,
    lookahead_depth: u16,
    low_delay_key_frame_scale: u8,
    y_dc_qp_index_offset: i8,
    u_dc_qp_index_offset: i8,
    v_dc_qp_index_offset: i8,
    qp_map_mode: u32,
    multi_pass: u32,
    alpha_layer_bitrate_ratio: u32,
    cb_qp_index_offset: i8,
    cr_qp_index_offset: i8,
    reserved2: u16,
    reserved: [u32; 4],
}
const _: () = assert!(size_of::<RcParams>() == 128 && offset_of!(RcParams, average_bit_rate) == 20 && offset_of!(RcParams, vbv_buffer_size) == 28);
const _: () = assert!(offset_of!(RcParams, min_qp) == 40 && offset_of!(RcParams, temporal_layer_qp) == 80 && offset_of!(RcParams, lookahead_depth) == 90);
const _: () = assert!(offset_of!(RcParams, qp_map_mode) == 96 && offset_of!(RcParams, multi_pass) == 100 && offset_of!(RcParams, cb_qp_index_offset) == 108 && offset_of!(RcParams, reserved) == 112);

#[repr(C)]
#[derive(Clone, Copy)]
struct H264Vui {
    overscan_info_present_flag: u32,
    overscan_info: u32,
    video_signal_type_present_flag: u32,
    video_format: u32,
    video_full_range_flag: u32,
    colour_description_present_flag: u32,
    colour_primaries: u32,
    transfer_characteristics: u32,
    colour_matrix: u32,
    chroma_sample_location_flag: u32,
    chroma_sample_location_top: u32,
    chroma_sample_location_bot: u32,
    bitstream_restriction_flag: u32,
    timing_info_present_flag: u32,
    num_unit_in_ticks: u32,
    time_scale: u32,
    reserved: [u32; 12],
}
const _: () = assert!(size_of::<H264Vui>() == 112);

#[repr(C)]
#[derive(Clone, Copy)]
struct ConfigH264 {
    /// enableTemporalSVC … outputAUD（第 6 位）… repeatSPSPPS（第 12 位）…
    flags: u32,
    level: u32,
    idr_period: u32,
    separate_colour_plane_flag: u32,
    disable_deblocking_filter_idc: u32,
    num_temporal_layers: u32,
    sps_id: u32,
    pps_id: u32,
    adaptive_transform_mode: u32,
    fmo_mode: u32,
    bdirect_mode: u32,
    entropy_coding_mode: u32,
    stereo_mode: u32,
    intra_refresh_period: u32,
    intra_refresh_cnt: u32,
    max_num_ref_frames: u32,
    slice_mode: u32,
    slice_mode_data: u32,
    vui: H264Vui,
    ltr_num_frames: u32,
    ltr_trust_mode: u32,
    chroma_format_idc: u32,
    max_temporal_layers: u32,
    use_bframes_as_ref: u32,
    num_ref_l0: u32,
    num_ref_l1: u32,
    reserved1: [u32; 267],
    reserved2: [usize; 64],
}
const _: () = assert!(size_of::<ConfigH264>() == 1792 && offset_of!(ConfigH264, idr_period) == 8 && offset_of!(ConfigH264, entropy_coding_mode) == 44);
const _: () = assert!(offset_of!(ConfigH264, slice_mode_data) == 68 && offset_of!(ConfigH264, vui) == 72 && offset_of!(ConfigH264, chroma_format_idc) == 192 && offset_of!(ConfigH264, num_ref_l1) == 208);

/// NV_ENC_CONFIG（编解码器配置是联合体，H.264 那一支最大，正好 1792 字节）
#[repr(C)]
#[derive(Clone, Copy)]
struct Config {
    version: u32,
    profile_guid: GUID,
    gop_length: u32,
    frame_interval_p: i32,
    mono_chrome_encoding: u32,
    frame_field_mode: u32,
    mv_precision: u32,
    rc_params: RcParams,
    h264: ConfigH264,
    reserved: [u32; 278],
    reserved2: [usize; 64],
}
const _: () = assert!(size_of::<Config>() == 3584 && offset_of!(Config, gop_length) == 20 && offset_of!(Config, mv_precision) == 36);
const _: () = assert!(offset_of!(Config, rc_params) == 40 && offset_of!(Config, h264) == 168 && offset_of!(Config, reserved) == 1960 && offset_of!(Config, reserved2) == 3072);

#[repr(C)]
struct PresetConfig {
    version: u32,
    preset_cfg: Config,
    reserved1: [u32; 255],
    reserved2: [usize; 64],
}
const _: () = assert!(size_of::<PresetConfig>() == 5128 && offset_of!(PresetConfig, preset_cfg) == 8 && offset_of!(PresetConfig, reserved1) == 3592);

#[repr(C)]
#[derive(Clone, Copy)]
struct InitParams {
    version: u32,
    encode_guid: GUID,
    preset_guid: GUID,
    encode_width: u32,
    encode_height: u32,
    dar_width: u32,
    dar_height: u32,
    frame_rate_num: u32,
    frame_rate_den: u32,
    enable_encode_async: u32,
    enable_ptd: u32,
    flags: u32,
    priv_data_size: u32,
    priv_data: *mut c_void,
    encode_config: *mut Config,
    max_encode_width: u32,
    max_encode_height: u32,
    max_me_hint_counts_per_block: [[u32; 4]; 2],
    tuning_info: u32,
    buffer_format: u32,
    reserved: [u32; 287],
    reserved2: [usize; 64],
}
const _: () = assert!(size_of::<InitParams>() == 1808 && offset_of!(InitParams, encode_width) == 36 && offset_of!(InitParams, enable_ptd) == 64);
const _: () = assert!(offset_of!(InitParams, priv_data) == 80 && offset_of!(InitParams, encode_config) == 88 && offset_of!(InitParams, max_encode_width) == 96);
const _: () = assert!(offset_of!(InitParams, tuning_info) == 136 && offset_of!(InitParams, buffer_format) == 140 && offset_of!(InitParams, reserved2) == 1296);

#[repr(C)]
struct ReconfigureParams {
    version: u32,
    re_init_encode_params: InitParams,
    /// resetEncoder:1 forceIDR:1
    flags: u32,
}
const _: () = assert!(size_of::<ReconfigureParams>() == 1824 && offset_of!(ReconfigureParams, re_init_encode_params) == 8);

#[repr(C)]
struct OpenSessionParams {
    version: u32,
    device_type: u32,
    device: *mut c_void,
    reserved: *mut c_void,
    api_version: u32,
    reserved1: [u32; 253],
    reserved2: [usize; 64],
}
const _: () = assert!(size_of::<OpenSessionParams>() == 1552 && offset_of!(OpenSessionParams, device) == 8 && offset_of!(OpenSessionParams, api_version) == 24);

#[repr(C)]
struct CreateBitstreamBuffer {
    version: u32,
    size: u32,
    memory_heap: u32,
    reserved: u32,
    bitstream_buffer: *mut c_void,
    bitstream_buffer_ptr: *mut c_void,
    reserved1: [u32; 58],
    reserved2: [usize; 64],
}
const _: () = assert!(size_of::<CreateBitstreamBuffer>() == 776 && offset_of!(CreateBitstreamBuffer, bitstream_buffer) == 16);

#[repr(C)]
struct RegisterResource {
    version: u32,
    resource_type: u32,
    width: u32,
    height: u32,
    pitch: u32,
    sub_resource_index: u32,
    resource_to_register: *mut c_void,
    registered_resource: *mut c_void,
    buffer_format: u32,
    buffer_usage: u32,
    input_fence_point: *mut c_void,
    reserved1: [u32; 247],
    reserved2: [usize; 61],
}
const _: () = assert!(size_of::<RegisterResource>() == 1536 && offset_of!(RegisterResource, resource_to_register) == 24 && offset_of!(RegisterResource, registered_resource) == 32);
const _: () = assert!(offset_of!(RegisterResource, buffer_format) == 40 && offset_of!(RegisterResource, buffer_usage) == 44 && offset_of!(RegisterResource, input_fence_point) == 48);

#[repr(C)]
struct MapInputResource {
    version: u32,
    sub_resource_index: u32,
    input_resource: *mut c_void,
    registered_resource: *mut c_void,
    mapped_resource: *mut c_void,
    mapped_buffer_fmt: u32,
    reserved1: [u32; 251],
    reserved2: [usize; 63],
}
const _: () = assert!(size_of::<MapInputResource>() == 1544 && offset_of!(MapInputResource, registered_resource) == 16 && offset_of!(MapInputResource, mapped_resource) == 24);
const _: () = assert!(offset_of!(MapInputResource, mapped_buffer_fmt) == 32);

/// NV_ENC_PIC_PARAMS：只写到 pictureType，后面（各编解码器的逐帧参数等）全是 0
#[repr(C)]
struct PicParams {
    version: u32,
    input_width: u32,
    input_height: u32,
    input_pitch: u32,
    encode_pic_flags: u32,
    frame_idx: u32,
    input_time_stamp: u64,
    input_duration: u64,
    input_buffer: *mut c_void,
    output_bitstream: *mut c_void,
    completion_event: *mut c_void,
    buffer_fmt: u32,
    picture_struct: u32,
    picture_type: u32,
    rest: [u32; 821],
}
const _: () = assert!(size_of::<PicParams>() == 3360 && offset_of!(PicParams, encode_pic_flags) == 16 && offset_of!(PicParams, input_time_stamp) == 24);
const _: () = assert!(offset_of!(PicParams, input_buffer) == 40 && offset_of!(PicParams, output_bitstream) == 48 && offset_of!(PicParams, completion_event) == 56);
const _: () = assert!(offset_of!(PicParams, buffer_fmt) == 64 && offset_of!(PicParams, picture_struct) == 68 && offset_of!(PicParams, picture_type) == 72);

#[repr(C)]
struct LockBitstream {
    version: u32,
    /// doNotWait:1 ltrFrame:1 getRCStats:1
    flags: u32,
    output_bitstream: *mut c_void,
    slice_offsets: *mut u32,
    frame_idx: u32,
    hw_encode_status: u32,
    num_slices: u32,
    bitstream_size_in_bytes: u32,
    output_time_stamp: u64,
    output_duration: u64,
    bitstream_buffer_ptr: *mut c_void,
    picture_type: u32,
    picture_struct: u32,
    frame_avg_qp: u32,
    rest: [u32; 367],
}
const _: () = assert!(size_of::<LockBitstream>() == 1544 && offset_of!(LockBitstream, output_bitstream) == 8 && offset_of!(LockBitstream, bitstream_size_in_bytes) == 36);
const _: () = assert!(offset_of!(LockBitstream, output_time_stamp) == 40 && offset_of!(LockBitstream, bitstream_buffer_ptr) == 56 && offset_of!(LockBitstream, picture_type) == 64);
const _: () = assert!(offset_of!(LockBitstream, frame_avg_qp) == 72);

type Enc = *mut c_void;
type FnOpenSessionEx = unsafe extern "C" fn(*mut OpenSessionParams, *mut Enc) -> i32;
type FnGetPresetConfigEx = unsafe extern "C" fn(Enc, GUID, GUID, u32, *mut PresetConfig) -> i32;
type FnInitialize = unsafe extern "C" fn(Enc, *mut InitParams) -> i32;
type FnCreateBitstream = unsafe extern "C" fn(Enc, *mut CreateBitstreamBuffer) -> i32;
type FnPtr = unsafe extern "C" fn(Enc, *mut c_void) -> i32;
type FnEncodePicture = unsafe extern "C" fn(Enc, *mut PicParams) -> i32;
type FnLockBitstream = unsafe extern "C" fn(Enc, *mut LockBitstream) -> i32;
type FnMapInput = unsafe extern "C" fn(Enc, *mut MapInputResource) -> i32;
type FnDestroy = unsafe extern "C" fn(Enc) -> i32;
type FnRegister = unsafe extern "C" fn(Enc, *mut RegisterResource) -> i32;
type FnReconfigure = unsafe extern "C" fn(Enc, *mut ReconfigureParams) -> i32;
type FnLastError = unsafe extern "C" fn(Enc) -> *const i8;

/// NV_ENCODE_API_FUNCTION_LIST：41 个函数指针（含一个保留位）+ 277 个保留
#[repr(C)]
struct FunctionList {
    version: u32,
    reserved: u32,
    open_encode_session: usize,
    get_encode_guid_count: usize,
    get_encode_profile_guid_count: usize,
    get_encode_profile_guids: usize,
    get_encode_guids: usize,
    get_input_format_count: usize,
    get_input_formats: usize,
    get_encode_caps: usize,
    get_encode_preset_count: usize,
    get_encode_preset_guids: usize,
    get_encode_preset_config: usize,
    initialize_encoder: Option<FnInitialize>,
    create_input_buffer: usize,
    destroy_input_buffer: usize,
    create_bitstream_buffer: Option<FnCreateBitstream>,
    destroy_bitstream_buffer: Option<FnPtr>,
    encode_picture: Option<FnEncodePicture>,
    lock_bitstream: Option<FnLockBitstream>,
    unlock_bitstream: Option<FnPtr>,
    lock_input_buffer: usize,
    unlock_input_buffer: usize,
    get_encode_stats: usize,
    get_sequence_params: usize,
    register_async_event: usize,
    unregister_async_event: usize,
    map_input_resource: Option<FnMapInput>,
    unmap_input_resource: Option<FnPtr>,
    destroy_encoder: Option<FnDestroy>,
    invalidate_ref_frames: usize,
    open_encode_session_ex: Option<FnOpenSessionEx>,
    register_resource: Option<FnRegister>,
    unregister_resource: Option<FnPtr>,
    reconfigure_encoder: Option<FnReconfigure>,
    reserved1: usize,
    create_mv_buffer: usize,
    destroy_mv_buffer: usize,
    run_motion_estimation_only: usize,
    get_last_error_string: Option<FnLastError>,
    set_io_cuda_streams: usize,
    get_encode_preset_config_ex: Option<FnGetPresetConfigEx>,
    get_sequence_param_ex: usize,
    reserved2: [usize; 277],
}
const _: () = assert!(size_of::<FunctionList>() == 2552 && offset_of!(FunctionList, get_encode_guids) == 40 && offset_of!(FunctionList, encode_picture) == 136);
const _: () = assert!(offset_of!(FunctionList, map_input_resource) == 208 && offset_of!(FunctionList, destroy_encoder) == 224 && offset_of!(FunctionList, open_encode_session_ex) == 240);
const _: () = assert!(offset_of!(FunctionList, register_resource) == 248 && offset_of!(FunctionList, reconfigure_encoder) == 264 && offset_of!(FunctionList, get_last_error_string) == 304);
const _: () = assert!(offset_of!(FunctionList, get_encode_preset_config_ex) == 320);

/// 全 0 的结构体（这些都是 C 的 POD，0 是合法值）
fn zeroed<T>() -> Box<T> {
    unsafe { Box::new(std::mem::zeroed()) }
}

// ---------------- CUDA 驱动接口（只用到的几个） ----------------

type CuCtx = *mut c_void;
type CuDevPtr = u64;

/// CUDA_MEMCPY2D（dynlink_cuda.h）
#[repr(C)]
struct Memcpy2D {
    src_x_in_bytes: usize,
    src_y: usize,
    src_memory_type: u32,
    src_host: *const c_void,
    src_device: CuDevPtr,
    src_array: *mut c_void,
    src_pitch: usize,
    dst_x_in_bytes: usize,
    dst_y: usize,
    dst_memory_type: u32,
    dst_host: *mut c_void,
    dst_device: CuDevPtr,
    dst_array: *mut c_void,
    dst_pitch: usize,
    width_in_bytes: usize,
    height: usize,
}
const _: () = assert!(size_of::<Memcpy2D>() == 128 && offset_of!(Memcpy2D, src_host) == 24 && offset_of!(Memcpy2D, dst_memory_type) == 72 && offset_of!(Memcpy2D, width_in_bytes) == 112);
const CU_MEMORYTYPE_HOST: u32 = 1;
const CU_MEMORYTYPE_DEVICE: u32 = 2;

crate::dl_fns! {
    struct Cuda {
        cuInit: fn(u32) -> i32;
        cuDeviceGetCount: fn(*mut i32) -> i32;
        cuDeviceGet: fn(*mut i32, i32) -> i32;
        cuDeviceGetName: fn(*mut c_char, i32, i32) -> i32;
        cuCtxCreate_v2: fn(*mut CuCtx, u32, i32) -> i32;
        cuCtxDestroy_v2: fn(CuCtx) -> i32;
        cuCtxPushCurrent_v2: fn(CuCtx) -> i32;
        cuCtxPopCurrent_v2: fn(*mut CuCtx) -> i32;
        cuMemAllocPitch_v2: fn(*mut CuDevPtr, *mut usize, usize, usize, u32) -> i32;
        cuMemFree_v2: fn(CuDevPtr) -> i32;
        cuMemcpy2D_v2: fn(*const Memcpy2D) -> i32;
    }
}

fn cu(st: i32, what: &str) -> anyhow::Result<()> {
    anyhow::ensure!(st == 0, "CUDA {what}失败（{st}）");
    Ok(())
}

/// 进入 / 离开 CUDA 上下文（NVENC 与拷贝都要在上下文里做）
struct CtxGuard<'a>(&'a Cuda);
impl<'a> CtxGuard<'a> {
    fn push(c: &'a Cuda, ctx: CuCtx) -> anyhow::Result<Self> {
        cu(unsafe { (c.cuCtxPushCurrent_v2)(ctx) }, "切换上下文")?;
        Ok(Self(c))
    }
}
impl Drop for CtxGuard<'_> {
    fn drop(&mut self) {
        let mut old: CuCtx = std::ptr::null_mut();
        unsafe { (self.0.cuCtxPopCurrent_v2)(&mut old) };
    }
}

// ---------------- 编码器 ----------------

fn preset() -> (GUID, u32) {
    match std::env::var("LEMON_GALAXY_NVENC_PRESET").ok().and_then(|s| s.trim().trim_start_matches(['p', 'P']).parse::<u32>().ok()) {
        Some(1) => (PRESET_P1, 1),
        Some(3) => (PRESET_P3, 3),
        _ => (PRESET_P2, 2),
    }
}

/// CBR：平均码率按 `rate.kbps`（画面帧率低时 galaxy_host::pacing 已放大），VBV 按时间算（决定单帧最大，见 pacing::VBV_MS）
fn apply_rate(rc: &mut RcParams, rate: EncoderRate, fps: u32) {
    let bps = rate.kbps.clamp(200, 500_000) * 1000;
    rc.rate_control_mode = RC_CBR;
    rc.average_bit_rate = bps;
    rc.max_bit_rate = bps;
    rc.vbv_buffer_size = rate.vbv_bits(bps, fps);
    rc.vbv_initial_delay = rc.vbv_buffer_size;
    // 量化参数下限：画面停下后补编、预算放大时，不往近乎无损里砸码率（实测不设时静止画面每帧约 6.5 KB 反复重编噪声，永不收敛）
    rc.flags |= RC_ENABLE_MIN_QP;
    rc.min_qp = Qp { inter_p: MIN_QP_P, inter_b: MIN_QP_P, intra: MIN_QP_I };
}

/// enableMinQP（RcParams.flags 第 0 位）
const RC_ENABLE_MIN_QP: u32 = 1;
/// 量化参数下限：P 帧 20、I 帧 18，文字已经看不出损失
const MIN_QP_P: u32 = 20;
const MIN_QP_I: u32 = 18;

pub struct Nvenc {
    cuda: Cuda,
    ctx: CuCtx,
    api: Box<FunctionList>,
    enc: Enc,
    bitstream: *mut c_void,
    input: CuDevPtr,
    pitch: usize,
    registered: *mut c_void,
    init: Box<InitParams>,
    config: Box<Config>,
    width: u32,
    height: u32,
    fps: u32,
    frame: u32,
    name: String,
}

// 只在采集线程里用
unsafe impl Send for Nvenc {}

fn libs() -> anyhow::Result<(&'static Lib, &'static Lib)> {
    use std::sync::OnceLock;
    static CUDA: OnceLock<Result<Lib, String>> = OnceLock::new();
    static ENC: OnceLock<Result<Lib, String>> = OnceLock::new();
    let c = CUDA.get_or_init(|| Lib::open(&["libcuda.so.1", "libcuda.so"])).as_ref().map_err(|e| anyhow::anyhow!("没有 NVIDIA 驱动（{e}）"))?;
    let e = ENC.get_or_init(|| Lib::open(&["libnvidia-encode.so.1", "libnvidia-encode.so"])).as_ref().map_err(|e| anyhow::anyhow!("没有 NVIDIA 编码库 libnvidia-encode（{e}）"))?;
    Ok((c, e))
}

impl Nvenc {
    pub fn new(width: u32, height: u32, fps: u32, kbps: u32) -> anyhow::Result<Self> {
        let (cuda_lib, enc_lib) = libs()?;
        let cuda = Cuda::load(cuda_lib).map_err(anyhow::Error::msg)?;
        unsafe {
            cu((cuda.cuInit)(0), "初始化")?;
            let mut n = 0;
            cu((cuda.cuDeviceGetCount)(&mut n), "列设备")?;
            anyhow::ensure!(n > 0, "没有 NVIDIA 显卡");
            let idx: i32 = std::env::var("LEMON_GALAXY_CUDA_DEVICE").ok().and_then(|s| s.parse().ok()).unwrap_or(0).clamp(0, n - 1);
            let mut dev = 0;
            cu((cuda.cuDeviceGet)(&mut dev, idx), "取设备")?;
            let mut name = [0 as c_char; 128];
            (cuda.cuDeviceGetName)(name.as_mut_ptr(), 127, dev);
            let gpu = std::ffi::CStr::from_ptr(name.as_ptr()).to_string_lossy().into_owned();
            let mut ctx: CuCtx = std::ptr::null_mut();
            cu((cuda.cuCtxCreate_v2)(&mut ctx, 0, dev), "建上下文")?;
            // 建完是当前线程的上下文：先弹出，用的时候再压进来
            let mut old: CuCtx = std::ptr::null_mut();
            (cuda.cuCtxPopCurrent_v2)(&mut old);
            let mut me = Self {
                cuda,
                ctx,
                api: zeroed(),
                enc: std::ptr::null_mut(),
                bitstream: std::ptr::null_mut(),
                input: 0,
                pitch: 0,
                registered: std::ptr::null_mut(),
                init: zeroed(),
                config: zeroed(),
                width,
                height,
                fps: fps.max(1),
                frame: 0,
                name: String::new(),
            };
            me.open(enc_lib, kbps)?;
            let (_, p) = preset();
            me.name = format!("NVENC H.264 P{p}（{gpu}）");
            Ok(me)
        }
    }

    fn open(&mut self, lib: &Lib, kbps: u32) -> anyhow::Result<()> {
        // 借用 self.cuda 的同时还要改 self 的其他字段：先拷出函数指针
        let (push, pop) = (self.cuda.cuCtxPushCurrent_v2, self.cuda.cuCtxPopCurrent_v2);
        unsafe {
            push(self.ctx);
        }
        let r = self.open_in_ctx(lib, kbps);
        let mut old: CuCtx = std::ptr::null_mut();
        unsafe {
            pop(&mut old);
        }
        r
    }

    fn open_in_ctx(&mut self, lib: &Lib, kbps: u32) -> anyhow::Result<()> {
        unsafe {
            let max_ver: unsafe extern "C" fn(*mut u32) -> i32 = lib.sym(c"NvEncodeAPIGetMaxSupportedVersion").map_err(anyhow::Error::msg)?;
            let create: unsafe extern "C" fn(*mut FunctionList) -> i32 = lib.sym(c"NvEncodeAPICreateInstance").map_err(anyhow::Error::msg)?;
            let mut v = 0u32;
            max_ver(&mut v);
            anyhow::ensure!((v >> 4, v & 0xf) >= (API_MAJOR, API_MINOR), "NVIDIA 驱动太旧（支持到 NVENC {}.{}，需要 {API_MAJOR}.{API_MINOR}，即 520 版驱动起）", v >> 4, v & 0xf);
            self.api.version = FUNCTION_LIST_VER;
            check(create(&mut *self.api), "取接口")?;
            let open = self.api.open_encode_session_ex.ok_or_else(|| anyhow::anyhow!("NVENC 接口不完整"))?;
            let mut sp: Box<OpenSessionParams> = zeroed();
            sp.version = OPEN_SESSION_VER;
            sp.device_type = DEVICE_TYPE_CUDA;
            sp.device = self.ctx;
            sp.api_version = API_VERSION;
            let mut enc: Enc = std::ptr::null_mut();
            check(open(&mut *sp, &mut enc), "打开编码会话（GeForce 同时编码路数可能已满）")?;
            self.enc = enc;

            let (preset_guid, _) = preset();
            let mut pc: Box<PresetConfig> = zeroed();
            pc.version = PRESET_CONFIG_VER;
            pc.preset_cfg.version = CONFIG_VER;
            let get_preset = self.api.get_encode_preset_config_ex.ok_or_else(|| anyhow::anyhow!("NVENC 接口不完整"))?;
            let st = get_preset(self.enc, CODEC_H264, preset_guid, TUNING_ULTRA_LOW_LATENCY, &mut *pc);
            self.check(st, "取 H.264 预设（这块显卡可能不支持 H.264 编码）")?;

            let c = &mut *self.config;
            *c = pc.preset_cfg;
            c.version = CONFIG_VER;
            c.profile_guid = PROFILE_H264_MAIN;
            c.gop_length = INFINITE_GOP;
            c.frame_interval_p = 1;
            apply_rate(&mut c.rc_params, EncoderRate::plain(kbps), self.fps);
            let h = &mut c.h264;
            h.flags = (h.flags | H264_REPEAT_SPSPPS) & !(H264_OUTPUT_AUD | H264_ENABLE_INTRA_REFRESH | H264_ENABLE_LTR | H264_FILLER_DATA);
            h.idr_period = INFINITE_GOP;
            h.chroma_format_idc = 1;
            h.slice_mode = 0;
            h.slice_mode_data = 0;
            h.vui.video_signal_type_present_flag = 0;
            h.vui.colour_description_present_flag = 0;

            let i = &mut *self.init;
            i.version = INIT_PARAMS_VER;
            i.encode_guid = CODEC_H264;
            i.preset_guid = preset_guid;
            i.encode_width = self.width;
            i.encode_height = self.height;
            i.dar_width = self.width;
            i.dar_height = self.height;
            i.frame_rate_num = self.fps;
            i.frame_rate_den = 1;
            i.enable_encode_async = 0;
            i.enable_ptd = 1;
            i.encode_config = &mut *self.config;
            i.max_encode_width = self.width;
            i.max_encode_height = self.height;
            i.tuning_info = TUNING_ULTRA_LOW_LATENCY;
            i.buffer_format = BUFFER_FORMAT_ARGB;
            let init = self.api.initialize_encoder.ok_or_else(|| anyhow::anyhow!("NVENC 接口不完整"))?;
            let st = init(self.enc, &mut *self.init);
            self.check(st, "初始化")?;

            let mut bb: Box<CreateBitstreamBuffer> = zeroed();
            bb.version = BITSTREAM_BUFFER_VER;
            let cb = self.api.create_bitstream_buffer.ok_or_else(|| anyhow::anyhow!("NVENC 接口不完整"))?;
            let st = cb(self.enc, &mut *bb);
            self.check(st, "输出缓冲")?;
            self.bitstream = bb.bitstream_buffer;

            // 输入：一张显存，登记一次
            let mut dptr: CuDevPtr = 0;
            let mut pitch = 0usize;
            cu((self.cuda.cuMemAllocPitch_v2)(&mut dptr, &mut pitch, self.width as usize * 4, self.height as usize, 16), "分配显存")?;
            self.input = dptr;
            self.pitch = pitch;
            let mut r: Box<RegisterResource> = zeroed();
            r.version = REGISTER_VER;
            r.resource_type = RESOURCE_TYPE_CUDADEVICEPTR;
            r.width = self.width;
            r.height = self.height;
            r.pitch = pitch as u32;
            r.resource_to_register = dptr as *mut c_void;
            r.buffer_format = BUFFER_FORMAT_ARGB;
            r.buffer_usage = BUFFER_USAGE_INPUT_IMAGE;
            let f = self.api.register_resource.ok_or_else(|| anyhow::anyhow!("NVENC 接口不完整"))?;
            let st = f(self.enc, &mut *r);
            self.check(st, "登记输入显存")?;
            self.registered = r.registered_resource;
        }
        Ok(())
    }

    fn check(&self, status: i32, what: &str) -> anyhow::Result<()> {
        if status == SUCCESS {
            return Ok(());
        }
        let detail = unsafe {
            match (self.api.get_last_error_string, self.enc.is_null()) {
                (Some(f), false) => {
                    let p = f(self.enc);
                    if p.is_null() {
                        String::new()
                    } else {
                        std::ffi::CStr::from_ptr(p as *const c_char).to_string_lossy().into_owned()
                    }
                }
                _ => String::new(),
            }
        };
        anyhow::bail!("NVENC {what}失败（{status}{}{detail}）", if detail.is_empty() { "" } else { "：" })
    }
}

fn check(status: i32, what: &str) -> anyhow::Result<()> {
    anyhow::ensure!(status == SUCCESS, "NVENC {what}失败（{status}）");
    Ok(())
}

impl HwEncoder for Nvenc {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn encode(&mut self, bgra: &[u8], src_w: u32, keyframe: bool, pts_us: u64) -> anyhow::Result<Vec<EncodedFrame>> {
        anyhow::ensure!(bgra.len() >= src_w as usize * self.height as usize * 4 && src_w >= self.width, "画面尺寸不对");
        let _g = CtxGuard::push(&self.cuda, self.ctx)?;
        let copy = Memcpy2D {
            src_x_in_bytes: 0,
            src_y: 0,
            src_memory_type: CU_MEMORYTYPE_HOST,
            src_host: bgra.as_ptr() as *const c_void,
            src_device: 0,
            src_array: std::ptr::null_mut(),
            src_pitch: src_w as usize * 4,
            dst_x_in_bytes: 0,
            dst_y: 0,
            dst_memory_type: CU_MEMORYTYPE_DEVICE,
            dst_host: std::ptr::null_mut(),
            dst_device: self.input,
            dst_array: std::ptr::null_mut(),
            dst_pitch: self.pitch,
            width_in_bytes: self.width as usize * 4,
            height: self.height as usize,
        };
        cu(unsafe { (self.cuda.cuMemcpy2D_v2)(&copy) }, "上传画面")?;
        let api = &*self.api;
        let (Some(map), Some(unmap), Some(encode), Some(lock), Some(unlock)) = (api.map_input_resource, api.unmap_input_resource, api.encode_picture, api.lock_bitstream, api.unlock_bitstream) else {
            anyhow::bail!("NVENC 接口不完整")
        };
        unsafe {
            let mut m: Box<MapInputResource> = zeroed();
            m.version = MAP_INPUT_VER;
            m.registered_resource = self.registered;
            self.check(map(self.enc, &mut *m), "映射输入")?;
            let mut p: Box<PicParams> = zeroed();
            p.version = PIC_PARAMS_VER;
            p.input_width = self.width;
            p.input_height = self.height;
            p.input_pitch = self.pitch as u32;
            p.encode_pic_flags = if keyframe { PIC_FLAG_FORCEIDR | PIC_FLAG_OUTPUT_SPSPPS } else { 0 };
            p.frame_idx = self.frame;
            p.input_time_stamp = pts_us;
            p.input_buffer = m.mapped_resource;
            p.output_bitstream = self.bitstream;
            p.buffer_fmt = m.mapped_buffer_fmt;
            p.picture_struct = PIC_STRUCT_FRAME;
            self.frame = self.frame.wrapping_add(1);
            let st = encode(self.enc, &mut *p);
            if st != SUCCESS {
                unmap(self.enc, m.mapped_resource);
                return self.check(st, "编码").map(|_| vec![]);
            }
            let mut l: Box<LockBitstream> = zeroed();
            l.version = LOCK_BITSTREAM_VER;
            l.output_bitstream = self.bitstream;
            let st = lock(self.enc, &mut *l);
            if st != SUCCESS {
                unmap(self.enc, m.mapped_resource);
                return self.check(st, "取输出").map(|_| vec![]);
            }
            let data = std::slice::from_raw_parts(l.bitstream_buffer_ptr as *const u8, l.bitstream_size_in_bytes as usize).to_vec();
            let key = l.picture_type == PIC_TYPE_IDR || l.picture_type == PIC_TYPE_I;
            unlock(self.enc, self.bitstream);
            unmap(self.enc, m.mapped_resource);
            if data.is_empty() {
                return Ok(vec![]);
            }
            Ok(vec![EncodedFrame { keyframe: key, data }])
        }
    }

    fn set_bitrate(&mut self, rate: EncoderRate) -> bool {
        let Some(f) = self.api.reconfigure_encoder else { return false };
        let Ok(_g) = CtxGuard::push(&self.cuda, self.ctx) else { return false };
        let mut cfg = *self.config;
        apply_rate(&mut cfg.rc_params, rate, self.fps);
        let mut r: Box<ReconfigureParams> = zeroed();
        r.version = RECONFIGURE_VER;
        r.re_init_encode_params = *self.init;
        r.re_init_encode_params.encode_config = &mut cfg;
        r.flags = 0;
        if unsafe { f(self.enc, &mut *r) } != SUCCESS {
            return false;
        }
        *self.config = cfg;
        self.init.encode_config = &mut *self.config;
        true
    }
}

impl Drop for Nvenc {
    fn drop(&mut self) {
        unsafe {
            (self.cuda.cuCtxPushCurrent_v2)(self.ctx);
            if !self.enc.is_null() {
                if !self.registered.is_null() {
                    if let Some(f) = self.api.unregister_resource {
                        f(self.enc, self.registered);
                    }
                }
                if !self.bitstream.is_null() {
                    if let Some(f) = self.api.destroy_bitstream_buffer {
                        f(self.enc, self.bitstream);
                    }
                }
                if let Some(f) = self.api.destroy_encoder {
                    f(self.enc);
                }
            }
            if self.input != 0 {
                (self.cuda.cuMemFree_v2)(self.input);
            }
            let mut old: CuCtx = std::ptr::null_mut();
            (self.cuda.cuCtxPopCurrent_v2)(&mut old);
            (self.cuda.cuCtxDestroy_v2)(self.ctx);
        }
    }
}
