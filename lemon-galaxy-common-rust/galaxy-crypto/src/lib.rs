//! LemonGalaxy 密码学。设计见 docs/security.md。

pub mod opaque;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use galaxy_proto::cert::{
    ConnTicket, DeviceCert, RevocationList, SignedBlob, CONN_TICKET_DOMAIN, DEVICE_CERT_DOMAIN,
    REVOCATION_DOMAIN, SESSION_AUTH_DOMAIN,
};
use galaxy_proto::peer::PeerRole;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("签名无效")]
    BadSignature,
    #[error("解密失败")]
    Decrypt,
    #[error("数据格式错误")]
    Malformed,
    #[error("OPAQUE: {0}")]
    Opaque(String),
    #[error("口令认证失败")]
    Pake,
}

pub type Result<T> = std::result::Result<T, CryptoError>;

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    rand::rngs::OsRng.fill_bytes(&mut b);
    b
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

pub fn hkdf32(ikm: &[u8], salt: &[u8], info: &[u8]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(Some(salt), ikm);
    let mut out = [0u8; 32];
    hk.expand(info, &mut out).expect("hkdf 32");
    out
}

pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("hmac key");
    m.update(data);
    m.finalize().into_bytes().into()
}

pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.len() == b.len() && a.ct_eq(b).into()
}

fn domain_msg(domain: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(domain.len() + msg.len());
    v.extend_from_slice(domain);
    v.extend_from_slice(msg);
    v
}

// ---------------- Ed25519 ----------------

/// 设备身份密钥 / 账号签名密钥 / 服务器签名密钥都是 Ed25519。
#[derive(Clone)]
pub struct SignKey(SigningKey);

impl SignKey {
    pub fn generate() -> Self {
        Self(SigningKey::generate(&mut rand::rngs::OsRng))
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        let a: [u8; 32] = b.try_into().map_err(|_| CryptoError::Malformed)?;
        Ok(Self(SigningKey::from_bytes(&a)))
    }
    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }
    pub fn public(&self) -> [u8; 32] {
        self.0.verifying_key().to_bytes()
    }
    pub fn sign(&self, domain: &[u8], msg: &[u8]) -> Vec<u8> {
        self.0.sign(&domain_msg(domain, msg)).to_bytes().to_vec()
    }
}

impl std::fmt::Debug for SignKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SignKey(pub={})", hex::encode(&self.public()[..6]))
    }
}

pub fn verify(pubkey: &[u8], domain: &[u8], msg: &[u8], sig: &[u8]) -> Result<()> {
    let pk: [u8; 32] = pubkey.try_into().map_err(|_| CryptoError::Malformed)?;
    let vk = VerifyingKey::from_bytes(&pk).map_err(|_| CryptoError::Malformed)?;
    let s: [u8; 64] = sig.try_into().map_err(|_| CryptoError::Malformed)?;
    vk.verify(&domain_msg(domain, msg), &Signature::from_bytes(&s))
        .map_err(|_| CryptoError::BadSignature)
}

// ---------------- 账号机密 ----------------

/// 账号签名密钥 + 加密库密钥。只有知道账号密码的客户端能解开。
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct AccountSecrets {
    pub sign_key: [u8; 32],
    pub vault_key: [u8; 32],
}

impl std::fmt::Debug for AccountSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AccountSecrets(..)")
    }
}

const ACCOUNT_AAD: &[u8] = b"lemon-galaxy/account-secrets/v1";

impl AccountSecrets {
    pub fn generate() -> Self {
        Self { sign_key: random_bytes(), vault_key: random_bytes() }
    }
    pub fn signer(&self) -> SignKey {
        SignKey(SigningKey::from_bytes(&self.sign_key))
    }
    pub fn public(&self) -> [u8; 32] {
        self.signer().public()
    }

    fn kek(export_key: &[u8], account_id: u64) -> [u8; 32] {
        hkdf32(export_key, &account_id.to_be_bytes(), b"lemon-galaxy/account-kek/v1")
    }

    fn aad(account_id: u64, epoch: u32) -> Vec<u8> {
        let mut a = ACCOUNT_AAD.to_vec();
        a.extend_from_slice(&account_id.to_be_bytes());
        a.extend_from_slice(&epoch.to_be_bytes());
        a
    }

    /// 用 OPAQUE export_key 包裹，返回 (nonce, ciphertext)。
    pub fn wrap(&self, export_key: &[u8], account_id: u64, epoch: u32) -> (Vec<u8>, Vec<u8>) {
        let kek = Self::kek(export_key, account_id);
        let plain = postcard::to_stdvec(self).expect("postcard");
        let (n, c) = seal(&kek, &plain, &Self::aad(account_id, epoch));
        (n, c)
    }

    pub fn unwrap(export_key: &[u8], account_id: u64, epoch: u32, nonce: &[u8], ct: &[u8]) -> Result<Self> {
        let kek = Self::kek(export_key, account_id);
        let mut plain = open(&kek, nonce, ct, &Self::aad(account_id, epoch))?;
        let s: Self = postcard::from_bytes(&plain).map_err(|_| CryptoError::Malformed)?;
        plain.zeroize();
        Ok(s)
    }

