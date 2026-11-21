//! OPAQUE（RFC 9807）封装：Ristretto255 + TripleDH + Argon2id。
//! 客户端和服务器只交换不透明的字节串。

use crate::{CryptoError, Result};
use opaque_ke::{
    CipherSuite, ClientLogin, ClientLoginFinishParameters, ClientRegistration,
    ClientRegistrationFinishParameters, CredentialFinalization, CredentialRequest,
    CredentialResponse, RegistrationRequest, RegistrationResponse, RegistrationUpload, ServerLogin,
    ServerLoginParameters, ServerRegistration, ServerSetup,
};
use rand::rngs::OsRng;

pub struct Suite;

impl CipherSuite for Suite {
    type OprfCs = opaque_ke::Ristretto255;
    type KeyExchange = opaque_ke::TripleDh<opaque_ke::Ristretto255, sha2::Sha512>;
    type Ksf = argon2::Argon2<'static>;
}

fn err<E: std::fmt::Debug>(e: E) -> CryptoError {
    CryptoError::Opaque(format!("{e:?}"))
}

/// 凭据标识：用户名小写。
pub fn credential_id(username: &str) -> Vec<u8> {
    username.trim().to_lowercase().into_bytes()
}

// ---------------- 服务器 ----------------

pub struct OpaqueServer {
    setup: ServerSetup<Suite>,
}

impl OpaqueServer {
    pub fn generate() -> Self {
        Self { setup: ServerSetup::<Suite>::new(&mut OsRng) }
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        Ok(Self { setup: ServerSetup::<Suite>::deserialize(b).map_err(err)? })
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        self.setup.serialize().to_vec()
    }

    pub fn register_start(&self, request: &[u8], username: &str) -> Result<Vec<u8>> {
        let req = RegistrationRequest::<Suite>::deserialize(request).map_err(err)?;
        let r = ServerRegistration::<Suite>::start(&self.setup, req, &credential_id(username)).map_err(err)?;
        Ok(r.message.serialize().to_vec())
    }

    /// 返回要存进数据库的注册记录。
    pub fn register_finish(&self, upload: &[u8]) -> Result<Vec<u8>> {
        let up = RegistrationUpload::<Suite>::deserialize(upload).map_err(err)?;
        Ok(ServerRegistration::<Suite>::finish(up).serialize().to_vec())
    }

    /// 登录第一步。`record` 为 None（用户不存在）时仍返回看起来正常的响应，防止枚举用户名。
    pub fn login_start(&self, record: Option<&[u8]>, request: &[u8], username: &str) -> Result<(Vec<u8>, Vec<u8>)> {
        let rec = match record {
            Some(r) => Some(ServerRegistration::<Suite>::deserialize(r).map_err(err)?),
            None => None,
        };
        let req = CredentialRequest::<Suite>::deserialize(request).map_err(err)?;
        let r = ServerLogin::start(
            &mut OsRng,
            &self.setup,
            rec,
            req,
            &credential_id(username),
            ServerLoginParameters::default(),
        )
        .map_err(err)?;
        Ok((r.state.serialize().to_vec(), r.message.serialize().to_vec()))
    }

    /// 登录第二步，成功返回会话密钥（双方一致）。
    pub fn login_finish(&self, state: &[u8], finalization: &[u8]) -> Result<Vec<u8>> {
        let st = ServerLogin::<Suite>::deserialize(state).map_err(err)?;
        let fin = CredentialFinalization::<Suite>::deserialize(finalization).map_err(err)?;
        let r = st.finish(fin, ServerLoginParameters::default()).map_err(err)?;
        Ok(r.session_key.to_vec())
    }

    /// 服务器代替客户端完成一次注册（管理员设初始密码时用），返回注册记录。密码用完即丢。
    pub fn register_with_password(&self, username: &str, password: &str) -> Result<Vec<u8>> {
        let (st, req) = client_register_start(password)?;
        let resp = self.register_start(&req, username)?;
        let (upload, _export) = client_register_finish(&st, password, &resp)?;
        self.register_finish(&upload)
    }
}

// ---------------- 客户端 ----------------

pub fn client_register_start(password: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    let r = ClientRegistration::<Suite>::start(&mut OsRng, password.as_bytes()).map_err(err)?;
    Ok((r.state.serialize().to_vec(), r.message.serialize().to_vec()))
}

/// 返回 (上传给服务器的注册数据, export_key)。
pub fn client_register_finish(state: &[u8], password: &str, response: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let st = ClientRegistration::<Suite>::deserialize(state).map_err(err)?;
    let resp = RegistrationResponse::<Suite>::deserialize(response).map_err(err)?;
    let r = st
        .finish(&mut OsRng, password.as_bytes(), resp, ClientRegistrationFinishParameters::default())
        .map_err(err)?;
    Ok((r.message.serialize().to_vec(), r.export_key.to_vec()))
}

pub fn client_login_start(password: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    let r = ClientLogin::<Suite>::start(&mut OsRng, password.as_bytes()).map_err(err)?;
    Ok((r.state.serialize().to_vec(), r.message.serialize().to_vec()))
}

pub struct ClientLoginOutput {
    pub finalization: Vec<u8>,
    pub export_key: Vec<u8>,
    pub session_key: Vec<u8>,
}

/// 密码错误时这里会失败（不需要服务器参与就能判断）。
pub fn client_login_finish(state: &[u8], password: &str, response: &[u8]) -> Result<ClientLoginOutput> {
    let st = ClientLogin::<Suite>::deserialize(state).map_err(err)?;
    let resp = CredentialResponse::<Suite>::deserialize(response).map_err(err)?;
    let r = st
        .finish(&mut OsRng, password.as_bytes(), resp, ClientLoginFinishParameters::default())
        .map_err(err)?;
    Ok(ClientLoginOutput {
        finalization: r.message.serialize().to_vec(),
        export_key: r.export_key.to_vec(),
        session_key: r.session_key.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_flow() {
        let server = OpaqueServer::generate();
        let server = OpaqueServer::from_bytes(&server.to_bytes()).unwrap();
        let record = server.register_with_password("Alice", "init-pass").unwrap();

        // 正确密码
        let (cst, req) = client_login_start("init-pass").unwrap();
        let (sst, resp) = server.login_start(Some(&record), &req, "alice").unwrap();
        let out = client_login_finish(&cst, "init-pass", &resp).unwrap();
        let sk = server.login_finish(&sst, &out.finalization).unwrap();
        assert_eq!(sk, out.session_key);

        // export_key 每次登录一致
        let (cst2, req2) = client_login_start("init-pass").unwrap();
        let (_s2, resp2) = server.login_start(Some(&record), &req2, "alice").unwrap();
        let out2 = client_login_finish(&cst2, "init-pass", &resp2).unwrap();
        assert_eq!(out.export_key, out2.export_key);

        // 错误密码
        let (cst3, req3) = client_login_start("wrong").unwrap();
        let (_s3, resp3) = server.login_start(Some(&record), &req3, "alice").unwrap();
        assert!(client_login_finish(&cst3, "wrong", &resp3).is_err());

        // 用户不存在也有响应
        let (cst4, req4) = client_login_start("x").unwrap();
        let (_s4, resp4) = server.login_start(None, &req4, "nobody").unwrap();
        assert!(client_login_finish(&cst4, "x", &resp4).is_err());
    }
}
