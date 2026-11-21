//! LemonGalaxy 客户端内核（主控端与被控端共用的服务器交互，以及主控端的连接与会话）。

pub mod account;
pub mod audio;
pub mod clipfiles;
#[cfg(all(feature = "playback", not(target_os = "linux")))]
pub mod playback;
/// Linux 上没有声音输出（不是主控端平台），保留同样的接口，打开时报不支持。
#[cfg(all(feature = "playback", target_os = "linux"))]
pub mod playback {
    use crate::audio::PcmOutput;
    use std::sync::Arc;

    pub struct DevicePlayer;

    impl DevicePlayer {
        pub fn start(_volume: f32) -> Result<Arc<Self>, String> {
            Err("这个平台不支持播放声音".into())
        }
        pub fn set_volume(&self, _v: f32) {}
        pub fn set_muted(&self, _m: bool) {}
        pub fn stop(&self) {}
    }

    impl PcmOutput for DevicePlayer {
        fn write(&self, _pcm: &[f32]) {}
    }
}
pub mod record;
pub mod ai;
pub mod api;
pub mod connector;
pub mod error;
pub mod identity;
pub mod session;
pub mod signal;
pub mod ssh;
pub mod update;
pub mod vault;
pub mod wake;

pub use error::{ClientError, Result};
