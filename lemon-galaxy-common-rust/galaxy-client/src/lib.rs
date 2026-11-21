//! LemonGalaxy 客户端内核（主控端与被控端共用的服务器交互，以及主控端的连接与会话）。

pub mod account;
pub mod ai;
pub mod api;
pub mod connector;
pub mod error;
pub mod identity;
pub mod session;
pub mod signal;
pub mod ssh;
pub mod vault;

pub use error::{ClientError, Result};
