use crate::config::{LoadBalanceStrategy, RetryConfig, RouteConfig};
use crate::error::GatewayError;
use crate::metrics::RequestMetrics;
use crate::router::RouteMatch;
use bytes::Bytes;
use futures::future::BoxFuture;
use http::Request;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

pub type ProxyBody = Full<Bytes>;

#[derive(Clone)]
pub struct ProxyClient {
    client: reqwest::Client,
    lb_counter: Arc<AtomicUsize>,
}

impl ProxyClient {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .pool_max_idle_per_host(100)
            .pool_idle_timeout(Duration::from_secs(60))
            .tcp_keepalive(Duration::from_secs(75))
            .build()
            .expect("Failed to build reqwest client");

        Self {
            client,
            lb_counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn with_pool(max_idle: usize, idle_timeout: Duration) -> Self {
        let client = reqwest::Client::builder()
            .pool_max_idle_per_host(max_idle)
            .pool_idle_timeout(idle_timeout)
            .tcp_keepalive(Duration::from_secs(75))
            .build()
            .expect("Failed to build reqwest client");

        Self {
            client,
            lb_counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub async fn proxy(
        &self,
        mut req: Request<Incoming>,
        route: &RouteMatch,
        metrics: &RequestMetrics,
    ) -> Result<hyper::Response<ProxyBody>, GatewayError> {
        let upstream_url = &route.upstream_url;
        let uri = req.uri();
        let path = uri.path();
        let query = uri.query().map(|q| format!("?{}", q)).unwrap_or_default();

        let target_path = if let Some(ref prefix) = route.strip_prefix {
            path.strip_prefix(prefix).unwrap_or(path).to_string()
        } else {
            path.to_string()
        };

        let target_url = format!("{}{}{}", upstream_url, target_path, query);
        debug!("Proxying to: {}", target_url);

        let method = reqwest::Method::from_bytes(req.method().as_str().as_bytes())
            .map_err(|e| GatewayError::Proxy(format!("Invalid method: {}", e)))?;

        let mut proxy_req = self.client.request(method, &target_url);

        // Copy headers, filtering hop-by-hop headers
        let mut headers = reqwest::header::HeaderMap::new();
        for (key, value) in req.headers().iter() {
            let key_str = key.as_str().to_lowercase();
            if key_str == "host"
                || key_str == "connection"
                || key_str == "keep-alive"
                || key_str == "proxy-authenticate"
                || key_str == "proxy-authorization"
                || key_str == "te"
                || key_str == "trailers"
                || key_str == "transfer-encoding"
                || key_str == "upgrade"
            {
                continue;
            }
            headers.insert(
                reqwest::header::HeaderName::from_bytes(key.as_str().as_bytes()).unwrap(),
                reqwest::header::HeaderValue::from_bytes(value.as_bytes()).unwrap(),
            );
        }
        proxy_req = proxy_req.headers(headers);

        // Copy body with streaming for large bodies
        let body_bytes = req.into_body()
            .collect()
            .await
            .map_err(|e| GatewayError::Proxy(format!("Body read error: {}", e)))?
            .to_bytes();

        proxy_req = proxy_req.body(body_bytes.to_vec());

        // Execute with timeout and retry
        let timeout = route.timeout_ms.map(Duration::from_millis);
        let response = self.execute_with_retry(proxy_req, route.retry.as_ref(), timeout).await?;

        let status = response.status();
        let mut builder = hyper::Response::builder().status(status.as_u16());

        for (key, value) in response.headers().iter() {
            let key_str = key.as_str().to_lowercase();
            if key_str == "transfer-encoding" || key_str == "content-encoding" {
                continue;
            }
            builder = builder.header(key.as_str(), value.as_bytes());
        }

        let body_bytes = response.bytes().await
            .map_err(|e| GatewayError::Proxy(format!("Response body error: {}", e)))?;

        metrics.record_success(status.as_u16(), body_bytes.len() as u64);

        let body = Full::new(Bytes::from(body_bytes));
        builder.body(body).map_err(|e| GatewayError::HttpBody(e))
    }

    async fn execute_with_retry(
        &self,
        req: reqwest::RequestBuilder,
        retry: Option<&RetryConfig>,
        timeout: Option<Duration>,
    ) -> Result<reqwest::Response, GatewayError> {
        let max_attempts = retry.map(|r| r.max_attempts).unwrap_or(1);
        let backoff = retry.map(|r| Duration::from_millis(r.backoff_ms));

        for attempt in 1..=max_attempts {
            let mut request = req.try_clone()
                .ok_or_else(|| GatewayError::Proxy("Cannot clone request".to_string()))?;

            if let Some(t) = timeout {
                request = request.timeout(t);
            }

            match request.send().await {
                Ok(resp) => {
                    if resp.status().is_server_error() && attempt < max_attempts {
                        warn!("Upstream returned {}, retrying ({}/{})", resp.status(), attempt, max_attempts);
                        if let Some(b) = backoff {
                            tokio::time::sleep(b * attempt).await;
                        }
                        continue;
                    }
                    return Ok(resp);
                }
                Err(e) => {
                    if attempt < max_attempts {
                        warn!("Proxy request failed: {}, retrying ({}/{})", e, attempt, max_attempts);
                        if let Some(b) = backoff {
                            tokio::time::sleep(b * attempt).await;
                        }
                        continue;
                    }
                    return Err(GatewayError::Proxy(format!("Request failed after {} attempts: {}", max_attempts, e)));
                }
            }
        }

        Err(GatewayError::UpstreamUnavailable)
    }

    pub fn select_endpoint(&self, endpoints: &[String], strategy: &LoadBalanceStrategy, client_ip: Option<&str>) -> Option<String> {
        if endpoints.is_empty() {
            return None;
        }

        match strategy {
            LoadBalanceStrategy::RoundRobin => {
                let idx = self.lb_counter.fetch_add(1, Ordering::SeqCst) % endpoints.len();
                Some(endpoints[idx].clone())
            }
            LoadBalanceStrategy::Random => {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let mut hasher = DefaultHasher::new();
                std::time::SystemTime::now().hash(&mut hasher);
                let idx = hasher.finish() as usize % endpoints.len();
                Some(endpoints[idx].clone())
            }
            LoadBalanceStrategy::IpHash => {
                if let Some(ip) = client_ip {
                    use std::collections::hash_map::DefaultHasher;
                    use std::hash::{Hash, Hasher};
                    let mut hasher = DefaultHasher::new();
                    ip.hash(&mut hasher);
                    let idx = hasher.finish() as usize % endpoints.len();
                    Some(endpoints[idx].clone())
                } else {
                    Some(endpoints[0].clone())
                }
            }
            LoadBalanceStrategy::LeastConnections => {
                // Simplified: use round-robin as proxy for least connections
                let idx = self.lb_counter.fetch_add(1, Ordering::SeqCst) % endpoints.len();
                Some(endpoints[idx].clone())
            }
        }
    }
}

impl Default for ProxyClient {
    fn default() -> Self {
        Self::new()
    }
}
