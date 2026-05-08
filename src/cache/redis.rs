use crate::cache::{CacheBackendTrait, CachedResponse};
use crate::error::GatewayError;
use redis::{aio::ConnectionManager, AsyncCommands, RedisResult};
use std::sync::Arc;
use tokio::sync::Mutex;
use std::time::Duration;

#[derive(Clone)]
pub struct RedisCache {
    client: redis::Client,
    conn: Arc<Mutex<Option<ConnectionManager>>>,
}

impl RedisCache {
    pub fn new(url: &str) -> Result<Self, GatewayError> {
        let client = redis::Client::open(url)
            .map_err(|e| GatewayError::Cache(format!("Invalid Redis URL: {}", e)))?;

        Ok(Self {
            client,
            conn: Arc::new(Mutex::new(None)),
        })
    }

    async fn get_conn(&self) -> Result<ConnectionManager, GatewayError> {
        let mut guard = self.conn.lock().await;
        if let Some(conn) = guard.as_ref() {
            return Ok(conn.clone());
        }

        let conn = ConnectionManager::new(self.client.clone())
            .await
            .map_err(|e| GatewayError::Cache(format!("Redis connection failed: {}", e)))?;

        *guard = Some(conn.clone());
        Ok(conn)
    }

    fn serialize(value: &CachedResponse) -> Result<Vec<u8>, GatewayError> {
        serde_json::to_vec(value)
            .map_err(|e| GatewayError::Serialization(e))
    }

    fn deserialize(data: &[u8]) -> Result<CachedResponse, GatewayError> {
        serde_json::from_slice(data)
            .map_err(|e| GatewayError::Serialization(e))
    }
}

#[async_trait::async_trait]
impl CacheBackendTrait for RedisCache {
    async fn get(&self, key: &str) -> Result<Option<CachedResponse>, GatewayError> {
        let mut conn = self.get_conn().await?;
        let data: Option<Vec<u8>> = conn
            .get(key)
            .await
            .map_err(|e| GatewayError::Cache(format!("Redis GET failed: {}", e)))?;

        match data {
            Some(bytes) => {
                let value = Self::deserialize(&bytes)?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    async fn set(&self, key: &str, value: &CachedResponse, ttl: Duration) -> Result<(), GatewayError> {
        let mut conn = self.get_conn().await?;
        let data = Self::serialize(value)?;
        let ttl_secs = ttl.as_secs() as u64;

        conn.set_ex(key, data, ttl_secs)
            .await
            .map_err(|e| GatewayError::Cache(format!("Redis SET failed: {}", e)))?;

        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), GatewayError> {
        let mut conn = self.get_conn().await?;
        conn.del(key)
            .await
            .map_err(|e| GatewayError::Cache(format!("Redis DEL failed: {}", e)))?;

        Ok(())
    }

    async fn clear(&self) -> Result<(), GatewayError> {
        let mut conn = self.get_conn().await?;
        let _: () = redis::cmd("FLUSHDB")
            .query_async(&mut conn)
            .await
            .map_err(|e| GatewayError::Cache(format!("Redis FLUSHDB failed: {}", e)))?;

        Ok(())
    }
}
