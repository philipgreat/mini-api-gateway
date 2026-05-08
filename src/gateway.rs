use crate::cache::CacheLayer;
use crate::config::GatewayConfig;
use crate::discovery::{consul::ConsulDiscovery, etcd::EtcdDiscovery, DynServiceDiscovery};
use crate::error::GatewayError;
use crate::metrics::{record_active_connections, MetricsExporter, RequestMetrics};
use crate::middleware::{auth::AuthLayer, cors::CorsLayer, rate_limit::RateLimitLayer};
use crate::proxy::{ProxyBody, ProxyClient};
use crate::router::Router;
use crate::tls::create_tls_acceptor;
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tracing::{error, info, warn};

pub struct Gateway {
    config: GatewayConfig,
    router: Arc<Router>,
    proxy_client: ProxyClient,
    metrics: Option<MetricsExporter>,
    tls_acceptor: Option<TlsAcceptor>,
    auth_layer: Option<AuthLayer>,
    rate_limit_layer: Option<RateLimitLayer>,
    cors_layer: Option<CorsLayer>,
    cache_layer: Option<CacheLayer>,
}

impl Gateway {
    pub async fn new(config: GatewayConfig) -> Result<Self, GatewayError> {
        // Initialize router
        let mut router = Router::new(config.routes.clone())?;

        // Initialize service discovery if configured
        let discovery: Option<DynServiceDiscovery> = if let Some(ref disc_config) = config.discovery {
            if disc_config.enabled {
                let discovery: DynServiceDiscovery = match disc_config.provider {
                    crate::config::DiscoveryProvider::Consul { .. } => {
                        Arc::new(ConsulDiscovery::new(disc_config)?)
                    }
                    crate::config::DiscoveryProvider::Etcd { .. } => {
                        Arc::new(EtcdDiscovery::new(disc_config)?)
                    }
                };

                // Start watching services
                for route in &config.routes {
                    if let crate::config::UpstreamConfig::Service { ref name, .. } = route.upstream {
                        let _ = discovery.watch(name).await;
                    }
                }

                router = router.with_discovery(discovery.clone());
                Some(discovery)
            } else {
                None
            }
        } else {
            None
        };

        // Initialize TLS if configured
        let tls_acceptor = if let Some(ref tls_config) = config.tls {
            if tls_config.enabled {
                Some(create_tls_acceptor(tls_config)?)
            } else {
                None
            }
        } else {
            None
        };

        // Initialize auth layer
        let auth_layer = if let Some(ref auth_config) = config.auth {
            if auth_config.enabled {
                Some(AuthLayer::new(auth_config).map_err(|e| GatewayError::Auth(e.to_string()))?)
            } else {
                None
            }
        } else {
            None
        };

        // Initialize rate limit layer
        let rate_limit_layer = if let Some(ref rl_config) = config.rate_limit {
            if rl_config.enabled {
                Some(RateLimitLayer::new(
                    rl_config.requests_per_second,
                    rl_config.burst_size,
                    rl_config.key_strategy.clone(),
                ))
            } else {
                None
            }
        } else {
            None
        };

        // Initialize CORS layer
        let cors_layer = if let Some(ref cors_config) = config.cors {
            if cors_config.enabled {
                Some(CorsLayer::new(
                    cors_config.allow_origins.clone(),
                    cors_config.allow_methods.clone(),
                    cors_config.allow_headers.clone(),
                    cors_config.allow_credentials,
                    cors_config.max_age,
                    cors_config.expose_headers.clone(),
                ))
            } else {
                None
            }
        } else {
            None
        };

        // Initialize cache layer
        let cache_layer = if let Some(ref cache_config) = config.cache {
            if cache_config.enabled {
                Some(CacheLayer::new(cache_config)?)
            } else {
                None
            }
        } else {
            None
        };

        // Initialize proxy client with connection pool
        let proxy_client = if let Some(ref pool_config) = config.pool {
            if pool_config.enabled {
                ProxyClient::with_pool(
                    pool_config.max_connections,
                    Duration::from_secs(pool_config.idle_timeout_secs),
                )
            } else {
                ProxyClient::new()
            }
        } else {
            ProxyClient::new()
        };

        // Initialize metrics
        let metrics = if let Some(ref metrics_config) = config.metrics {
            if metrics_config.enabled {
                Some(MetricsExporter::new())
            } else {
                None
            }
        } else {
            None
        };

        Ok(Self {
            config,
            router: Arc::new(router),
            proxy_client,
            metrics,
            tls_acceptor,
            auth_layer,
            rate_limit_layer,
            cors_layer,
            cache_layer,
        })
    }

