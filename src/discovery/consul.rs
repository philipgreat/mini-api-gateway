use crate::config::DiscoveryConfig;
use crate::discovery::ServiceDiscovery;
use crate::error::GatewayError;
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

#[derive(Clone)]
pub struct ConsulDiscovery {
    client: reqwest::Client,
    address: String,
    datacenter: Option<String>,
    cache: Arc<RwLock<HashMap<String, Vec<String>>>>,
}

#[derive(Debug, Deserialize)]
struct ConsulService {
    #[serde(rename = "ServiceAddress")]
    service_address: String,
    #[serde(rename = "ServicePort")]
    service_port: u16,
    #[serde(rename = "ServiceTags")]
    #[serde(default)]
    service_tags: Vec<String>,
    #[serde(rename = "Checks")]
    #[serde(default)]
    checks: Vec<ConsulCheck>,
}

#[derive(Debug, Deserialize)]
struct ConsulCheck {
    #[serde(rename = "Status")]
    status: String,
}

impl ConsulDiscovery {
    pub fn new(config: &DiscoveryConfig) -> Result<Self, GatewayError> {
        let address = match &config.provider {
            crate::config::DiscoveryProvider::Consul { address, .. } => address.clone(),
            _ => return Err(GatewayError::Discovery("Invalid provider for Consul".to_string())),
        };

        let datacenter = match &config.provider {
            crate::config::DiscoveryProvider::Consul { datacenter, .. } => datacenter.clone(),
            _ => None,
        };

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|e| GatewayError::Discovery(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            client,
            address,
            datacenter,
            cache: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    fn build_url(&self, service_name: &str) -> String {
        let mut url = format!("{}/v1/health/service/{}", self.address, service_name);
        if let Some(ref dc) = self.datacenter {
            url.push_str(&format!("?dc={}", dc));
        }
        url
    }
}

#[async_trait]
impl ServiceDiscovery for ConsulDiscovery {
    async fn resolve(&self, service_name: &str) -> Result<String, GatewayError> {
        // Check cache first
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

        // Update cache
        {
            let mut cache = self.cache.write().await;
            cache.insert(service_name.to_string(), endpoints.clone());
        }

        Ok(endpoints[0].clone())
    }

    async fn health_check(&self, service_name: &str) -> Result<Vec<String>, GatewayError> {
        let url = self.build_url(service_name);
        debug!("Querying Consul for service: {}", service_name);

        let response = self.client
            .get(&url)
            .send()
            .await
            .map_err(|e| GatewayError::Discovery(format!("Consul query failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(GatewayError::Discovery(format!(
                "Consul returned status: {}",
                response.status()
            )));
        }

        let services: Vec<ConsulService> = response
            .json()
            .await
            .map_err(|e| GatewayError::Discovery(format!("Failed to parse Consul response: {}", e)))?;

        let mut endpoints = Vec::new();
        for svc in services {
            let healthy = svc.checks.iter().all(|c| c.status == "passing");
            if !healthy {
                warn!("Service {} has failing checks", service_name);
                continue;
            }

            let address = if svc.service_address.is_empty() {
                continue;
            } else {
                svc.service_address
            };

            endpoints.push(format!("http://{}:{}", address, svc.service_port));
        }

        info!("Discovered {} healthy endpoints for {}", endpoints.len(), service_name);
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
