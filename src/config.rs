use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    pub server: ServerConfig,
    pub tls: Option<TlsConfig>,
    pub routes: Vec<RouteConfig>,
    pub rate_limit: Option<RateLimitConfig>,
    pub cors: Option<CorsConfig>,
    pub auth: Option<AuthConfig>,
    pub cache: Option<CacheConfig>,
    pub metrics: Option<MetricsConfig>,
    pub discovery: Option<DiscoveryConfig>,
    pub logging: LoggingConfig,
    pub pool: Option<PoolConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub listen: SocketAddr,
    pub workers: Option<usize>,
    pub request_timeout_secs: u64,
    pub keepalive_secs: u64,
    pub max_body_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    pub enabled: bool,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub client_auth: Option<ClientAuthConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientAuthConfig {
    pub enabled: bool,
    pub ca_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteConfig {
    pub id: String,
    pub path: String,
    pub methods: Option<Vec<String>>,
    pub upstream: UpstreamConfig,
    pub strip_prefix: Option<String>,
    pub retry: Option<RetryConfig>,
    pub timeout_ms: Option<u64>,
    pub cache_enabled: Option<bool>,
    pub auth_required: Option<bool>,
    pub rate_limit_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UpstreamConfig {
    Static { url: String },
    Service { name: String, discovery: String },
    LoadBalance { endpoints: Vec<String>, strategy: LoadBalanceStrategy },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadBalanceStrategy {
    RoundRobin,
    Random,
    LeastConnections,
    IpHash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub backoff_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    pub enabled: bool,
    pub requests_per_second: u32,
    pub burst_size: u32,
    pub key_strategy: RateLimitKeyStrategy,
    pub paths: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitKeyStrategy {
    Global,
    Ip,
    Header(String),
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorsConfig {
    pub enabled: bool,
    pub allow_origins: Vec<String>,
    pub allow_methods: Vec<String>,
    pub allow_headers: Vec<String>,
    pub allow_credentials: bool,
    pub max_age: Option<u64>,
    pub expose_headers: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub enabled: bool,
    #[serde(flatten)]
    pub provider: AuthProvider,
    pub excluded_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthProvider {
    Jwt {
        secret: String,
        issuer: Option<String>,
        audience: Option<String>,
    },
    OAuth2 {
        client_id: String,
        client_secret: String,
        token_url: String,
        authorize_url: Option<String>,
        scopes: Option<Vec<String>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    pub enabled: bool,
    #[serde(flatten)]
    pub backend: CacheBackend,
    pub default_ttl_secs: u64,
    pub max_capacity: Option<u64>,
    pub cacheable_methods: Vec<String>,
    pub cacheable_statuses: Vec<u16>,
    pub key_strategy: CacheKeyStrategy,
    pub excluded_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CacheBackend {
    Memory,
    Redis { url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheKeyStrategy {
    Uri,
    UriWithMethod,
    UriWithHeaders(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    pub enabled: bool,
    pub listen: SocketAddr,
    pub endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryConfig {
    pub enabled: bool,
    #[serde(flatten)]
    pub provider: DiscoveryProvider,
    pub refresh_interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DiscoveryProvider {
    Consul { address: String, datacenter: Option<String> },
    Etcd { endpoints: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: LogFormat,
    pub output: LogOutput,
    pub request_id_header: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogFormat {
    Json,
    Pretty,
    Compact,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogOutput {
    Stdout,
    Stderr,
    File { path: PathBuf, rotation: Option<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolConfig {
    pub enabled: bool,
    pub max_connections: usize,
    pub idle_timeout_secs: u64,
    pub connection_timeout_ms: u64,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                listen: "0.0.0.0:8080".parse().unwrap(),
                workers: None,
                request_timeout_secs: 30,
                keepalive_secs: 75,
                max_body_size: 10 * 1024 * 1024, // 10MB
            },
            tls: None,
            routes: vec![],
            rate_limit: None,
            cors: Some(CorsConfig {
                enabled: true,
                allow_origins: vec!["*".to_string()],
                allow_methods: vec!["GET".to_string(), "POST".to_string(), "PUT".to_string(), "DELETE".to_string(), "OPTIONS".to_string()],
                allow_headers: vec!["*".to_string()],
                allow_credentials: false,
                max_age: Some(3600),
                expose_headers: None,
            }),
            auth: None,
            cache: None,
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
}

impl GatewayConfig {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let settings = config::Config::builder()
            .add_source(config::File::with_name(path).required(false))
            .add_source(config::Environment::with_prefix("GATEWAY").separator("__"))
            .build()?;

        let mut config: GatewayConfig = settings.try_deserialize()?;

        // Ensure default values are filled
        if config.server.workers.is_none() {
            config.server.workers = Some(num_cpus::get());
        }

        Ok(config)
    }

    pub fn from_yaml(content: &str) -> anyhow::Result<Self> {
        let config: GatewayConfig = serde_yaml::from_str(content)?;
        Ok(config)
    }
}
