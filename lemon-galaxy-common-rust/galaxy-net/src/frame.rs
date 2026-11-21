//! QUIC 流上的帧读写：4 字节小端长度 + postcard。

use galaxy_proto::MAX_FRAME;
use serde::de::DeserializeOwned;
use serde::Serialize;

pub async fn write<T: Serialize>(s: &mut quinn::SendStream, v: &T) -> anyhow::Result<()> {
    let buf = galaxy_proto::frame::encode(v);
    s.write_all(&buf).await?;
    Ok(())
}

/// 读一帧；对方正常关闭流返回 None。
pub async fn read<T: DeserializeOwned>(r: &mut quinn::RecvStream) -> anyhow::Result<Option<T>> {
    read_max(r, MAX_FRAME).await
}

pub async fn read_max<T: DeserializeOwned>(r: &mut quinn::RecvStream, max: usize) -> anyhow::Result<Option<T>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len).await {
        Ok(()) => {}
        Err(quinn::ReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let n = u32::from_le_bytes(len) as usize;
    galaxy_proto::frame::check_len(n, max)?;
    let mut body = vec![0u8; n];
    r.read_exact(&mut body).await?;
    Ok(Some(galaxy_proto::frame::decode_body(&body)?))
}
