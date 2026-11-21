//! 通用帧读写（任意 AsyncRead / AsyncWrite：QUIC 流或子进程管道）。

use galaxy_proto::MAX_FRAME;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub async fn write<W: AsyncWrite + Unpin, T: Serialize>(w: &mut W, v: &T) -> anyhow::Result<()> {
    w.write_all(&galaxy_proto::frame::encode(v)).await?;
    Ok(())
}

pub async fn read<R: AsyncRead + Unpin, T: DeserializeOwned>(r: &mut R) -> anyhow::Result<Option<T>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let n = u32::from_le_bytes(len) as usize;
    galaxy_proto::frame::check_len(n, MAX_FRAME)?;
    let mut body = vec![0u8; n];
    r.read_exact(&mut body).await?;
    Ok(Some(galaxy_proto::frame::decode_body(&body)?))
}
