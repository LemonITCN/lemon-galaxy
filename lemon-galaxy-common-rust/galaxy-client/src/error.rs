#[derive(Debug, Clone, thiserror::Error)]
pub enum ClientError {
    #[error("{0}")]
    Network(String),
    #[error("{message}")]
    Auth { code: String, message: String },
    #[error("{0}")]
    Permission(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Protocol(String),
    #[error("已取消")]
    Cancelled,
    #[error("{0}")]
    Other(String),
}

impl ClientError {
    pub fn code(&self) -> &str {
        match self {
            ClientError::Auth { code, .. } => code,
            ClientError::Network(_) => "network",
            ClientError::Permission(_) => "forbidden",
            ClientError::NotFound(_) => "not_found",
            ClientError::Protocol(_) => "protocol",
            ClientError::Cancelled => "cancelled",
            ClientError::Other(_) => "other",
        }
    }
}

impl From<anyhow::Error> for ClientError {
    fn from(e: anyhow::Error) -> Self {
        ClientError::Other(e.to_string())
    }
}

impl From<reqwest::Error> for ClientError {
    fn from(e: reqwest::Error) -> Self {
        ClientError::Network(e.to_string())
    }
}

impl From<galaxy_crypto::CryptoError> for ClientError {
    fn from(e: galaxy_crypto::CryptoError) -> Self {
        ClientError::Protocol(e.to_string())
    }
}

impl From<galaxy_net::handshake::HandshakeError> for ClientError {
    fn from(e: galaxy_net::handshake::HandshakeError) -> Self {
        match e {
            galaxy_net::handshake::HandshakeError::Rejected(m) => ClientError::Permission(m),
            galaxy_net::handshake::HandshakeError::Auth(m) => ClientError::Auth { code: "peer_auth".into(), message: m },
            galaxy_net::handshake::HandshakeError::Io(m) => ClientError::Network(m),
        }
    }
}

impl From<quinn::ConnectionError> for ClientError {
    fn from(e: quinn::ConnectionError) -> Self {
        ClientError::Network(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, ClientError>;
