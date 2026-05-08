use metrics::{counter, gauge, histogram, describe_counter, describe_gauge, describe_histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, error};

#[derive(Clone)]
pub struct MetricsExporter {
    handle: Arc<PrometheusHandle>,
}

impl MetricsExporter {
    pub fn new() -> Self {
        let builder = PrometheusBuilder::new();
        let handle = builder
            .install_recorder()
            .expect("Failed to install Prometheus recorder");

        Self::register_descriptions();

        Self { handle: Arc::new(handle) }
    }

    fn register_descriptions() {
        describe_counter!(
            "gateway_requests_total",
            "Total number of HTTP requests"
        );
        describe_counter!(
            "gateway_requests_error_total",
            "Total number of HTTP error responses"
        );
        describe_histogram!(
            "gateway_request_duration_seconds",
            "HTTP request latency in seconds"
        );
        describe_histogram!(
            "gateway_response_size_bytes",
            "HTTP response size in bytes"
        );
        describe_histogram!(
            "gateway_request_size_bytes",
            "HTTP request size in bytes"
        );
        describe_gauge!(
            "gateway_active_connections",
            "Number of active connections"
        );
        describe_gauge!(
            "gateway_upstreams_available",
            "Number of available upstream endpoints"
        );
        describe_counter!(
            "gateway_cache_hits_total",
            "Total number of cache hits"
        );
        describe_counter!(
            "gateway_cache_misses_total",
            "Total number of cache misses"
        );
        describe_counter!(
            "gateway_rate_limited_total",
            "Total number of rate limited requests"
        );
        describe_counter!(
            "gateway_auth_failures_total",
            "Total number of authentication failures"
        );
    }

    pub fn render(&self) -> String {
        self.handle.render()
    }

    pub fn handle(&self) -> Arc<PrometheusHandle> {
        self.handle.clone()
    }

    pub async fn serve(self, addr: SocketAddr, endpoint: String) -> anyhow::Result<()> {
        use hyper::service::service_fn;
        use hyper::{Request, Response, body::Incoming};
        use hyper_util::rt::TokioIo;
        use std::convert::Infallible;
        use tokio::net::TcpListener;
        use http_body_util::Full;
        use bytes::Bytes;

        let listener = TcpListener::bind(addr).await?;
        info!("Metrics server listening on http://{}", addr);

        loop {
            let (stream, _) = listener.accept().await?;
            let io = TokioIo::new(stream);
            let endpoint = endpoint.clone();
            let metrics = self.render();

            tokio::spawn(async move {
                let svc = service_fn(move |req: Request<Incoming>| {
                    let ep = endpoint.clone();
                    let m = metrics.clone();
                    async move {
                        if req.uri().path() == ep {
                            let response = Response::builder()
                                .status(200)
                                .header("Content-Type", "text/plain; charset=utf-8")
                                .body(Full::new(Bytes::from(m)))
                                .unwrap();
                            Ok::<_, Infallible>(response)
                        } else {
                            let response = Response::builder()
                                .status(404)
                                .body(Full::new(Bytes::from("Not Found")))
                                .unwrap();
                            Ok(response)
                        }
                    }
                });

                if let Err(e) = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await
                {
                    error!("Metrics server connection error: {}", e);
                }
            });
        }
    }
}

#[derive(Clone)]
pub struct RequestMetrics {
    start: Instant,
    method: String,
    path: String,
    route_id: String,
}

impl RequestMetrics {
    pub fn new(method: &str, path: &str, route_id: &str) -> Self {
        Self {
            start: Instant::now(),
            method: method.to_string(),
            path: path.to_string(),
            route_id: route_id.to_string(),
        }
    }

    pub fn record_success(&self, status: u16, response_size: u64) {
        let duration = self.start.elapsed().as_secs_f64();
        let labels = [
            ("method", self.method.clone()),
            ("path", self.path.clone()),
            ("route_id", self.route_id.clone()),
            ("status", status.to_string()),
        ];

        counter!("gateway_requests_total", &labels).increment(1);
        histogram!("gateway_request_duration_seconds", &labels).record(duration);
        histogram!("gateway_response_size_bytes", &labels).record(response_size as f64);

        if status >= 400 {
            counter!("gateway_requests_error_total", &labels).increment(1);
        }
    }

    pub fn record_cache_hit(&self) {
        counter!(
            "gateway_cache_hits_total",
            "route_id" => self.route_id.clone()
        ).increment(1);
    }

    pub fn record_cache_miss(&self) {
        counter!(
            "gateway_cache_misses_total",
            "route_id" => self.route_id.clone()
        ).increment(1);
    }

    pub fn record_rate_limited(&self) {
        counter!(
            "gateway_rate_limited_total",
            "route_id" => self.route_id.clone(),
            "method" => self.method.clone()
        ).increment(1);
    }

    pub fn record_auth_failure(&self) {
        counter!(
            "gateway_auth_failures_total",
            "route_id" => self.route_id.clone()
        ).increment(1);
    }
}

pub fn record_active_connections(delta: i64) {
    gauge!("gateway_active_connections").increment(delta as f64);
}

pub fn record_request_size(size: u64) {
    histogram!("gateway_request_size_bytes").record(size as f64);
}
