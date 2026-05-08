use crate::config::{CacheBackend, CacheConfig, CacheKeyStrategy};
use crate::error::GatewayError;
use crate::proxy::ProxyBody;
use bytes::Bytes;
use http::Request;
use hyper::body::Incoming;
use std::sync::Arc;
use std::time::Duration;

pub mod memory;
pub mod redis;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CachedResponse {
    pub status: u16,
    pub headers: Vec<(String, Vec<u8>)>,
    #[serde(with = "serde_bytes")]
    pub body: Vec<u8>,
}

#[async_trait::async_trait]
pub trait CacheBackendTrait: Send + Sync {
    async fn get(&self, key: &str) -> Result<Option<CachedResponse>, GatewayError>;
    async fn set(&self, key: &str, value: &CachedResponse, ttl: Duration) -> Result<(), GatewayError>;
    async fn delete(&self, key: &str) -> Result<(), GatewayError>;
    async fn clear(&self) -> Result<(), GatewayError>;
}

pub type DynCache = Arc<dyn CacheBackendTrait>;

#[derive(Clone)]
pub struct CacheLayer {
    backend: DynCache,
    config: CacheConfig,
}

impl CacheLayer {
    pub fn new(config: &CacheConfig) -> Result<Self, GatewayError> {
        let backend: DynCache = match &config.backend {
            CacheBackend::Memory => Arc::new(memory::MemoryCache::new(config.max_capacity)),
            CacheBackend::Redis { url } => Arc::new(
                redis::RedisCache::new(url)
                    .map_err(|e| GatewayError::Cache(format!("Redis cache init failed: {}", e)))?,
            ),
        };

        Ok(Self {
            backend,
            config: config.clone(),
        })
    }

    pub fn should_cache<B>(&self, req: &Request<B>, status: u16) -> bool {
        if !self.config.enabled {
            return false;
        }

        let method = req.method().as_str();
        if !self.config.cacheable_methods.iter().any(|m| m.eq_ignore_ascii_case(method)) {
            return false;
        }

        if !self.config.cacheable_statuses.iter().any(|&s| s == status) {
            return false;
        }

        let path = req.uri().path();
        if self.config.excluded_paths.iter().any(|p| path.starts_with(p)) {
            return false;
        }

        true
    }

    pub fn build_key<B>(&self, req: &Request<B>) -> String {
        let uri = req.uri().to_string();
        let method = req.method().as_str();

        match &self.config.key_strategy {
            CacheKeyStrategy::Uri => format!("cache:{}", uri),
            CacheKeyStrategy::UriWithMethod => format!("cache:{}:{}", method, uri),
            CacheKeyStrategy::UriWithHeaders(headers) => {
                let mut key = format!("cache:{}:{}", method, uri);
                for header_name in headers {
                    if let Some(value) = req.headers().get(header_name) {
                        key.push(':');
                        key.push_str(&String::from_utf8_lossy(value.as_bytes()));
                    }
                }
                key
            }
        }
    }

    pub async fn get<B>(&self, req: &Request<B>) -> Result<Option<CachedResponse>, GatewayError> {
        let key = self.build_key(req);
        self.backend.get(&key).await
    }

    pub async fn set(&self, key: &str, value: &CachedResponse) -> Result<(), GatewayError> {
        let ttl = Duration::from_secs(self.config.default_ttl_secs);
        self.backend.set(key, value, ttl).await
    }

    pub fn backend(&self) -> &DynCache {
        &self.backend
    }
}
