//! 服务器 HTTP 接口客户端（固定证书指纹）。

use crate::error::{ClientError, Result};
use galaxy_net::ServerAddr;
use galaxy_proto::api::ErrorBody;
use parking_lot::RwLock;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct Api {
    pub server: ServerAddr,
    http: reqwest::Client,
    token: Arc<RwLock<Option<String>>>,
    lang: String,
}

impl Api {
    pub fn new(server: ServerAddr, lang: &str) -> Result<Self> {
        galaxy_net::init_crypto();
        let mut b = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .user_agent(format!("LemonGalaxy/{}", galaxy_proto::APP_VERSION));
        if server.fingerprint.is_some() {
            let cfg = galaxy_net::tls::server_client_config(server.fingerprint, &[b"h2", b"http/1.1"]);
            b = b.use_preconfigured_tls(cfg);
        } else {
            b = b.use_rustls_tls();
        }
        Ok(Self { server, http: b.build()?, token: Default::default(), lang: lang.to_string() })
    }

    pub fn set_token(&self, t: Option<String>) {
        *self.token.write() = t;
    }

    pub fn token(&self) -> Option<String> {
        self.token.read().clone()
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    fn req(&self, m: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut r = self.http.request(m, format!("{}/api/v1{}", self.server.base_url(), path)).header("accept-language", &self.lang);
        if let Some(t) = self.token.read().as_ref() {
            r = r.bearer_auth(t);
        }
        r
    }

    async fn parse<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T> {
        let status = resp.status();
        let bytes = resp.bytes().await?;
        if status.is_success() {
            return serde_json::from_slice(&bytes).map_err(|e| ClientError::Protocol(format!("响应格式错误: {e}")));
        }
        let (code, message) = match serde_json::from_slice::<ErrorBody>(&bytes) {
            Ok(b) => (b.error.code, b.error.message),
            Err(_) => (format!("http_{}", status.as_u16()), String::from_utf8_lossy(&bytes).chars().take(200).collect()),
        };
        Err(match status.as_u16() {
            401 | 423 => ClientError::Auth { code, message },
            403 if code == "must_change_password" || code == "disabled" => ClientError::Auth { code, message },
            403 => ClientError::Permission(message),
            404 => ClientError::NotFound(message),
            409 => ClientError::Auth { code, message },
            _ => ClientError::Other(message),
        })
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        Self::parse(self.req(reqwest::Method::GET, path).send().await?).await
    }

    pub async fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        Self::parse(self.req(reqwest::Method::POST, path).json(body).send().await?).await
    }

    pub async fn put<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        Self::parse(self.req(reqwest::Method::PUT, path).json(body).send().await?).await
    }

    pub async fn delete<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        Self::parse(self.req(reqwest::Method::DELETE, path).send().await?).await
    }

    /// 流式 GET（下载文件）。`path` 可以是 /api/v1 下的相对路径，也可以是以 /api/ 开头的完整路径。
    pub async fn get_stream(&self, path: &str) -> Result<reqwest::Response> {
        let p = path.strip_prefix("/api/v1").unwrap_or(path);
        let resp = self.req(reqwest::Method::GET, p).timeout(Duration::from_secs(3600)).send().await?;
        if resp.status().is_success() {
            Ok(resp)
        } else {
            Err(Self::parse::<serde_json::Value>(resp).await.err().unwrap_or(ClientError::Other("请求失败".into())))
        }
    }

    /// 流式 POST（AI 网关 SSE）。返回原始响应，由调用方逐块读取。
    pub async fn post_stream<B: Serialize>(&self, path: &str, body: &B) -> Result<reqwest::Response> {
        let resp = self.req(reqwest::Method::POST, path).timeout(Duration::from_secs(900)).json(body).send().await?;
        if resp.status().is_success() {
            Ok(resp)
        } else {
            Err(Self::parse::<serde_json::Value>(resp).await.err().unwrap_or(ClientError::Other("请求失败".into())))
        }
    }
}
