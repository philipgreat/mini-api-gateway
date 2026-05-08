use crate::config::DiscoveryConfig;
use crate::discovery::ServiceDiscovery;
use crate::error::GatewayError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

#[derive(Clone)]
pub struct EtcdDiscovery {
    client: reqwest::Client,
    endpoints: Vec<String>,
    prefix: String,
    cache: Arc<RwLock<std::collections::HashMap<String, Vec<String>>>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EtcdRangeRequest {
    key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    range_end: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EtcdRangeResponse {
    #[serde(default)]
    kvs: Vec<EtcdKeyValue>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EtcdKeyValue {
    key: String,
    value: String,
}

impl EtcdDiscovery {
    pub fn new(config: &DiscoveryConfig) -> Result<Self, GatewayError> {
        let endpoints = match &config.provider {
            crate::config::DiscoveryProvider::Etcd { endpoints } => endpoints.clone(),
            _ => return Err(GatewayError::Discovery("Invalid provider for Etcd".to_string())),
        };

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|e| GatewayError::Discovery(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            client,
            endpoints,
            prefix: "/services/".to_string(),
            cache: Arc::new(RwLock::new(std::collections::HashMap::new())),
        })
    }

    fn get_endpoint(&self) -> String {
        self.endpoints.first()
            .cloned()
            .unwrap_or_else(|| "http://127.0.0.1:2379".to_string())
    }

    fn service_key(&self, service_name: &str) -> String {
        format!("{}{}", self.prefix, service_name)
    }

    fn range_end(&self, key: &str) -> String {
        let mut bytes = key.as_bytes().to_vec();
        if let Some(last) = bytes.last_mut() {
            *last = last.wrapping_add(1);
        }
        String::from_utf8_lossy(&bytes).to_string()
    }
}

#[async_trait]
impl ServiceDiscovery for EtcdDiscovery {
    async fn resolve(&self, service_name: &str) -> Result<String, GatewayError> {
        {
            let cache = self.cache.read().await;
            if let Some(endpoints) = cache.get(service_name) {
                if !endpoints.is_empty() {
                    return Ok(endpoints[0].clone());
                }
            }
        }

        let endpoints = self.health_check(service_name).await?;
        if endpoints.is_empty() {
            return Err(GatewayError::UpstreamUnavailable);
        }

        Ok(endpoints[0].clone())
    }

    async fn health_check(&self, service_name: &str) -> Result<Vec<String>, GatewayError> {
        let key = self.service_key(service_name);
        let range_end = self.range_end(&key);
        let endpoint = self.get_endpoint();

        debug!("Querying Etcd for service: {} at {}", service_name, endpoint);

        let request = EtcdRangeRequest {
            key: base64::encode(&key),
            range_end: Some(base64::encode(&range_end)),
        };

        let url = format!("{}/v3/kv/range", endpoint);
        let response = self.client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| GatewayError::Discovery(format!("Etcd query failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(GatewayError::Discovery(format!(
                "Etcd returned status: {}",
                response.status()
            )));
        }

        let etcd_resp: EtcdRangeResponse = response
            .json()
            .await
            .map_err(|e| GatewayError::Discovery(format!("Failed to parse Etcd response: {}", e)))?;

        let mut endpoints = Vec::new();
        for kv in etcd_resp.kvs {
            let decoded_key = String::from_utf8_lossy(
                &base64::decode(&kv.key).unwrap_or_default()
            ).to_string();
            let decoded_value = String::from_utf8_lossy(
                &base64::decode(&kv.value).unwrap_or_default()
            ).to_string();
            let endpoint_url = decoded_value.trim().to_string();
            if !endpoint_url.is_empty() {
                endpoints.push(endpoint_url);
            }
            debug!("Etcd key: {}, value: {}", decoded_key, decoded_value);
        }

        if endpoints.is_empty() {
            warn!("No endpoints found for service {} in Etcd", service_name);
        } else {
            info!("Discovered {} endpoints for {} from Etcd", endpoints.len(), service_name);
        }

        {
            let mut cache = self.cache.write().await;
            cache.insert(service_name.to_string(), endpoints.clone());
        }

        Ok(endpoints)
    }

    async fn watch(&self, service_name: &str) -> Result<(), GatewayError> {
        let this = self.clone();
        let name = service_name.to_string();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                match this.health_check(&name).await {
                    Ok(endpoints) => {
                        let mut cache = this.cache.write().await;
                        cache.insert(name.clone(), endpoints);
                    }
                    Err(e) => {
                        error!("Health check failed for {}: {}", name, e);
                    }
                }
            }
        });

        Ok(())
    }
}