    pub async fn run(self) -> Result<(), GatewayError> {
        let addr = self.config.server.listen;
        let listener = TcpListener::bind(addr).await?;

        if self.tls_acceptor.is_some() {
            info!("Gateway listening on https://{}", addr);
        } else {
            info!("Gateway listening on http://{}", addr);
        }

        // Start metrics server if configured
        if let Some(metrics) = self.metrics.clone() {
            if let Some(ref metrics_config) = self.config.metrics {
                let metrics_addr = metrics_config.listen;
                let metrics_endpoint = metrics_config.endpoint.clone();
                tokio::spawn(async move {
                    if let Err(e) = metrics.serve(metrics_addr, metrics_endpoint).await {
                        error!("Metrics server error: {}", e);
                    }
                });
            }
        }

        // Accept incoming connections
        let tls_acceptor = self.tls_acceptor.clone();
        let gateway = self.clone();

        loop {
            let (stream, client_addr) = listener.accept().await?;
            let gateway = gateway.clone();
            let acceptor = tls_acceptor.clone();

            record_active_connections(1);

            tokio::spawn(async move {
                if let Some(acceptor) = acceptor {
                    match acceptor.accept(stream).await {
                        Ok(tls_stream) => {
                            let io = TokioIo::new(tls_stream);
                            if let Err(e) = serve_connection(gateway, io, client_addr).await {
                                error!("TLS connection error: {}", e);
                            }
                        }
                        Err(e) => {
                            error!("TLS handshake error: {}", e);
                        }
                    }
                } else {
                    let io = TokioIo::new(stream);
                    if let Err(e) = serve_connection(gateway, io, client_addr).await {
                        error!("Connection error: {}", e);
                    }
                }

                record_active_connections(-1);
            });
        }
    }

    async fn handle_request(
        &self,
        mut req: Request<Incoming>,
        _client_addr: std::net::SocketAddr,
    ) -> Result<Response<ProxyBody>, Infallible> {
        let method = req.method().clone();
        let path = req.uri().path().to_string();

        // Add request ID
        let request_id = uuid::Uuid::new_v4().to_string();
        if let Some(ref header) = self.config.logging.request_id_header {
            if let Ok(name) = http::header::HeaderName::from_bytes(header.as_bytes()) {
                if let Ok(value) = http::header::HeaderValue::from_str(&request_id) {
                    req.headers_mut().insert(name, value);
                }
            }
        }

        let start = std::time::Instant::now();

        // Match route
        let route_match = match self.router.match_route(&req).await {
            Some(route) => route,
            None => {
                warn!("No route found for: {} {}", method, path);
                return Ok(Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .header("Content-Type", "application/json")
                    .body(Full::new(Bytes::from(
                        r#"{"error":"Not Found","message":"No route matches the request"}"#
                    )))
                    .unwrap());
            }
        };

        let route_id = route_match.route_id.clone();
        let metrics = RequestMetrics::new(method.as_str(), &path, &route_id);

        // Handle CORS preflight
        if let Some(ref cors) = self.cors_layer {
            if req.method() == http::Method::OPTIONS {
                let origin = req.headers()
                    .get(http::header::ORIGIN)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();

                if cors.is_origin_allowed(&origin) {
                    let response = cors.build_preflight_response(&origin);
                    return Ok(response);
                }
            }
        }

        // Check auth
        if let Some(ref auth) = self.auth_layer {
            if auth.config.enabled && !auth.is_excluded_path(&path) {
                if let Some(token) = auth.extract_token(&req) {
                    match auth.validate_jwt(&token) {
                        Ok(claims) => {
                            if let Ok(value) = http::header::HeaderValue::from_str(&claims.sub) {
                                req.headers_mut().insert("x-user-id", value);
                            }
                        }
                        Err(e) => {
                            metrics.record_auth_failure();
                            return Ok(Response::builder()
                                .status(StatusCode::UNAUTHORIZED)
                                .header("WWW-Authenticate", "Bearer")
                                .body(Full::new(Bytes::from(format!(
                                    r#"{{"error":"Unauthorized","message":"{}"}}"#,
                                    e
                                ))))
                                .unwrap());
                        }
                    }
                } else {
                    metrics.record_auth_failure();
                    return Ok(Response::builder()
                        .status(StatusCode::UNAUTHORIZED)
                        .header("WWW-Authenticate", "Bearer")
                        .body(Full::new(Bytes::from(
                            r#"{"error":"Unauthorized","message":"Missing or invalid token"}"#
                        )))
                        .unwrap());
                }
            }
        }

        // Check rate limit
        if let Some(ref rate_limit) = self.rate_limit_layer {
            let key = match &rate_limit.key_strategy {
                crate::config::RateLimitKeyStrategy::Global => "global".to_string(),
                crate::config::RateLimitKeyStrategy::Ip => {
                    req.headers()
                        .get("x-forwarded-for")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.split(',').next())
                        .map(|s| s.trim().to_string())
                        .unwrap_or_else(|| "unknown".to_string())
                }
                crate::config::RateLimitKeyStrategy::Header(header_name) => {
                    req.headers()
                        .get(header_name)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("unknown")
                        .to_string()
                }
                crate::config::RateLimitKeyStrategy::Custom(key) => key.clone(),
            };

            if rate_limit.limiter.check_key(&key).is_err() {
                metrics.record_rate_limited();
                return Ok(Response::builder()
                    .status(StatusCode::TOO_MANY_REQUESTS)
                    .header("Content-Type", "application/json")
                    .body(Full::new(Bytes::from(
                        r#"{"error":"Rate limit exceeded","retry_after":1}"#
                    )))
                    .unwrap());
            }
        }

