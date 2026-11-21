//! TLS 监听器：后台任务完成握手，axum 只拿到握手完成的连接，慢客户端不会卡住接入。

use axum::extract::connect_info::Connected;
use axum::serve::IncomingStream;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;

pub struct TlsListener {
    rx: mpsc::Receiver<(TlsStream<TcpStream>, SocketAddr)>,
    local: SocketAddr,
}

impl TlsListener {
    pub fn new(tcp: TcpListener, cfg: Arc<rustls::ServerConfig>) -> std::io::Result<Self> {
        let local = tcp.local_addr()?;
        let (tx, rx) = mpsc::channel(256);
        let acceptor = TlsAcceptor::from(cfg);
        tokio::spawn(async move {
            loop {
                let (sock, addr) = match tcp.accept().await {
                    Ok(x) => x,
                    Err(e) => {
                        tracing::warn!("TCP accept: {e}");
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                };
                let _ = sock.set_nodelay(true);
                let acceptor = acceptor.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    match tokio::time::timeout(Duration::from_secs(10), acceptor.accept(sock)).await {
                        Ok(Ok(s)) => {
                            let _ = tx.send((s, addr)).await;
                        }
                        Ok(Err(e)) => tracing::debug!("TLS 握手失败 {addr}: {e}"),
                        Err(_) => tracing::debug!("TLS 握手超时 {addr}"),
                    }
                });
            }
        });
        Ok(Self { rx, local })
    }
}

impl axum::serve::Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            if let Some(x) = self.rx.recv().await {
                return x;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        Ok(self.local)
    }
}

/// 请求的对端地址（ConnectInfo）。
#[derive(Clone, Copy, Debug)]
pub struct PeerAddr(pub SocketAddr);

impl Connected<IncomingStream<'_, TlsListener>> for PeerAddr {
    fn connect_info(stream: IncomingStream<'_, TlsListener>) -> Self {
        PeerAddr(*stream.remote_addr())
    }
}
