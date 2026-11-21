//! 端到端认证握手（docs/security.md 第 5 节）。
//!
//! 双方在主控端打开的第一条双向流上互发 Hello：设备证书（账号签名）+ 用设备密钥对 TLS exporter 的签名。
//! 临时协助模式下被控端没有证书，改用以验证码为口令的 SPAKE2 认证，同样绑定 exporter。

use crate::frame;
use galaxy_crypto::{AssistPake, SignKey};
use galaxy_proto::cert::{DeviceCert, SignedBlob};
use galaxy_proto::peer::{AssistAuth, HandshakeResult, Hello, HelloAck, PeerRole};
use galaxy_proto::PROTOCOL_VERSION;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum HandshakeError {
    #[error("对方拒绝：{0}")]
    Rejected(String),
    #[error("认证失败：{0}")]
    Auth(String),
    #[error("连接错误：{0}")]
    Io(String),
}

impl From<anyhow::Error> for HandshakeError {
    fn from(e: anyhow::Error) -> Self {
        HandshakeError::Io(e.to_string())
    }
}

pub struct AssistSecret {
    pub code: String,
    pub device_code: String,
}

pub struct Established {
    pub send: quinn::SendStream,
    pub recv: quinn::RecvStream,
    pub ack: HelloAck,
    pub peer_pubkey: [u8; 32],
    pub peer_cert: Option<DeviceCert>,
}

fn make_hello(key: &SignKey, exporter: &[u8], role: PeerRole, cert: Option<SignedBlob>, ticket: Option<SignedBlob>, assist: bool) -> Hello {
    Hello {
        proto: PROTOCOL_VERSION,
        role,
        app_version: galaxy_proto::APP_VERSION.into(),
        cert,
        device_pubkey: key.public(),
        sig: galaxy_crypto::session_auth_sign(key, exporter, role),
        ticket,
        assist,
    }
}

async fn spake(send: &mut quinn::SendStream, recv: &mut quinn::RecvStream, secret: &AssistSecret, exporter: &[u8], me: PeerRole) -> Result<(), HandshakeError> {
    let (st, msg) = AssistPake::start(&secret.code, &secret.device_code);
    frame::write(send, &AssistAuth::Spake(msg)).await?;
    let peer: AssistAuth = frame::read(recv).await?.ok_or_else(|| HandshakeError::Io("流已关闭".into()))?;
    let AssistAuth::Spake(pm) = peer else { return Err(HandshakeError::Auth("协助认证消息错误".into())) };
    let shared = st.finish(&pm).map_err(|_| HandshakeError::Auth("验证码错误".into()))?;
    frame::write(send, &AssistAuth::Confirm(galaxy_crypto::assist_confirm(&shared, exporter, me))).await?;
    let other = if me == PeerRole::Host { PeerRole::Controller } else { PeerRole::Host };
    let peer: AssistAuth = frame::read(recv).await?.ok_or_else(|| HandshakeError::Io("流已关闭".into()))?;
    match peer {
        AssistAuth::Confirm(c) if galaxy_crypto::ct_eq(&c, &galaxy_crypto::assist_confirm(&shared, exporter, other)) => Ok(()),
        AssistAuth::Failed { reason } => Err(HandshakeError::Auth(reason)),
        _ => Err(HandshakeError::Auth("验证码错误".into())),
    }
}

/// 主控端握手。`verify_host` 校验对方 Hello（正常模式下校验证书）。
pub async fn controller<F>(
    conn: &quinn::Connection,
    key: &SignKey,
    cert: Option<SignedBlob>,
    ticket: Option<SignedBlob>,
    assist: Option<AssistSecret>,
    verify_host: F,
) -> Result<Established, HandshakeError>
where
    F: FnOnce(&Hello) -> Result<Option<DeviceCert>, String>,
{
    let exporter = crate::quic::exporter(conn)?;
    let (mut send, mut recv) = conn.open_bi().await.map_err(|e| HandshakeError::Io(e.to_string()))?;
    frame::write(&mut send, &make_hello(key, &exporter, PeerRole::Controller, cert, ticket, assist.is_some())).await?;
    let host: Hello = tokio::time::timeout(Duration::from_secs(15), frame::read(&mut recv))
        .await
        .map_err(|_| HandshakeError::Io("等待对方超时".into()))??
        .ok_or_else(|| HandshakeError::Io("对方关闭了连接".into()))?;
    if host.role != PeerRole::Host {
        return Err(HandshakeError::Auth("角色错误".into()));
    }
    galaxy_crypto::session_auth_verify(&host.device_pubkey, &exporter, PeerRole::Host, &host.sig).map_err(|_| HandshakeError::Auth("对方签名无效".into()))?;
    let peer_cert = verify_host(&host).map_err(HandshakeError::Auth)?;
    if let Some(a) = &assist {
        spake(&mut send, &mut recv, a, &exporter, PeerRole::Controller).await?;
    }
    // 协助模式下要等被控端本人点允许，最多等 2 分钟
    let res: HandshakeResult = tokio::time::timeout(Duration::from_secs(130), frame::read(&mut recv))
        .await
        .map_err(|_| HandshakeError::Rejected("对方未响应".into()))??
        .ok_or_else(|| HandshakeError::Io("对方关闭了连接".into()))?;
    match res {
        HandshakeResult::Ok(ack) => Ok(Established { send, recv, ack, peer_pubkey: host.device_pubkey, peer_cert }),
        HandshakeResult::Rejected { message, .. } => Err(HandshakeError::Rejected(message)),
    }
}

