use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::{BodyExt, Full};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::{Layer, Service};
use tracing::{debug, trace};

use crate::cache::{CacheLayer, CachedResponse};
use crate::proxy::ProxyBody;

#[derive(Clone)]
pub struct CacheMiddlewareLayer {
    cache: CacheLayer,
}

impl CacheMiddlewareLayer {
    pub fn new(cache: CacheLayer) -> Self {
        Self { cache }
    }
}

impl<S> Layer<S> for CacheMiddlewareLayer {
    type Service = CacheMiddlewareService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CacheMiddlewareService {
            inner,
            cache: self.cache.clone(),
        }
    }
}

#[derive(Clone)]
pub struct CacheMiddlewareService<S> {
    inner: S,
    cache: CacheLayer,
}

impl<S, B> Service<Request<B>> for CacheMiddlewareService<S>
where
    S: Service<Request<B>, Response = Response<ProxyBody>>,
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
        let method = req.method().clone();
        let path = req.uri().path().to_string();

        // Only cache GET requests by default
        if method != http::Method::GET {
            trace!("Skipping cache for non-GET request: {} {}", method, path);
            let future = self.inner.call(req);
            return Box::pin(async move { future.await });
        }

        let cache = self.cache.clone();
        let cache_key = cache.build_key(&req);
        
        // Check cache
        let future = self.inner.call(req);

        Box::pin(async move {
            // Try to get from cache first
            if let Ok(Some(cached)) = cache.backend().get(&cache_key).await {
                debug!("Cache hit for: {}", path);
                let mut builder = Response::builder().status(cached.status);
                for (name, value) in &cached.headers {
                    builder = builder.header(name, value.as_slice());
                }
                builder = builder.header("X-Cache", "HIT");
                return Ok(builder.body(Full::new(Bytes::from(cached.body))).unwrap());
            }

            let response = future.await?;
            let status = response.status();

            // Check if response should be cached
            if cache.should_cache(&Request::new(()), status.as_u16()) {
                let (parts, body) = response.into_parts();
                let body_bytes = body.collect().await.unwrap().to_bytes();

                let cached = CachedResponse {
                    status: status.as_u16(),
                    headers: parts.headers.iter()
                        .map(|(k, v)| (k.as_str().to_string(), v.as_bytes().to_vec()))
                        .collect(),
                    body: body_bytes.to_vec(),
                };

                if let Err(e) = cache.set(&cache_key, &cached).await {
                    debug!("Failed to cache response: {}", e);
                }

                let mut builder = Response::builder().status(status);
                for (key, value) in &parts.headers {
                    builder = builder.header(key, value);
                }
                builder = builder.header("X-Cache", "MISS");
                Ok(builder.body(Full::new(body_bytes)).unwrap())
            } else {
                Ok(response)
            }
        })
    }
}
