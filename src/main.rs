use mini_api_gateway::config::GatewayConfig;
use mini_api_gateway::gateway::Gateway;
use std::env;
use tracing::{error, info};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    init_logging();

    info!("Starting Mini API Gateway...");

    // Load configuration
    let config_path = env::var("GATEWAY_CONFIG").unwrap_or_else(|_| "config.yaml".to_string());
    
    let config = if std::path::Path::new(&config_path).exists() {
        GatewayConfig::load(&config_path)?
    } else {
        info!("No config file found at {}, using defaults with demo routes", config_path);
        create_demo_config()
    };

    // Create and run gateway
    let gateway = Gateway::new(config).await?;
    gateway.run().await?;

    Ok(())
}

fn init_logging() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mini_api_gateway=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer().with_target(true))
        .init();
}

fn create_demo_config() -> GatewayConfig {
    use mini_api_gateway::config::*;
    use std::net::SocketAddr;

    GatewayConfig {
        server: ServerConfig {
            listen: "0.0.0.0:8080".parse().unwrap(),
            workers: Some(num_cpus::get()),
            request_timeout_secs: 30,
            keepalive_secs: 75,
            max_body_size: 10 * 1024 * 1024,
        },
        tls: None,
        routes: vec![
            RouteConfig {
                id: "api".to_string(),
                path: "/api/**".to_string(),
                methods: None,
                upstream: UpstreamConfig::Static {
                    url: "http://localhost:3000".to_string(),
                },
                strip_prefix: Some("/api".to_string()),
                retry: Some(RetryConfig {
                    max_attempts: 3,
                    backoff_ms: 100,
                }),
                timeout_ms: Some(30000),
                cache_enabled: Some(false),
                auth_required: Some(false),
                rate_limit_key: None,
            },
            RouteConfig {
                id: "health".to_string(),
                path: "/health".to_string(),
                methods: Some(vec!["GET".to_string()]),
                upstream: UpstreamConfig::Static {
                    url: "http://localhost:3000".to_string(),
                },
                strip_prefix: None,
                retry: None,
                timeout_ms: Some(5000),
                cache_enabled: Some(false),
                auth_required: Some(false),
                rate_limit_key: None,
            },
        ],
        rate_limit: Some(RateLimitConfig {
            enabled: true,
            requests_per_second: 100,
            burst_size: 200,
            key_strategy: RateLimitKeyStrategy::Ip,
            paths: None,
        }),
        cors: Some(CorsConfig {
            enabled: true,
            allow_origins: vec!["*".to_string()],
            allow_methods: vec![
                "GET".to_string(),
                "POST".to_string(),
                "PUT".to_string(),
                "DELETE".to_string(),
                "OPTIONS".to_string(),
            ],
            allow_headers: vec!["*".to_string()],
            allow_credentials: false,
            max_age: Some(3600),
            expose_headers: Some(vec!["X-Request-ID".to_string()]),
        }),
        auth: None,
        cache: Some(CacheConfig {
            enabled: true,
            backend: CacheBackend::Memory,
            default_ttl_secs: 300,
            max_capacity: Some(10000),
            cacheable_methods: vec!["GET".to_string()],
            cacheable_statuses: vec![200, 301, 404],
            key_strategy: CacheKeyStrategy::UriWithMethod,
            excluded_paths: vec!["/api/auth/**".to_string()],
        }),
        metrics: Some(MetricsConfig {
            enabled: true,
            listen: "0.0.0.0:9090".parse().unwrap(),
            endpoint: "/metrics".to_string(),
        }),
        discovery: None,
        logging: LoggingConfig {
            level: "info".to_string(),
            format: LogFormat::Json,
            output: LogOutput::Stdout,
            request_id_header: Some("x-request-id".to_string()),
        },
        pool: Some(PoolConfig {
            enabled: true,
            max_connections: 100,
            idle_timeout_secs: 60,
            connection_timeout_ms: 5000,
        }),
    }
}
