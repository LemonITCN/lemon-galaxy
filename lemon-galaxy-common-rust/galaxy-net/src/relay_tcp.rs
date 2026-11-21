//! TCP 中转：`wss://服务器/api/v1/relay?token=`，二进制帧即负载。

use crate::addr::ServerAddr;
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::Connector;

pub async fn connect(server: &ServerAddr, token: [u8; 16]) -> anyhow::Result<(mpsc::Sender<Bytes>, mpsc::Receiver<Bytes>)> {
    let url = server.ws_url(&format!("/api/v1/relay?token={}", galaxy_proto::b64::encode(&token)));
    let cfg = crate::tls::server_client_config(server.fingerprint, &[b"http/1.1"]);
    let (ws, _) = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio_tungstenite::connect_async_tls_with_config(url, None, true, Some(Connector::Rustls(Arc::new(cfg)))),
    )
    .await??;
    let (mut sink, mut stream) = ws.split();
    let (out_tx, mut out_rx) = mpsc::channel::<Bytes>(2048);
    let (in_tx, in_rx) = mpsc::channel::<Bytes>(2048);
    tokio::spawn(async move {
        while let Some(b) = out_rx.recv().await {
            if sink.send(Message::Binary(b)).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });
    tokio::spawn(async move {
        while let Some(Ok(m)) = stream.next().await {
            if let Message::Binary(b) = m {
                if in_tx.send(b).await.is_err() {
                    break;
                }
            }
        }
    });
    Ok((out_tx, in_rx))
}
