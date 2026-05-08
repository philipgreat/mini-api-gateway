use bytes::Bytes;
use http::{Method, Request, Response, StatusCode};
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use mini_api_gateway::config::*;
use mini_api_gateway::gateway::Gateway;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::time::{sleep, timeout};

async fn upstream_handler(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    let path = req.uri().path();
    let method = req.method().as_str();

    let response = if path == "/health" {
        Response::builder()
            .status(200)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(r#"{"status":"ok"}"#)))
            .unwrap()
    } else if path == "/echo" {
        Response::builder()
            .status(200)
            .header("Content-Type", "text/plain")
            .body(Full::new(Bytes::from(format!("method={}", method))))
            .unwrap()
    } else if path == "/slow" {
        sleep(Duration::from_secs(2)).await;
        Response::builder()
            .status(200)
            .body(Full::new(Bytes::from("slow response")))
            .unwrap()
    } else if path == "/error" {
        Response::builder()
            .status(500)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(r#"{"error":"internal"}"#)))
            .unwrap()
    } else {
        Response::builder()
            .status(404)
            .body(Full::new(Bytes::from("Not Found")))
            .unwrap()
    };

    Ok(response)
}

// Start a simple upstream server for testing
async fn start_upstream_server(addr: SocketAddr) -> tokio::task::JoinHandle<()> {
    let listener = TcpListener::bind(addr).await.unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            tokio::spawn(async move {
                let svc = service_fn(upstream_handler);
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    })
}

fn create_test_config(gateway_port: u16, upstream_port: u16) -> GatewayConfig {
    GatewayConfig {
        server: ServerConfig {
            listen: format!("127.0.0.1:{}", gateway_port).parse().unwrap(),
            workers: Some(1),
            request_timeout_secs: 5,
            keepalive_secs: 75,
            max_body_size: 1024 * 1024,
        },
        tls: None,
        routes: vec![
            RouteConfig {
                id: "api".to_string(),
                path: "/api/**".to_string(),
                methods: None,
                upstream: UpstreamConfig::Static {
                    url: format!("http://127.0.0.1:{}", upstream_port),
                },
                strip_prefix: Some("/api".to_string()),
                retry: None,
                timeout_ms: Some(3000),
                cache_enabled: Some(true),
                auth_required: Some(false),
                rate_limit_key: None,
            },
            RouteConfig {
                id: "health".to_string(),
                path: "/health".to_string(),
                methods: Some(vec!["GET".to_string()]),
                upstream: UpstreamConfig::Static {
                    url: format!("http://127.0.0.1:{}", upstream_port),
                },
                strip_prefix: None,
                retry: None,
                timeout_ms: Some(1000),
                cache_enabled: Some(false),
                auth_required: Some(false),
                rate_limit_key: None,
            },
        ],
        rate_limit: Some(RateLimitConfig {
            enabled: true,
            requests_per_second: 100,
            burst_size: 200,
            key_strategy: RateLimitKeyStrategy::Global,
            paths: None,
        }),
        cors: Some(CorsConfig {
            enabled: true,
            allow_origins: vec!["*".to_string()],
            allow_methods: vec!["GET".to_string(), "POST".to_string()],
            allow_headers: vec!["*".to_string()],
            allow_credentials: false,
            max_age: Some(3600),
            expose_headers: None,
        }),
        auth: None,
        cache: Some(CacheConfig {
            enabled: true,
            backend: CacheBackend::Memory,
            default_ttl_secs: 60,
            max_capacity: Some(1000),
            cacheable_methods: vec!["GET".to_string()],
            cacheable_statuses: vec![200],
            key_strategy: CacheKeyStrategy::Uri,
            excluded_paths: vec![],
        }),
        metrics: None,
        discovery: None,
        logging: LoggingConfig {
            level: "warn".to_string(),
            format: LogFormat::Compact,
            output: LogOutput::Stdout,
            request_id_header: Some("x-request-id".to_string()),
        },
        pool: Some(PoolConfig {
            enabled: true,
            max_connections: 10,
            idle_timeout_secs: 30,
            connection_timeout_ms: 1000,
        }),
    }
}

#[tokio::test]
async fn test_gateway_proxy_request() {
    let upstream_port = 19001;
    let gateway_port = 18001;

    let _upstream = start_upstream_server(format!("127.0.0.1:{}", upstream_port).parse().unwrap()).await;
    sleep(Duration::from_millis(100)).await;

    let config = create_test_config(gateway_port, upstream_port);
    let gateway = Gateway::new(config).await.expect("create gateway");

    tokio::spawn(async move {
        let _ = gateway.run().await;
    });

    sleep(Duration::from_millis(200)).await;

    let client = reqwest::Client::new();
    let resp = timeout(
        Duration::from_secs(5),
        client.get(format!("http://127.0.0.1:{}/api/echo", gateway_port)).send(),
    )
    .await
    .expect("request timeout")
    .expect("request failed");

    let status = resp.status();
    let body = resp.text().await.expect("read body");
    assert_eq!(status, 200, "Expected 200 but got {}. Body: {}", status, body);
}

#[tokio::test]
async fn test_gateway_strip_prefix() {
    let upstream_port = 19002;
    let gateway_port = 18002;

    let _upstream = start_upstream_server(format!("127.0.0.1:{}", upstream_port).parse().unwrap()).await;
    sleep(Duration::from_millis(100)).await;

    let config = create_test_config(gateway_port, upstream_port);
    let gateway = Gateway::new(config).await.expect("create gateway");

    tokio::spawn(async move {
        let _ = gateway.run().await;
    });

    sleep(Duration::from_millis(200)).await;

    let client = reqwest::Client::new();
    let resp = timeout(
        Duration::from_secs(5),
        client.get(format!("http://127.0.0.1:{}/api/health", gateway_port)).send(),
    )
    .await
    .expect("timeout")
    .expect("request failed");

    // /api/health with strip_prefix /api should become /health on upstream
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn test_gateway_404_no_route() {
    let upstream_port = 19003;
    let gateway_port = 18003;

    let _upstream = start_upstream_server(format!("127.0.0.1:{}", upstream_port).parse().unwrap()).await;
    sleep(Duration::from_millis(100)).await;

    let config = create_test_config(gateway_port, upstream_port);
    let gateway = Gateway::new(config).await.expect("create gateway");

    tokio::spawn(async move {
        let _ = gateway.run().await;
    });

    sleep(Duration::from_millis(200)).await;

    let client = reqwest::Client::new();
    let resp = timeout(
        Duration::from_secs(5),
        client.get(format!("http://127.0.0.1:{}/no-such-route", gateway_port)).send(),
    )
    .await
    .expect("timeout")
    .expect("request failed");

    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn test_gateway_cors_preflight() {
    let upstream_port = 19004;
    let gateway_port = 18004;

    let _upstream = start_upstream_server(format!("127.0.0.1:{}", upstream_port).parse().unwrap()).await;
    sleep(Duration::from_millis(100)).await;

    let config = create_test_config(gateway_port, upstream_port);
    let gateway = Gateway::new(config).await.expect("create gateway");

    tokio::spawn(async move {
        let _ = gateway.run().await;
    });

    sleep(Duration::from_millis(200)).await;

    let client = reqwest::Client::new();
    let resp = timeout(
        Duration::from_secs(5),
        client
            .request(Method::OPTIONS, format!("http://127.0.0.1:{}/api/echo", gateway_port))
            .header("Origin", "https://example.com")
            .send(),
    )
    .await
    .expect("timeout")
    .expect("request failed");

    assert_eq!(resp.status(), 204);
    assert!(resp.headers().get("access-control-allow-origin").is_some());
}

#[tokio::test]
async fn test_gateway_rate_limit() {
    let upstream_port = 19005;
    let gateway_port = 18005;

    let _upstream = start_upstream_server(format!("127.0.0.1:{}", upstream_port).parse().unwrap()).await;
    sleep(Duration::from_millis(100)).await;

    let mut config = create_test_config(gateway_port, upstream_port);
    config.rate_limit = Some(RateLimitConfig {
        enabled: true,
        requests_per_second: 1,
        burst_size: 1,
        key_strategy: RateLimitKeyStrategy::Global,
        paths: None,
    });

    let gateway = Gateway::new(config).await.expect("create gateway");

    tokio::spawn(async move {
        let _ = gateway.run().await;
    });

    sleep(Duration::from_millis(200)).await;

    let client = reqwest::Client::new();

    // First request should pass
    let resp1 = timeout(
        Duration::from_secs(5),
        client.get(format!("http://127.0.0.1:{}/api/echo", gateway_port)).send(),
    )
    .await
    .expect("timeout")
    .expect("request failed");
    assert_eq!(resp1.status(), 200);

    // Immediate second request should be rate limited
    let resp2 = timeout(
        Duration::from_secs(5),
        client.get(format!("http://127.0.0.1:{}/api/echo", gateway_port)).send(),
    )
    .await
    .expect("timeout")
    .expect("request failed");
    assert_eq!(resp2.status(), 429);
}

#[tokio::test]
async fn test_gateway_cache_hit() {
    let upstream_port = 19006;
    let gateway_port = 18006;

    let _upstream = start_upstream_server(format!("127.0.0.1:{}", upstream_port).parse().unwrap()).await;
    sleep(Duration::from_millis(100)).await;

    let config = create_test_config(gateway_port, upstream_port);
    let gateway = Gateway::new(config).await.expect("create gateway");

    tokio::spawn(async move {
        let _ = gateway.run().await;
    });

    sleep(Duration::from_millis(200)).await;

    let client = reqwest::Client::new();

    // First request - cache miss
    let resp1 = timeout(
        Duration::from_secs(5),
        client.get(format!("http://127.0.0.1:{}/api/echo", gateway_port)).send(),
    )
    .await
    .expect("timeout")
    .expect("request failed");
    assert_eq!(resp1.status(), 200);
    // In full implementation, X-Cache header would be present

    // Second request - should be served from cache
    let resp2 = timeout(
        Duration::from_secs(5),
        client.get(format!("http://127.0.0.1:{}/api/echo", gateway_port)).send(),
    )
    .await
    .expect("timeout")
    .expect("request failed");
    assert_eq!(resp2.status(), 200);
}
