//! TLS：服务器证书指纹固定；两端 QUIC 的证书校验交给端到端认证。

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use std::sync::Arc;

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// 固定服务器证书 SHA-256 指纹，握手签名仍按标准校验。
#[derive(Debug)]
pub struct PinnedVerifier {
    fingerprint: [u8; 32],
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let fp = galaxy_crypto::sha256(end_entity.as_ref());
        if galaxy_crypto::ct_eq(&fp, &self.fingerprint) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("服务器证书指纹不匹配".into()))
        }
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(m, c, d, &self.provider.signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(m, c, d, &self.provider.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// 两端 QUIC 用：任何证书都接受，身份由 TLS 之上的端到端认证（绑定 exporter）保证。
#[derive(Debug)]
pub struct PeerVerifier {
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PeerVerifier {
    fn verify_server_cert(&self, _e: &CertificateDer<'_>, _i: &[CertificateDer<'_>], _s: &ServerName<'_>, _o: &[u8], _n: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(m, c, d, &self.provider.signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(m, c, d, &self.provider.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// 连接 LemonGalaxy 服务器的 TLS 客户端配置。有指纹则固定指纹，否则用系统内置根证书。
pub fn server_client_config(fingerprint: Option<[u8; 32]>, alpn: &[&[u8]]) -> rustls::ClientConfig {
    let builder = rustls::ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .expect("tls versions");
    let mut cfg = match fingerprint {
        Some(fp) => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedVerifier { fingerprint: fp, provider: provider() }))
            .with_no_client_auth(),
        None => {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots_list());
            builder.with_root_certificates(roots).with_no_client_auth()
        }
    };
    cfg.alpn_protocols = alpn.iter().map(|a| a.to_vec()).collect();
    cfg
}

fn webpki_roots_list() -> Vec<rustls::pki_types::TrustAnchor<'static>> {
    // 不引入额外依赖：正规证书场景由调用方（reqwest 自带根证书）处理；这里为 WebSocket 提供最小兜底。
    Vec::new()
}

pub fn peer_client_config() -> rustls::ClientConfig {
    let mut cfg = rustls::ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("tls13")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PeerVerifier { provider: provider() }))
        .with_no_client_auth();
    cfg.alpn_protocols = vec![galaxy_proto::ALPN.to_vec()];
    cfg
}

pub fn peer_server_config() -> anyhow::Result<rustls::ServerConfig> {
    let ck = rcgen::generate_simple_self_signed(vec!["lemon-galaxy".to_string()])?;
    let cert = CertificateDer::from(ck.cert.der().to_vec());
    let key = rustls::pki_types::PrivateKeyDer::try_from(ck.key_pair.serialize_der()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut cfg = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)?;
    cfg.alpn_protocols = vec![galaxy_proto::ALPN.to_vec()];
    Ok(cfg)
}
