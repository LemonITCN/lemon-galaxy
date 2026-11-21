//! 服务器密钥：TLS 证书、签名密钥、主密钥、OPAQUE 服务器参数。首次启动生成，权限 0600。

use galaxy_crypto::opaque::OpaqueServer;
use galaxy_crypto::SignKey;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct Keys {
    pub sign: SignKey,
    pub master: [u8; 32],
    pub opaque: OpaqueServer,
    pub tls: Arc<rustls::ServerConfig>,
    pub tls_cert_der: Vec<u8>,
    pub fingerprint: String,
}

fn write_secret(path: &Path, data: &[u8]) -> anyhow::Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(path, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn load_or<F: FnOnce() -> Vec<u8>>(path: &Path, gen: F) -> anyhow::Result<Vec<u8>> {
    if path.exists() {
        Ok(std::fs::read(path)?)
    } else {
        let v = gen();
        write_secret(path, &v)?;
        Ok(v)
    }
}

pub struct TlsSource {
    pub cert_path: Option<PathBuf>,
    pub key_path: Option<PathBuf>,
}

impl Keys {
    pub fn load(data_dir: &Path, public_host: &str, tls_src: &TlsSource) -> anyhow::Result<Self> {
        let kd = data_dir.join("keys");
        std::fs::create_dir_all(&kd)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&kd, std::fs::Permissions::from_mode(0o700));
        }

        let sign_bytes = load_or(&kd.join("sign.key"), || SignKey::generate().to_bytes().to_vec())?;
        let sign = SignKey::from_bytes(&sign_bytes)?;
        let master_v = load_or(&kd.join("master.key"), || galaxy_crypto::random_bytes::<32>().to_vec())?;
        let master: [u8; 32] = master_v.as_slice().try_into().map_err(|_| anyhow::anyhow!("master.key 损坏"))?;
        let opaque_v = load_or(&kd.join("opaque.bin"), || OpaqueServer::generate().to_bytes())?;
        let opaque = OpaqueServer::from_bytes(&opaque_v)?;

        let (cert_pem, key_pem) = match (&tls_src.cert_path, &tls_src.key_path) {
            (Some(c), Some(k)) => (std::fs::read(c)?, std::fs::read(k)?),
            _ => {
                let cp = kd.join("tls-cert.pem");
                let kp = kd.join("tls-key.pem");
                if !cp.exists() || !kp.exists() {
                    let mut names = vec!["localhost".to_string(), "127.0.0.1".to_string(), "::1".to_string()];
                    if !public_host.is_empty() && !names.contains(&public_host.to_string()) {
                        names.insert(0, public_host.to_string());
                    }
                    let ck = rcgen::generate_simple_self_signed(names)?;
                    write_secret(&cp, ck.cert.pem().as_bytes())?;
                    write_secret(&kp, ck.key_pair.serialize_pem().as_bytes())?;
                }
                (std::fs::read(&cp)?, std::fs::read(&kp)?)
            }
        };

        let certs: Vec<rustls::pki_types::CertificateDer<'static>> =
            rustls_pemfile::certs(&mut cert_pem.as_slice()).collect::<Result<_, _>>()?;
        let key = rustls_pemfile::private_key(&mut key_pem.as_slice())?
            .ok_or_else(|| anyhow::anyhow!("TLS 私钥文件里没有私钥"))?;
        let leaf = certs.first().ok_or_else(|| anyhow::anyhow!("TLS 证书文件为空"))?.to_vec();
        let fingerprint = galaxy_crypto::cert_fingerprint(&leaf);

        let mut cfg = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])?
            .with_no_client_auth()
            .with_single_cert(certs, key)?;
        cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

        Ok(Self { sign, master, opaque, tls: Arc::new(cfg), tls_cert_der: leaf, fingerprint })
    }

    /// 用主密钥加密数据库里的敏感字段（API Key、TOTP 密钥）。格式：nonce(24) || 密文。
    pub fn seal(&self, plain: &[u8], purpose: &str) -> Vec<u8> {
        let (mut n, c) = galaxy_crypto::seal(&self.master, plain, purpose.as_bytes());
        n.extend_from_slice(&c);
        n
    }

    pub fn open(&self, blob: &[u8], purpose: &str) -> Option<Vec<u8>> {
        if blob.len() < 24 {
            return None;
        }
        galaxy_crypto::open(&self.master, &blob[..24], &blob[24..], purpose.as_bytes()).ok()
    }
}
