//! 帧格式：4 字节小端长度 + postcard 负载。异步读写在 galaxy-net。

use serde::de::DeserializeOwned;
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("frame too large: {0}")]
    TooLarge(usize),
    #[error("decode: {0}")]
    Decode(String),
}

pub fn encode<T: Serialize>(msg: &T) -> Vec<u8> {
    let body = postcard::to_stdvec(msg).expect("postcard encode");
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

pub fn decode_body<T: DeserializeOwned>(body: &[u8]) -> Result<T, FrameError> {
    postcard::from_bytes(body).map_err(|e| FrameError::Decode(e.to_string()))
}

pub fn check_len(len: usize, max: usize) -> Result<(), FrameError> {
    if len > max {
        Err(FrameError::TooLarge(len))
    } else {
        Ok(())
    }
}
