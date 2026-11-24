//! Wayland 后端：GNOME 走 Mutter 自己的远程桌面接口（mutter.rs），其他桌面走 xdg-desktop-portal（portal.rs）；
//! 画面都经 PipeWire 收（pipewire.rs，运行时加载 libpipewire-0.3）。

pub mod mutter;
mod pipewire;
pub mod portal;
mod spa;