/// 被控端收到主控端 Hello 后，由上层决定是否接受。
pub enum HostVerdict {
    /// 正常模式：用这张证书回应（与对方同账号），之后发送 ack
    Accept { my_cert: SignedBlob, ack: HelloAck },
    /// 协助模式：先做口令认证，然后由 `decide` 决定
    Assist { secret: AssistSecret },
    Reject { code: String, message: String },
}

pub struct HostSession {
    pub send: quinn::SendStream,
    pub recv: quinn::RecvStream,
    pub hello: Hello,
    pub exporter: [u8; 32],
}

/// 被控端：接受控制流，读对方 Hello 并校验签名。
pub async fn host_accept(conn: &quinn::Connection) -> Result<HostSession, HandshakeError> {
    let exporter = crate::quic::exporter(conn)?;
    let (send, mut recv) = tokio::time::timeout(Duration::from_secs(15), conn.accept_bi())
        .await
        .map_err(|_| HandshakeError::Io("等待控制流超时".into()))?
        .map_err(|e| HandshakeError::Io(e.to_string()))?;
    let hello: Hello = tokio::time::timeout(Duration::from_secs(15), frame::read(&mut recv))
        .await
        .map_err(|_| HandshakeError::Io("等待对方超时".into()))??
        .ok_or_else(|| HandshakeError::Io("对方关闭了连接".into()))?;
    if hello.role != PeerRole::Controller || hello.proto != PROTOCOL_VERSION {
        return Err(HandshakeError::Auth("协议版本或角色不匹配".into()));
    }
    galaxy_crypto::session_auth_verify(&hello.device_pubkey, &exporter, PeerRole::Controller, &hello.sig).map_err(|_| HandshakeError::Auth("对方签名无效".into()))?;
    Ok(HostSession { send, recv, hello, exporter })
}

/// 被控端：按裁决完成握手。协助模式下 `decide` 在口令认证通过后调用（通常弹窗询问本人），返回 ack 或拒绝原因。
pub async fn host_finish<D, Fut>(mut s: HostSession, key: &SignKey, verdict: HostVerdict, decide: D) -> Result<(quinn::SendStream, quinn::RecvStream, Hello), HandshakeError>
where
    D: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<HelloAck, String>>,
{
    match verdict {
        HostVerdict::Reject { code, message } => {
            // 先回一个 Hello 让对方能读到拒绝原因
            frame::write(&mut s.send, &make_hello(key, &s.exporter, PeerRole::Host, None, None, false)).await?;
            frame::write(&mut s.send, &HandshakeResult::Rejected { code, message: message.clone() }).await?;
            let _ = s.send.finish();
            Err(HandshakeError::Rejected(message))
        }
        HostVerdict::Accept { my_cert, ack } => {
            frame::write(&mut s.send, &make_hello(key, &s.exporter, PeerRole::Host, Some(my_cert), None, false)).await?;
            frame::write(&mut s.send, &HandshakeResult::Ok(ack)).await?;
            Ok((s.send, s.recv, s.hello))
        }
        HostVerdict::Assist { secret } => {
            frame::write(&mut s.send, &make_hello(key, &s.exporter, PeerRole::Host, None, None, true)).await?;
            let exporter = s.exporter;
            spake(&mut s.send, &mut s.recv, &secret, &exporter, PeerRole::Host).await?;
            match decide().await {
                Ok(ack) => {
                    frame::write(&mut s.send, &HandshakeResult::Ok(ack)).await?;
                    Ok((s.send, s.recv, s.hello))
                }
                Err(msg) => {
                    frame::write(&mut s.send, &HandshakeResult::Rejected { code: "denied".into(), message: msg.clone() }).await?;
                    let _ = s.send.finish();
                    Err(HandshakeError::Rejected(msg))
                }
            }
        }
    }
}