    /// 用本机设备密钥派生的密钥包裹，存进本地安全存储。
    pub fn wrap_local(&self, local_key: &[u8; 32]) -> Vec<u8> {
        let plain = postcard::to_stdvec(self).expect("postcard");
        let (mut n, c) = seal(local_key, &plain, b"lemon-galaxy/account-local/v1");
        n.extend_from_slice(&c);
        n
    }
    pub fn unwrap_local(local_key: &[u8; 32], blob: &[u8]) -> Result<Self> {
        if blob.len() < 24 {
            return Err(CryptoError::Malformed);
        }
        let mut plain = open(local_key, &blob[..24], &blob[24..], b"lemon-galaxy/account-local/v1")?;
        let s: Self = postcard::from_bytes(&plain).map_err(|_| CryptoError::Malformed)?;
        plain.zeroize();
        Ok(s)
    }
}

// ---------------- XChaCha20-Poly1305 ----------------

pub fn seal(key: &[u8; 32], plain: &[u8], aad: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce: [u8; 24] = random_bytes();
    let ct = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad })
        .expect("encrypt");
    (nonce.to_vec(), ct)
}

pub fn open(key: &[u8; 32], nonce: &[u8], ct: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
    if nonce.len() != 24 {
        return Err(CryptoError::Malformed);
    }
    let cipher = XChaCha20Poly1305::new(key.into());
    cipher
        .decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad })
        .map_err(|_| CryptoError::Decrypt)
}

/// 加密库条目的附加数据：绑定用户、条目 ID、类型，防止服务器调换密文。
pub fn vault_aad(user_id: u64, item_id: &str, kind: &str) -> Vec<u8> {
    let mut a = b"lemon-galaxy/vault/v1".to_vec();
    a.extend_from_slice(&user_id.to_be_bytes());
    a.push(0);
    a.extend_from_slice(item_id.as_bytes());
    a.push(0);
    a.extend_from_slice(kind.as_bytes());
    a
}

// ---------------- 设备证书 / 票据 / 吊销 ----------------

pub fn sign_device_cert(account: &AccountSecrets, cert: &DeviceCert) -> SignedBlob {
    let data = cert.to_bytes();
    let sig = account.signer().sign(DEVICE_CERT_DOMAIN, &data);
    SignedBlob { data, sig }
}

pub fn verify_device_cert(account_pubkey: &[u8], blob: &SignedBlob) -> Result<DeviceCert> {
    verify(account_pubkey, DEVICE_CERT_DOMAIN, &blob.data, &blob.sig)?;
    DeviceCert::from_bytes(&blob.data).ok_or(CryptoError::Malformed)
}

pub fn sign_ticket(server_key: &SignKey, t: &ConnTicket) -> SignedBlob {
    let data = t.to_bytes();
    let sig = server_key.sign(CONN_TICKET_DOMAIN, &data);
    SignedBlob { data, sig }
}

pub fn verify_ticket(server_pubkey: &[u8], blob: &SignedBlob) -> Result<ConnTicket> {
    verify(server_pubkey, CONN_TICKET_DOMAIN, &blob.data, &blob.sig)?;
    ConnTicket::from_bytes(&blob.data).ok_or(CryptoError::Malformed)
}

pub fn sign_revocations(account: &AccountSecrets, r: &RevocationList) -> SignedBlob {
    let data = r.to_bytes();
    let sig = account.signer().sign(REVOCATION_DOMAIN, &data);
    SignedBlob { data, sig }
}

pub fn verify_revocations(account_pubkey: &[u8], blob: &SignedBlob) -> Result<RevocationList> {
    verify(account_pubkey, REVOCATION_DOMAIN, &blob.data, &blob.sig)?;
    RevocationList::from_bytes(&blob.data).ok_or(CryptoError::Malformed)
}

fn role_byte(role: PeerRole) -> u8 {
    match role {
        PeerRole::Controller => 1,
        PeerRole::Host => 2,
    }
}

/// 端到端会话认证签名：绑定 TLS exporter 与角色。
pub fn session_auth_sign(device: &SignKey, exporter: &[u8], role: PeerRole) -> Vec<u8> {
    let mut m = exporter.to_vec();
    m.push(role_byte(role));
    device.sign(SESSION_AUTH_DOMAIN, &m)
}

pub fn session_auth_verify(device_pubkey: &[u8], exporter: &[u8], role: PeerRole, sig: &[u8]) -> Result<()> {
    let mut m = exporter.to_vec();
    m.push(role_byte(role));
    verify(device_pubkey, SESSION_AUTH_DOMAIN, &m, sig)
}

// ---------------- 局域网发现 ----------------

/// 由账号公钥派生的局域网发现密钥。
pub fn lan_key(account_pubkey: &[u8]) -> [u8; 32] {
    hkdf32(account_pubkey, b"lemon-galaxy/lan/v1", b"")
}

