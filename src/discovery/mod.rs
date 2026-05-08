use crate::error::GatewayError;
use async_trait::async_trait;
use std::sync::Arc;

pub mod consul;
pub mod etcd;

#[async_trait]
pub trait ServiceDiscovery: Send + Sync {
    async fn resolve(&self, service_name: &str) -> Result<String, GatewayError>;
    async fn health_check(&self, service_name: &str) -> Result<Vec<String>, GatewayError>;
    async fn watch(&self, service_name: &str) -> Result<(), GatewayError>;
}

pub type DynServiceDiscovery = Arc<dyn ServiceDiscovery>;
