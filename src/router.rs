use crate::config::{RouteConfig, UpstreamConfig};
use crate::discovery::ServiceDiscovery;
use crate::error::GatewayError;
use http::Request;
use regex::Regex;
use std::sync::Arc;
use tracing::trace;

#[derive(Clone)]
pub struct Router {
    routes: Vec<RouteEntry>,
    discovery: Option<Arc<dyn ServiceDiscovery>>,
}

struct RouteEntry {
    config: RouteConfig,
    path_regex: Regex,
}

impl Clone for RouteEntry {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            path_regex: Regex::new(self.path_regex.as_str()).unwrap(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RouteMatch {
    pub route_id: String,
    pub upstream_url: String,
    pub strip_prefix: Option<String>,
    pub timeout_ms: Option<u64>,
    pub cache_enabled: bool,
    pub auth_required: bool,
    pub retry: Option<crate::config::RetryConfig>,
}

impl Router {
    pub fn new(routes: Vec<RouteConfig>) -> Result<Self, GatewayError> {
        let mut entries = Vec::with_capacity(routes.len());

        for route in routes {
            let pattern = if route.path.ends_with("/**") {
                let base = &route.path[..route.path.len() - 3];
                format!("^{}(/.*)?$", regex::escape(base))
            } else if route.path.ends_with("/*") {
                let base = &route.path[..route.path.len() - 2];
                format!("^{}(/.*)?$", regex::escape(base))
            } else {
                format!("^{}$", regex::escape(&route.path))
            };

            let regex = Regex::new(&pattern)
                .map_err(|e| GatewayError::Config(format!("Invalid route pattern '{}': {}", route.path, e)))?;

            entries.push(RouteEntry {
                config: route,
                path_regex: regex,
            });
        }

        Ok(Self {
            routes: entries,
            discovery: None,
        })
    }

    pub fn with_discovery(mut self, discovery: Arc<dyn ServiceDiscovery>) -> Self {
        self.discovery = Some(discovery);
        self
    }

    pub async fn match_route<B>(&self, req: &Request<B>) -> Option<RouteMatch> {
        let path = req.uri().path();
        let method = req.method().as_str();

        trace!("Routing request: {} {}", method, path);

        for entry in &self.routes {
            if !entry.path_regex.is_match(path) {
                continue;
            }

            if let Some(ref methods) = entry.config.methods {
                if !methods.iter().any(|m| m.eq_ignore_ascii_case(method)) {
                    continue;
                }
            }

            let upstream_url = self.resolve_upstream(&entry.config.upstream).await.ok()?;

            return Some(RouteMatch {
                route_id: entry.config.id.clone(),
                upstream_url,
                strip_prefix: entry.config.strip_prefix.clone(),
                timeout_ms: entry.config.timeout_ms,
                cache_enabled: entry.config.cache_enabled.unwrap_or(false),
                auth_required: entry.config.auth_required.unwrap_or(false),
                retry: entry.config.retry.clone(),
            });
        }

        None
    }

    async fn resolve_upstream(&self, upstream: &UpstreamConfig) -> Result<String, GatewayError> {
        match upstream {
            UpstreamConfig::Static { url } => Ok(url.clone()),
            UpstreamConfig::Service { name, discovery: _ } => {
                if let Some(ref disc) = self.discovery {
                    disc.resolve(name).await
                } else {
                    Err(GatewayError::Discovery("Service discovery not configured".to_string()))
                }
            }
            UpstreamConfig::LoadBalance { endpoints, strategy: _ } => {
                // Simple round-robin for now; load balancer is handled at proxy layer
                if endpoints.is_empty() {
                    return Err(GatewayError::UpstreamUnavailable);
                }
                Ok(endpoints[0].clone())
            }
        }
    }

    pub fn get_routes(&self) -> Vec<RouteConfig> {
        self.routes.iter().map(|e| e.config.clone()).collect()
    }
}
