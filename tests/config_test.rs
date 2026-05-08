use mini_api_gateway::config::*;

fn base_config() -> String {
    r#"server:
  listen: "127.0.0.1:3000"
  workers: 2
  request_timeout_secs: 60
  keepalive_secs: 120
  max_body_size: 5242880

logging:
  level: "info"
  format: json
  output: stdout

routes: []
"#.to_string()
}

#[test]
fn test_default_gateway_config() {
    let config = GatewayConfig::default();
    assert_eq!(config.server.listen.to_string(), "0.0.0.0:8080");
    assert_eq!(config.server.request_timeout_secs, 30);
    assert_eq!(config.server.max_body_size, 10 * 1024 * 1024);
}

#[test]
fn test_load_yaml_config() {
    let yaml = r#"
server:
  listen: "127.0.0.1:3000"
  workers: 2
  request_timeout_secs: 60
  keepalive_secs: 120
  max_body_size: 5242880

routes:
  - id: test-api
    path: "/api/**"
    upstream:
      type: static
      url: "http://localhost:8080"
    strip_prefix: "/api"
    cache_enabled: true

rate_limit:
  enabled: true
  requests_per_second: 50
  burst_size: 100
  key_strategy: ip

cors:
  enabled: true
  allow_origins:
    - "https://example.com"
  allow_methods:
    - "GET"
    - "POST"
  allow_headers:
    - "Content-Type"
    - "Authorization"
  allow_credentials: true
  max_age: 7200

metrics:
  enabled: true
  listen: "0.0.0.0:9090"
  endpoint: "/metrics"

logging:
  level: "info"
  format: json
  output: stdout
"#;

    let config = GatewayConfig::from_yaml(yaml).expect("Failed to parse YAML config");
    assert_eq!(config.server.listen.to_string(), "127.0.0.1:3000");
    assert_eq!(config.server.workers, Some(2));
    assert_eq!(config.server.max_body_size, 5242880);

    assert_eq!(config.routes.len(), 1);
    assert_eq!(config.routes[0].id, "test-api");
    assert_eq!(config.routes[0].path, "/api/**");

    let rate_limit = config.rate_limit.expect("Rate limit config missing");
    assert!(rate_limit.enabled);
    assert_eq!(rate_limit.requests_per_second, 50);
    assert_eq!(rate_limit.burst_size, 100);
    match rate_limit.key_strategy {
        RateLimitKeyStrategy::Ip => {}
        _ => panic!("Expected Ip key strategy"),
    }

    let cors = config.cors.expect("CORS config missing");
    assert!(cors.enabled);
    assert_eq!(cors.allow_origins, vec!["https://example.com"]);
    assert!(cors.allow_credentials);
    assert_eq!(cors.max_age, Some(7200));
}

#[test]
fn test_route_config_variants() {
    let yaml = r#"
server:
  listen: "127.0.0.1:3000"
  workers: 2
  request_timeout_secs: 60
  keepalive_secs: 120
  max_body_size: 5242880

routes:
  - id: static-route
    path: "/static/**"
    upstream:
      type: static
      url: "http://backend:8080"

  - id: service-route
    path: "/svc/**"
    upstream:
      type: service
      name: "my-service"
      discovery: "consul"

  - id: lb-route
    path: "/lb/**"
    upstream:
      type: load_balance
      endpoints:
        - "http://backend1:8080"
        - "http://backend2:8080"
      strategy: round_robin

logging:
  level: "info"
  format: json
  output: stdout
"#;

    let config = GatewayConfig::from_yaml(yaml).expect("parse yaml");
    assert_eq!(config.routes.len(), 3);

    match &config.routes[0].upstream {
        UpstreamConfig::Static { url } => assert_eq!(url, "http://backend:8080"),
        _ => panic!("Expected static upstream"),
    }

    match &config.routes[1].upstream {
        UpstreamConfig::Service { name, discovery } => {
            assert_eq!(name, "my-service");
            assert_eq!(discovery, "consul");
        }
        _ => panic!("Expected service upstream"),
    }

    match &config.routes[2].upstream {
        UpstreamConfig::LoadBalance { endpoints, strategy } => {
            assert_eq!(endpoints.len(), 2);
            match strategy {
                LoadBalanceStrategy::RoundRobin => {}
                _ => panic!("Expected round_robin strategy"),
            }
        }
        _ => panic!("Expected load_balance upstream"),
    }
}