        // Check cache
        if let Some(ref cache) = self.cache_layer {
            if route_match.cache_enabled && req.method() == http::Method::GET {
                let cache_key = cache.build_key(&req);
                if let Ok(Some(cached)) = cache.backend().get(&cache_key).await {
                    metrics.record_cache_hit();
                    let mut builder = Response::builder().status(cached.status);
                    for (name, value) in &cached.headers {
                        builder = builder.header(name, value.as_slice());
                    }
                    builder = builder.header("X-Cache", "HIT");
                    return Ok(builder.body(Full::new(Bytes::from(cached.body.clone()))).unwrap());
                }
            }
        }

        // Proxy the request
        let result = self.proxy_client.proxy(req, &route_match, &metrics).await;

        let duration = start.elapsed();

        let response = match result {
            Ok(mut response) => {
                // Add CORS headers to actual response
                if let Some(ref cors) = self.cors_layer {
                    let origin = response.headers()
                        .get(http::header::ORIGIN)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    if !origin.is_empty() && cors.is_origin_allowed(&origin) {
                        cors.add_cors_headers(&mut response, &origin);
                    }
                }

                response
            }
            Err(e) => {
                error!("Request handling error: {}", e);
                Response::builder()
                    .status(e.status_code())
                    .header("Content-Type", "application/json")
                    .body(Full::new(Bytes::from(format!(
                        r#"{{"error":"{}","message":"{}"}}"#,
                        e.status_code().as_u16(),
                        e
                    ))))
                    .unwrap()
            }
        };

        let status = response.status();
        if status.is_server_error() || status.is_client_error() {
            tracing::warn!(
                request_id = %request_id,
                method = %method,
                uri = %path,
                status = %status,
                duration_ms = %duration.as_millis(),
                "Request completed with error status"
            );
        } else {
            tracing::info!(
                request_id = %request_id,
                method = %method,
                uri = %path,
                status = %status,
                duration_ms = %duration.as_millis(),
                "Request completed"
            );
        }

        Ok(response)
    }
}

impl Clone for Gateway {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            router: self.router.clone(),
            proxy_client: self.proxy_client.clone(),
            metrics: self.metrics.clone(),
            tls_acceptor: self.tls_acceptor.clone(),
            auth_layer: self.auth_layer.clone(),
            rate_limit_layer: self.rate_limit_layer.clone(),
            cors_layer: self.cors_layer.clone(),
            cache_layer: self.cache_layer.clone(),
        }
    }
}

async fn serve_connection(
    gateway: Gateway,
    io: TokioIo<impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static>,
    client_addr: std::net::SocketAddr,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let svc = service_fn(move |req: Request<Incoming>| {
        let gateway = gateway.clone();
        async move {
            gateway.handle_request(req, client_addr).await
        }
    });

    hyper::server::conn::http1::Builder::new()
        .serve_connection(io, svc)
        .await?;

    Ok(())
}