pub fn lan_mac(key: &[u8; 32], input: &[u8]) -> [u8; 16] {
    let full = hmac_sha256(key, input);
    full[..16].try_into().unwrap()
}

/// 两端探测包的连接标签。
pub fn conn_tag(conn_id: &[u8; 16]) -> [u8; 8] {
    let mut v = conn_id.to_vec();
    v.extend_from_slice(b"lemon-galaxy/disco/v1");
    sha256(&v)[..8].try_into().unwrap()
}

// ---------------- 临时协助：SPAKE2 ----------------

pub struct AssistPake {
    inner: spake2::Spake2<spake2::Ed25519Group>,
}

impl AssistPake {
    /// 双方对称启动，口令为被控端屏幕上显示的验证码。返回 (状态, 发给对方的消息)。
    pub fn start(code: &str, device_code: &str) -> (Self, Vec<u8>) {
        let normalized: String = code.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_uppercase();
        let id = format!("lemon-galaxy/assist/v1/{device_code}");
        let (s, msg) = spake2::Spake2::<spake2::Ed25519Group>::start_symmetric(
            &spake2::Password::new(normalized.as_bytes()),
            &spake2::Identity::new(id.as_bytes()),
        );
        (Self { inner: s }, msg)
    }

    /// 收到对方消息后得到共享密钥。
    pub fn finish(self, peer_msg: &[u8]) -> Result<[u8; 32]> {
        let k = self.inner.finish(peer_msg).map_err(|_| CryptoError::Pake)?;
        Ok(sha256(&k))
    }
}

/// 协助模式确认码：双方用共享密钥对 exporter 与自己的角色计算，互相校验。
pub fn assist_confirm(shared: &[u8; 32], exporter: &[u8], role: PeerRole) -> Vec<u8> {
    let mut m = b"lemon-galaxy/assist-confirm/v1".to_vec();
    m.extend_from_slice(exporter);
    m.push(role_byte(role));
    hmac_sha256(shared, &m).to_vec()
}

/// 本机设备密钥派生的本地包裹密钥。
pub fn local_wrap_key(device: &SignKey) -> [u8; 32] {
    hkdf32(&device.to_bytes(), b"lemon-galaxy/local-wrap/v1", b"")
}

/// 服务器 TLS 证书指纹（SHA-256，base64url），客户端地址里的 `#sha256-...`。
pub fn cert_fingerprint(cert_der: &[u8]) -> String {
    format!("sha256-{}", galaxy_proto::b64::encode(&sha256(cert_der)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use galaxy_proto::{DeviceId, DeviceKind};

    #[test]
    fn account_wrap_roundtrip() {
        let s = AccountSecrets::generate();
        let ek = random_bytes::<64>();
        let (n, c) = s.wrap(&ek, 7, 1);
        let back = AccountSecrets::unwrap(&ek, 7, 1, &n, &c).unwrap();
        assert_eq!(back.sign_key, s.sign_key);
        assert!(AccountSecrets::unwrap(&ek, 7, 2, &n, &c).is_err());
        assert!(AccountSecrets::unwrap(&ek, 8, 1, &n, &c).is_err());
    }

    #[test]
    fn device_cert() {
        let acct = AccountSecrets::generate();
        let dev = SignKey::generate();
        let cert = DeviceCert {
            v: 1,
            account_id: 3,
            account_epoch: 1,
            device_id: DeviceId::random(),
            device_pubkey: dev.public(),
            kind: DeviceKind::Worker,
            issued_at: 1,
        };
        let blob = sign_device_cert(&acct, &cert);
        assert_eq!(verify_device_cert(&acct.public(), &blob).unwrap(), cert);
        let other = AccountSecrets::generate();
        assert!(verify_device_cert(&other.public(), &blob).is_err());
    }

    #[test]
    fn session_auth() {
        let dev = SignKey::generate();
        let ex = random_bytes::<32>();
        let sig = session_auth_sign(&dev, &ex, PeerRole::Controller);
        assert!(session_auth_verify(&dev.public(), &ex, PeerRole::Controller, &sig).is_ok());
        assert!(session_auth_verify(&dev.public(), &ex, PeerRole::Host, &sig).is_err());
    }

    #[test]
    fn assist_pake() {
        let (a, ma) = AssistPake::start("AB12 cd34", "123456789");
        let (b, mb) = AssistPake::start("ab12cd34", "123456789");
        let ka = a.finish(&mb).unwrap();
        let kb = b.finish(&ma).unwrap();
        assert_eq!(ka, kb);
        let (c, mc) = AssistPake::start("wrong", "123456789");
        let (d, md) = AssistPake::start("ab12cd34", "123456789");
        assert_ne!(c.finish(&md).unwrap(), d.finish(&mc).unwrap());
    }

    #[test]
    fn local_wrap() {
        let dev = SignKey::generate();
        let k = local_wrap_key(&dev);
        let s = AccountSecrets::generate();
        let b = s.wrap_local(&k);
        assert_eq!(AccountSecrets::unwrap_local(&k, &b).unwrap().vault_key, s.vault_key);
    }
}
