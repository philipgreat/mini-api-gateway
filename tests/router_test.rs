use http::{Request, Method};
use mini_api_gateway::config::{RouteConfig, UpstreamConfig};
use mini_api_gateway::router::Router;

fn make_request(method: Method, path: &str) -> Request<()> {
    Request::builder()
        .method(method)
        .uri(path)
        .body(())
        .unwrap()
}

#[tokio::test]
async fn test_exact_route_match() {
    let routes = vec![
        RouteConfig {
            id: "health".to_string(),
            path: "/health".to_string(),
            methods: Some(vec!["GET".to_string()]),
            upstream: UpstreamConfig::Static { url: "http://backend:8080".to_string() },
            strip_prefix: None,
            retry: None,
            timeout_ms: None,
            cache_enabled: Some(false),
            auth_required: Some(false),
            rate_limit_key: None,
        },
    ];

    let router = Router::new(routes).expect("create router");

    let req = make_request(Method::GET, "/health");
    let matched = router.match_route(&req).await;
    assert!(matched.is_some(), "Should match /health");
    let m = matched.unwrap();
    assert_eq!(m.route_id, "health");
    assert_eq!(m.upstream_url, "http://backend:8080");

    let req = make_request(Method::POST, "/health");
    let matched = router.match_route(&req).await;
    assert!(matched.is_none(), "Should not match POST /health");
}

#[tokio::test]
async fn test_wildcard_route_match() {
    let routes = vec![
        RouteConfig {
            id: "api".to_string(),
            path: "/api/**".to_string(),
            methods: None,
            upstream: UpstreamConfig::Static { url: "http://api:8080".to_string() },
            strip_prefix: Some("/api".to_string()),
            retry: None,
            timeout_ms: None,
            cache_enabled: Some(false),
            auth_required: Some(false),
            rate_limit_key: None,
        },
    ];

    let router = Router::new(routes).expect("create router");

    let req = make_request(Method::GET, "/api/users");
    let matched = router.match_route(&req).await;
    assert!(matched.is_some(), "Should match /api/users");
    let m = matched.unwrap();
    assert_eq!(m.route_id, "api");
    assert_eq!(m.strip_prefix, Some("/api".to_string()));

    let req = make_request(Method::POST, "/api/v1/orders/123");
    let matched = router.match_route(&req).await;
    assert!(matched.is_some(), "Should match deep path");
}

#[tokio::test]
async fn test_route_priority() {
    let routes = vec![
        RouteConfig {
            id: "specific".to_string(),
            path: "/api/health".to_string(),
            methods: None,
            upstream: UpstreamConfig::Static { url: "http://health:8080".to_string() },
            strip_prefix: None,
            retry: None,
            timeout_ms: None,
            cache_enabled: Some(false),
            auth_required: Some(false),
            rate_limit_key: None,
        },
        RouteConfig {
            id: "general".to_string(),
            path: "/api/**".to_string(),
            methods: None,
            upstream: UpstreamConfig::Static { url: "http://api:8080".to_string() },
            strip_prefix: None,
            retry: None,
            timeout_ms: None,
            cache_enabled: Some(false),
            auth_required: Some(false),
            rate_limit_key: None,
        },
    ];

    let router = Router::new(routes).expect("create router");

    let req = make_request(Method::GET, "/api/health");
    let matched = router.match_route(&req).await;
    assert!(matched.is_some());
    assert_eq!(matched.unwrap().route_id, "specific", "Specific route should match first");
}

#[tokio::test]
async fn test_no_route_match() {
    let routes = vec![
        RouteConfig {
            id: "api".to_string(),
            path: "/api/**".to_string(),
            methods: None,
            upstream: UpstreamConfig::Static { url: "http://api:8080".to_string() },
            strip_prefix: None,
            retry: None,
            timeout_ms: None,
            cache_enabled: Some(false),
            auth_required: Some(false),
            rate_limit_key: None,
        },
    ];

    let router = Router::new(routes).expect("create router");

    let req = make_request(Method::GET, "/other");
    let matched = router.match_route(&req).await;
    assert!(matched.is_none(), "Should not match unrelated path");
}

#[tokio::test]
async fn test_route_with_query_string() {
    let routes = vec![
        RouteConfig {
            id: "search".to_string(),
            path: "/search".to_string(),
            methods: None,
            upstream: UpstreamConfig::Static { url: "http://search:8080".to_string() },
            strip_prefix: None,
            retry: None,
            timeout_ms: None,
            cache_enabled: Some(false),
            auth_required: Some(false),
            rate_limit_key: None,
        },
    ];

    let router = Router::new(routes).expect("create router");

    let req = make_request(Method::GET, "/search?q=rust&page=1");
    let matched = router.match_route(&req).await;
    assert!(matched.is_some(), "Should match path ignoring query string");
}