#[test]
fn test_auth_config_jwt() {
    let yaml = r#"
server:
  listen: "127.0.0.1:3000"
  workers: 2
  request_timeout_secs: 60
  keepalive_secs: 120
  max_body_size: 5242880

routes: []

auth:
  enabled: true
  type: jwt
  secret: "super-secret"
  issuer: "gateway"
  audience: "api"
  excluded_paths:
    - "/health"
    - "/api/public/**"

logging:
  level: "info"
  format: json
  output: stdout
"#;

    let config = GatewayConfig::from_yaml(yaml).expect("parse yaml");
    let auth = config.auth.expect("auth config missing");
    assert!(auth.enabled);
    match auth.provider {
        AuthProvider::Jwt { secret, issuer, audience } => {
            assert_eq!(secret, "super-secret");
            assert_eq!(issuer, Some("gateway".to_string()));
            assert_eq!(audience, Some("api".to_string()));
        }
        _ => panic!("Expected JWT auth provider"),
    }
    assert_eq!(auth.excluded_paths, vec!["/health", "/api/public/**"]);
}

#[test]
fn test_cache_config_memory() {
    let yaml = r#"
server:
  listen: "127.0.0.1:3000"
  workers: 2
  request_timeout_secs: 60
  keepalive_secs: 120
  max_body_size: 5242880

routes: []

cache:
  enabled: true
  type: memory
  default_ttl_secs: 120
  max_capacity: 5000
  cacheable_methods:
    - "GET"
  cacheable_statuses:
    - 200
  key_strategy: uri_with_method
  excluded_paths: []

logging:
  level: "info"
  format: json
  output: stdout
"#;

    let config = GatewayConfig::from_yaml(yaml).expect("parse yaml");
    let cache = config.cache.expect("cache config missing");
    assert!(cache.enabled);
    match cache.backend {
        CacheBackend::Memory => {}
        _ => panic!("Expected memory backend"),
    }
    assert_eq!(cache.default_ttl_secs, 120);
    assert_eq!(cache.max_capacity, Some(5000));
}

#[test]
fn test_discovery_config() {
    let yaml_consul = r#"
server:
  listen: "127.0.0.1:3000"
  workers: 2
  request_timeout_secs: 60
  keepalive_secs: 120
  max_body_size: 5242880

routes: []

discovery:
  enabled: true
  type: consul
  address: "http://localhost:8500"
  datacenter: "dc1"
  refresh_interval_secs: 30

logging:
  level: "info"
  format: json
  output: stdout
"#;

    let config = GatewayConfig::from_yaml(yaml_consul).expect("parse yaml");
    let disc = config.discovery.expect("discovery config missing");
    assert!(disc.enabled);
    match disc.provider {
        DiscoveryProvider::Consul { address, datacenter } => {
            assert_eq!(address, "http://localhost:8500");
            assert_eq!(datacenter, Some("dc1".to_string()));
        }
        _ => panic!("Expected Consul provider"),
    }

    let yaml_etcd = r#"
server:
  listen: "127.0.0.1:3000"
  workers: 2
  request_timeout_secs: 60
  keepalive_secs: 120
  max_body_size: 5242880

routes: []

discovery:
  enabled: true
  type: etcd
  endpoints:
    - "http://localhost:2379"
    - "http://localhost:2380"
  refresh_interval_secs: 10

logging:
  level: "info"
  format: json
  output: stdout
"#;

    let config = GatewayConfig::from_yaml(yaml_etcd).expect("parse yaml");
    let disc = config.discovery.expect("discovery config missing");
    match disc.provider {
        DiscoveryProvider::Etcd { endpoints } => {
            assert_eq!(endpoints.len(), 2);
        }
        _ => panic!("Expected Etcd provider"),
    }
}
