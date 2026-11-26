//! NVIDIA NVENC 直连：运行时加载驱动自带的 nvEncodeAPI64.dll（没装 NVIDIA 驱动的机器上自然加载失败，不影响启动）。
//! 结构体按 Video Codec SDK 12.0 的 nvEncodeAPI.h 手写，只写用到的字段，其余按头文件的总大小补齐；
//! 尺寸与关键偏移用编译期断言核对过（数值来自 clang 对原头文件的 sizeof / offsetof，x64 与 ARM64 相同）。
//! 新驱动兼容旧版本接口，所以按 12.0 调用。
//!
//! 用法：同步编码、超低延迟调优、P1–P3 预设、无 B 帧、无限 GOP + 按需 IDR（IDR 前重复 SPS/PPS）、不写 AUD，
//! 码率用 nvEncReconfigureEncoder 运行时修改（不重置、不出关键帧）。
//! Media Foundation 里 NVIDIA 自己的编码器每帧要压约 29 ms 且一次只收一帧，所以 NVIDIA 显卡直接走这里。

use super::{has_parameter_sets, strip_aud, EncodedFrame, EncoderRate, HwEncoder};
use std::ffi::c_void;
use std::mem::{offset_of, size_of};
use windows::core::{Interface, GUID, PCSTR, PCWSTR};
use windows::Win32::Foundation::{FreeLibrary, HMODULE};
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;

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
type FnOpenSessionEx = unsafe extern "system" fn(*mut OpenSessionParams, *mut Enc) -> i32;
type FnGetPresetConfigEx = unsafe extern "system" fn(Enc, GUID, GUID, u32, *mut PresetConfig) -> i32;
type FnInitialize = unsafe extern "system" fn(Enc, *mut InitParams) -> i32;
type FnCreateBitstream = unsafe extern "system" fn(Enc, *mut CreateBitstreamBuffer) -> i32;
type FnPtr = unsafe extern "system" fn(Enc, *mut c_void) -> i32;
type FnEncodePicture = unsafe extern "system" fn(Enc, *mut PicParams) -> i32;
type FnLockBitstream = unsafe extern "system" fn(Enc, *mut LockBitstream) -> i32;
type FnMapInput = unsafe extern "system" fn(Enc, *mut MapInputResource) -> i32;
type FnDestroy = unsafe extern "system" fn(Enc) -> i32;
type FnRegister = unsafe extern "system" fn(Enc, *mut RegisterResource) -> i32;
type FnReconfigure = unsafe extern "system" fn(Enc, *mut ReconfigureParams) -> i32;
type FnLastError = unsafe extern "system" fn(Enc) -> *const i8;

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

// ---------------- 编码器 ----------------

/// 用的预设（1–3，越大越慢画质越好）。默认 P2，可用环境变量 LEMON_GALAXY_NVENC_PRESET 调。
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
    lib: HMODULE,
    api: Box<FunctionList>,
    enc: Enc,
    bitstream: *mut c_void,
    registered: Vec<(usize, *mut c_void, ID3D11Texture2D)>,
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

impl Nvenc {
    pub fn new(device: &ID3D11Device, width: u32, height: u32, fps: u32, kbps: u32, gpu: &str) -> anyhow::Result<Self> {
        let lib = load_library()?;
        let mut me = Self {
            lib,
            api: zeroed(),
            enc: std::ptr::null_mut(),
            bitstream: std::ptr::null_mut(),
            registered: Vec::new(),
            init: zeroed(),
            config: zeroed(),
            width,
            height,
            fps: fps.max(1),
            frame: 0,
            name: String::new(),
        };
        me.open(device, kbps)?;
        let (_, p) = preset();
        me.name = format!("NVENC H.264 P{p}（{gpu}）");
        Ok(me)
    }

