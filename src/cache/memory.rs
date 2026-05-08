use crate::cache::{CacheBackendTrait, CachedResponse};
use crate::error::GatewayError;
use moka::future::Cache;
use std::time::Duration;

pub struct MemoryCache {
    inner: Cache<String, CachedResponse>,
}

impl MemoryCache {
    pub fn new(max_capacity: Option<u64>) -> Self {
        let max_capacity = max_capacity.unwrap_or(10_000);
        let cache = Cache::builder()
            .max_capacity(max_capacity)
            .build();

        Self { inner: cache }
    }
}

#[async_trait::async_trait]
impl CacheBackendTrait for MemoryCache {
    async fn get(&self, key: &str) -> Result<Option<CachedResponse>, GatewayError> {
        Ok(self.inner.get(key).await)
    }

    async fn set(&self, key: &str, value: &CachedResponse, ttl: Duration) -> Result<(), GatewayError> {
        self.inner.insert(key.to_string(), value.clone()).await;
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), GatewayError> {
        self.inner.invalidate(key).await;
        Ok(())
    }

    async fn clear(&self) -> Result<(), GatewayError> {
        self.inner.invalidate_all();
        Ok(())
    }
}
