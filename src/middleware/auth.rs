use bytes::Bytes;
use http::{header, Request, Response, StatusCode};
use http_body_util::Full;
use hyper::body::Incoming;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, TokenData, Validation};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::{Layer, Service};
use tracing::{debug, warn};

use crate::config::{AuthConfig, AuthProvider};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    pub iat: usize,
    pub iss: Option<String>,
    pub aud: Option<String>,
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Clone)]
pub struct AuthLayer {
    pub config: AuthConfig,
    decoding_key: Option<DecodingKey>,
}

impl AuthLayer {
    pub fn new(config: &AuthConfig) -> Result<Self, GatewayError> {
        let decoding_key = match &config.provider {
            AuthProvider::Jwt { secret, .. } => {
                Some(DecodingKey::from_secret(secret.as_bytes()))
            }
            AuthProvider::OAuth2 { .. } => None,
        };

        Ok(Self {
            config: config.clone(),
            decoding_key,
        })
    }

    pub fn is_excluded_path(&self, path: &str) -> bool {
        self.config.excluded_paths.iter().any(|p| {
            if p.ends_with("/**") {
                let base = &p[..p.len() - 3];
                path.starts_with(base)
            } else if p.ends_with("/*") {
                let base = &p[..p.len() - 2];
                path.starts_with(base)
            } else {
                path == p
            }
        })
    }

    pub fn extract_token<B>(&self, req: &Request<B>) -> Option<String> {
        req.headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| {
                if s.starts_with("Bearer ") {
                    Some(s[7..].to_string())
                } else {
                    None
                }
            })
            .or_else(|| {
                req.uri()
                    .query()
                    .and_then(|q| {
                        q.split('&')
                            .find(|p| p.starts_with("token="))
                            .map(|p| p[6..].to_string())
                    })
            })
    }

    pub fn validate_jwt(&self, token: &str) -> Result<Claims, AuthError> {
        let decoding_key = self
            .decoding_key
            .as_ref()
            .ok_or(AuthError::Configuration("JWT not configured".to_string()))?;

        let mut validation = Validation::new(Algorithm::HS256);

        if let AuthProvider::Jwt { ref issuer, ref audience, .. } = self.config.provider {
            if let Some(iss) = issuer {
                validation.set_issuer(&[iss.as_str()]);
            }
            if let Some(aud) = audience {
                validation.set_audience(&[aud.as_str()]);
            }
        }

        let token_data: TokenData<Claims> = decode(token, decoding_key, &validation)
            .map_err(|e| AuthError::InvalidToken(e.to_string()))?;

        Ok(token_data.claims)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("Missing token")]
    MissingToken,
    #[error("Invalid token: {0}")]
    InvalidToken(String),
    #[error("Configuration error: {0}")]
    Configuration(String),
}

pub type GatewayError = crate::error::GatewayError;

impl<S> Layer<S> for AuthLayer {
    type Service = AuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthService {
            inner,
            config: self.clone(),
        }
    }
}

#[derive(Clone)]
pub struct AuthService<S> {
    inner: S,
    config: AuthLayer,
}

impl<S, B> Service<Request<B>> for AuthService<S>
where
    S: Service<Request<B>, Response = Response<Full<Bytes>>>,
    S::Future: Send + 'static,
    B: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<B>) -> Self::Future {
        let path = req.uri().path().to_string();

        if !self.config.config.enabled || self.config.is_excluded_path(&path) {
            let future = self.inner.call(req);
            return Box::pin(async move { future.await });
        }

        let token = match self.config.extract_token(&req) {
            Some(t) => t,
            None => {
                warn!("Missing authorization token for path: {}", path);
                return Box::pin(async move {
                    Ok(Response::builder()
                        .status(StatusCode::UNAUTHORIZED)
                        .header("WWW-Authenticate", "Bearer")
                        .body(Full::new(Bytes::from(
                            r#"{"error":"Unauthorized","message":"Missing or invalid token"}"#
                        )))
                        .unwrap())
                });
            }
        };

        match self.config.validate_jwt(&token) {
            Ok(claims) => {
                debug!("Authenticated user: {}", claims.sub);
                req.headers_mut().insert(
                    "x-user-id",
                    header::HeaderValue::from_str(&claims.sub).unwrap(),
                );
                let future = self.inner.call(req);
                Box::pin(async move { future.await })
            }
            Err(e) => {
                warn!("Token validation failed: {}", e);
                Box::pin(async move {
                    Ok(Response::builder()
                        .status(StatusCode::UNAUTHORIZED)
                        .header("WWW-Authenticate", "Bearer")
                        .body(Full::new(Bytes::from(format!(
                            r#"{{"error":"Unauthorized","message":"{}"}}"#,
                            e
                        ))))
                        .unwrap())
                })
            }
        }
    }
}