    fn open(&mut self, device: &ID3D11Device, kbps: u32) -> anyhow::Result<()> {
        unsafe {
            let max_ver: Option<unsafe extern "system" fn(*mut u32) -> i32> = std::mem::transmute(GetProcAddress(self.lib, PCSTR(b"NvEncodeAPIGetMaxSupportedVersion\0".as_ptr())));
            let create: Option<unsafe extern "system" fn(*mut FunctionList) -> i32> = std::mem::transmute(GetProcAddress(self.lib, PCSTR(b"NvEncodeAPICreateInstance\0".as_ptr())));
            let (Some(max_ver), Some(create)) = (max_ver, create) else { anyhow::bail!("nvEncodeAPI64.dll 不完整") };
            let mut v = 0u32;
            max_ver(&mut v);
            anyhow::ensure!((v >> 4, v & 0xf) >= (API_MAJOR, API_MINOR), "NVIDIA 驱动太旧（支持到 NVENC {}.{}，需要 {API_MAJOR}.{API_MINOR}）", v >> 4, v & 0xf);
            self.api.version = FUNCTION_LIST_VER;
            check(create(&mut *self.api), "取接口")?;
            let open = self.api.open_encode_session_ex.ok_or_else(|| anyhow::anyhow!("NVENC 接口不完整"))?;
            let mut sp: Box<OpenSessionParams> = zeroed();
            sp.version = OPEN_SESSION_VER;
            sp.device_type = DEVICE_TYPE_DIRECTX;
            sp.device = device.as_raw();
            sp.api_version = API_VERSION;
            let mut enc: Enc = std::ptr::null_mut();
            check(open(&mut *sp, &mut enc), "打开编码会话")?;
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
            // 关键帧只在需要时出（主控端请求、新观众）；没有 B 帧
            c.gop_length = INFINITE_GOP;
            c.frame_interval_p = 1;
            apply_rate(&mut c.rc_params, EncoderRate::plain(kbps), self.fps);
            let h = &mut c.h264;
            h.flags = (h.flags | H264_REPEAT_SPSPPS) & !(H264_OUTPUT_AUD | H264_ENABLE_INTRA_REFRESH | H264_ENABLE_LTR | H264_FILLER_DATA);
            h.idr_period = INFINITE_GOP;
            h.chroma_format_idc = 1;
            // 一帧一个条带；不写色彩描述（与软件编码一致，主控端按 BT.601 有限范围显示）
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
            i.buffer_format = BUFFER_FORMAT_NV12;
            let init = self.api.initialize_encoder.ok_or_else(|| anyhow::anyhow!("NVENC 接口不完整"))?;
            let st = init(self.enc, &mut *self.init);
            self.check(st, "初始化")?;

            let mut bb: Box<CreateBitstreamBuffer> = zeroed();
            bb.version = BITSTREAM_BUFFER_VER;
            let cb = self.api.create_bitstream_buffer.ok_or_else(|| anyhow::anyhow!("NVENC 接口不完整"))?;
            let st = cb(self.enc, &mut *bb);
            self.check(st, "输出缓冲")?;
            self.bitstream = bb.bitstream_buffer;
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
                        std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
                    }
                }
                _ => String::new(),
            }
        };
        anyhow::bail!("NVENC {what}失败（{status}{}{detail}）", if detail.is_empty() { "" } else { "：" })
    }

    fn register(&mut self, tex: &ID3D11Texture2D) -> anyhow::Result<*mut c_void> {
        let key = tex.as_raw() as usize;
        if let Some((_, h, _)) = self.registered.iter().find(|(k, _, _)| *k == key) {
            return Ok(*h);
        }
        let mut r: Box<RegisterResource> = zeroed();
        r.version = REGISTER_VER;
        r.resource_type = RESOURCE_TYPE_DIRECTX;
        r.width = self.width;
        r.height = self.height;
        r.resource_to_register = tex.as_raw();
        r.buffer_format = BUFFER_FORMAT_NV12;
        r.buffer_usage = BUFFER_USAGE_INPUT_IMAGE;
        let f = self.api.register_resource.ok_or_else(|| anyhow::anyhow!("NVENC 接口不完整"))?;
        self.check(unsafe { f(self.enc, &mut *r) }, "登记输入纹理")?;
        self.registered.push((key, r.registered_resource, tex.clone()));
        Ok(r.registered_resource)
    }
}

fn check(status: i32, what: &str) -> anyhow::Result<()> {
    anyhow::ensure!(status == SUCCESS, "NVENC {what}失败（{status}）");
    Ok(())
}

/// 从系统目录加载（不搜当前目录，防止被同名 DLL 劫持）。
fn load_library() -> anyhow::Result<HMODULE> {
    let mut buf = [0u16; 260];
    let n = unsafe { GetSystemDirectoryW(Some(&mut buf)) } as usize;
    anyhow::ensure!(n > 0 && n < buf.len(), "找不到系统目录");
    let path: Vec<u16> = String::from_utf16_lossy(&buf[..n]).encode_utf16().chain("\\nvEncodeAPI64.dll".encode_utf16()).chain(Some(0)).collect();
    unsafe { LoadLibraryW(PCWSTR(path.as_ptr())) }.map_err(|_| anyhow::anyhow!("没有 NVIDIA 编码运行库（nvEncodeAPI64.dll）"))
}

impl HwEncoder for Nvenc {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn encode(&mut self, nv12: &ID3D11Texture2D, keyframe: bool, pts_us: u64) -> anyhow::Result<Vec<EncodedFrame>> {
        let res = self.register(nv12)?;
        let api = &*self.api;
        let (Some(map), Some(unmap), Some(encode), Some(lock), Some(unlock)) = (api.map_input_resource, api.unmap_input_resource, api.encode_picture, api.lock_bitstream, api.unlock_bitstream) else {
            anyhow::bail!("NVENC 接口不完整")
        };
        unsafe {
            let mut m: Box<MapInputResource> = zeroed();
            m.version = MAP_INPUT_VER;
            m.registered_resource = res;
            self.check(map(self.enc, &mut *m), "映射输入")?;
            let mut p: Box<PicParams> = zeroed();
            p.version = PIC_PARAMS_VER;
            p.input_width = self.width;
            p.input_height = self.height;
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
            let mut data = std::slice::from_raw_parts(l.bitstream_buffer_ptr as *const u8, l.bitstream_size_in_bytes as usize).to_vec();
            let key = l.picture_type == PIC_TYPE_IDR || l.picture_type == PIC_TYPE_I;
            unlock(self.enc, self.bitstream);
            unmap(self.enc, m.mapped_resource);
            strip_aud(&mut data);
            if data.is_empty() {
                return Ok(vec![]);
            }
            Ok(vec![EncodedFrame { keyframe: key && has_parameter_sets(&data), data }])
        }
    }

    fn set_bitrate(&mut self, rate: EncoderRate) -> bool {
        let Some(f) = self.api.reconfigure_encoder else { return false };
        let mut cfg = *self.config;
        apply_rate(&mut cfg.rc_params, rate, self.fps);
        let mut r: Box<ReconfigureParams> = zeroed();
        r.version = RECONFIGURE_VER;
        r.re_init_encode_params = *self.init;
        r.re_init_encode_params.encode_config = &mut cfg;
        // resetEncoder = 0、forceIDR = 0：只改码控，不出关键帧
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
            if !self.enc.is_null() {
                if let Some(f) = self.api.unregister_resource {
                    for (_, h, _) in &self.registered {
                        f(self.enc, *h);
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
            self.registered.clear();
            let _ = FreeLibrary(self.lib);
        }
    }
}
