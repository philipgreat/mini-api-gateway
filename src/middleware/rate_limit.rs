use bytes::Bytes;
use governor::{clock::DefaultClock, state::keyed::DefaultKeyedStateStore, Quota, RateLimiter};
use http::Request;
use http_body_util::Full;
use hyper::body::Incoming;
use std::future::Future;
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tower::{Layer, Service};
use tracing::warn;

use crate::config::RateLimitKeyStrategy;

pub type KeyRateLimiter = RateLimiter<String, DefaultKeyedStateStore<String>, DefaultClock>;

#[derive(Clone)]
pub struct RateLimitLayer {
    pub limiter: Arc<KeyRateLimiter>,
    pub key_strategy: RateLimitKeyStrategy,
}

impl RateLimitLayer {
    pub fn new(rps: u32, burst: u32, key_strategy: RateLimitKeyStrategy) -> Self {
        let quota = Quota::per_second(
            NonZeroU32::new(rps).unwrap_or(NonZeroU32::new(100).unwrap())
        )
        .allow_burst(
            NonZeroU32::new(burst).unwrap_or(NonZeroU32::new(100).unwrap())
        );

        let limiter = Arc::new(RateLimiter::keyed(quota));

        Self {
            limiter,
            key_strategy,
        }
    }

    fn extract_key<B>(&self, req: &Request<B>, client_addr: Option<SocketAddr>) -> String {
        match &self.key_strategy {
            RateLimitKeyStrategy::Global => "global".to_string(),
            RateLimitKeyStrategy::Ip => {
                client_addr
                    .map(|addr| addr.ip().to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            }
            RateLimitKeyStrategy::Header(header_name) => {
                req.headers()
                    .get(header_name)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("unknown")
                    .to_string()
            }
            RateLimitKeyStrategy::Custom(key) => key.clone(),
        }
    }
}

impl<S> Layer<S> for RateLimitLayer {
    type Service = RateLimitService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimitService {
            inner,
            limiter: self.limiter.clone(),
            key_strategy: self.key_strategy.clone(),
        }
    }
}

#[derive(Clone)]
pub struct RateLimitService<S> {
    inner: S,
    limiter: Arc<KeyRateLimiter>,
    key_strategy: RateLimitKeyStrategy,
}

impl<S, B> Service<Request<B>> for RateLimitService<S>
where
    S: Service<Request<B>, Response = hyper::Response<Full<Bytes>>>,
    S::Future: Send + 'static,
    B: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        let key = match &self.key_strategy {
            RateLimitKeyStrategy::Global => "global".to_string(),
            RateLimitKeyStrategy::Ip => {
                // Extract from x-forwarded-for or connection info if available
                req.headers()
                    .get("x-forwarded-for")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.split(',').next())
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            }
            RateLimitKeyStrategy::Header(header_name) => {
                req.headers()
                    .get(header_name)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("unknown")
                    .to_string()
            }
            RateLimitKeyStrategy::Custom(key) => key.clone(),
        };

        match self.limiter.check_key(&key) {
            Ok(()) => {
                let future = self.inner.call(req);
                Box::pin(async move { future.await })
            }
            Err(_) => {
                warn!("Rate limit exceeded for key: {}", key);
                Box::pin(async move {
                    let response = hyper::Response::builder()
                        .status(429)
                        .header("Content-Type", "application/json")
                        .body(Full::new(Bytes::from(
                            r#"{"error":"Rate limit exceeded","retry_after":1}"#
                        )))
                        .unwrap();
                    Ok(response)
                })
            }
        }
    }
}
