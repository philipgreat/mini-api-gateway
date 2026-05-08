use std::fmt;

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] hyper::Error),

    #[error("HTTP body error: {0}")]
    HttpBody(#[from] hyper::http::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("TLS error: {0}")]
    Tls(String),

    #[error("Service discovery error: {0}")]
    Discovery(String),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Rate limit exceeded")]
    RateLimited,

    #[error("Cache error: {0}")]
    Cache(String),

    #[error("Proxy error: {0}")]
    Proxy(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Invalid URI: {0}")]
    InvalidUri(#[from] http::uri::InvalidUri),

    #[error("Request timeout")]
    Timeout,

    #[error("Upstream unavailable")]
    UpstreamUnavailable,

    #[error("Internal error: {0}")]
    Internal(String),
}

impl GatewayError {
    pub fn status_code(&self) -> http::StatusCode {
        match self {
            GatewayError::RateLimited => http::StatusCode::TOO_MANY_REQUESTS,
            GatewayError::Auth(_) => http::StatusCode::UNAUTHORIZED,
            GatewayError::Timeout => http::StatusCode::GATEWAY_TIMEOUT,
            GatewayError::UpstreamUnavailable => http::StatusCode::BAD_GATEWAY,
            GatewayError::InvalidUri(_) => http::StatusCode::BAD_REQUEST,
            _ => http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<GatewayError> for std::io::Error {
    fn from(err: GatewayError) -> Self {
        std::io::Error::new(std::io::ErrorKind::Other, err.to_string())
    }
}
