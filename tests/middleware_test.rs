use bytes::Bytes;
use http::{header, Method, Request, Response, StatusCode};
use http_body_util::Full;
use mini_api_gateway::config::{AuthConfig, AuthProvider, CorsConfig, RateLimitConfig, RateLimitKeyStrategy};
use mini_api_gateway::middleware::auth::AuthLayer;
use mini_api_gateway::middleware::cors::CorsLayer;
use mini_api_gateway::middleware::rate_limit::RateLimitLayer;
use std::future::{ready, Ready};
use std::task::{Context, Poll};
use tower::{Layer, Service};

// A simple mock service for testing middleware
#[derive(Clone)]
struct MockService;

impl<B> Service<Request<B>> for MockService {
    type Response = Response<Full<Bytes>>;
    type Error = std::convert::Infallible;
    type Future = Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _req: Request<B>) -> Self::Future {
        ready(Ok(Response::builder()
            .status(200)
            .body(Full::new(Bytes::from("OK")))
            .unwrap()))
    }
}

fn make_request(method: Method, path: &str) -> Request<()> {
    Request::builder()
        .method(method)
        .uri(path)
        .body(())
        .unwrap()
}

#[tokio::test]
async fn test_cors_preflight_request() {
    let cors_layer = CorsLayer::new(
        vec!["https://example.com".to_string()],
        vec!["GET".to_string(), "POST".to_string()],
        vec!["Content-Type".to_string()],
        false,
        Some(3600),
        None,
    );

    let mut service = cors_layer.layer(MockService);
    let req = Request::builder()
        .method(Method::OPTIONS)
        .uri("/api/test")
        .header(header::ORIGIN, "https://example.com")
        .body(())
        .unwrap();

    let response = service.call(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(response.headers().get("Access-Control-Allow-Origin").is_some());
}

#[tokio::test]
async fn test_cors_preflight_denied() {
    let cors_layer = CorsLayer::new(
        vec!["https://example.com".to_string()],
        vec!["GET".to_string()],
        vec!["*".to_string()],
        false,
        Some(3600),
        None,
    );

    let mut service = cors_layer.layer(MockService);
    let req = Request::builder()
        .method(Method::OPTIONS)
        .uri("/api/test")
        .header(header::ORIGIN, "https://evil.com")
        .body(())
        .unwrap();

    let response = service.call(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_cors_adds_headers_to_response() {
    let cors_layer = CorsLayer::new(
        vec!["*".to_string()],
        vec!["GET".to_string()],
        vec!["*".to_string()],
        false,
        Some(3600),
        None,
    );

    let mut service = cors_layer.layer(MockService);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/test")
        .header(header::ORIGIN, "https://any.com")
        .body(())
        .unwrap();

    let response = service.call(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("Access-Control-Allow-Origin").unwrap(),
        "https://any.com"
    );
}

#[tokio::test]
async fn test_rate_limit_allows_requests() {
    let rate_limit_layer = RateLimitLayer::new(
        1000,
        1000,
        RateLimitKeyStrategy::Global,
    );

    let mut service = rate_limit_layer.layer(MockService);
    let req = make_request(Method::GET, "/api/test");

    let response = service.call(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_rate_limit_blocks_excess() {
    let rate_limit_layer = RateLimitLayer::new(
        1,
        1,
        RateLimitKeyStrategy::Global,
    );

    let mut service = rate_limit_layer.layer(MockService);

    // First request should pass
    let req = make_request(Method::GET, "/api/test");
    let response = service.call(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Second request immediately should be rate limited
    let req = make_request(Method::GET, "/api/test2");
    let response = service.call(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn test_jwt_auth_valid_token() {
    let auth_config = AuthConfig {
        enabled: true,
        provider: AuthProvider::Jwt {
            secret: "test-secret".to_string(),
            issuer: Some("test".to_string()),
            audience: Some("api".to_string()),
        },
        excluded_paths: vec![],
    };

    let auth_layer = AuthLayer::new(&auth_config).expect("create auth layer");

    // Create a valid JWT token
    use jsonwebtoken::{encode, EncodingKey, Header};
    use mini_api_gateway::middleware::auth::Claims;
    use std::time::{SystemTime, UNIX_EPOCH};

    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as usize;
    let claims = Claims {
        sub: "user123".to_string(),
        exp: now + 3600,
        iat: now,
        iss: Some("test".to_string()),
        aud: Some("api".to_string()),
        extra: Default::default(),
    };

    let token = encode(&Header::default(), &claims, &EncodingKey::from_secret("test-secret".as_bytes())).unwrap();

    let mut service = auth_layer.layer(MockService);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/protected")
        .header(header::AUTHORIZATION, format!("Bearer {}", token))
        .body(())
        .unwrap();

    let response = service.call(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_jwt_auth_missing_token() {
    let auth_config = AuthConfig {
        enabled: true,
        provider: AuthProvider::Jwt {
            secret: "test-secret".to_string(),
            issuer: None,
            audience: None,
        },
        excluded_paths: vec![],
    };

    let auth_layer = AuthLayer::new(&auth_config).expect("create auth layer");
    let mut service = auth_layer.layer(MockService);

    let req = make_request(Method::GET, "/api/protected");
    let response = service.call(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_jwt_auth_excluded_path() {
    let auth_config = AuthConfig {
        enabled: true,
        provider: AuthProvider::Jwt {
            secret: "test-secret".to_string(),
            issuer: None,
            audience: None,
        },
        excluded_paths: vec!["/health".to_string()],
    };

    let auth_layer = AuthLayer::new(&auth_config).expect("create auth layer");
    let mut service = auth_layer.layer(MockService);

    let req = make_request(Method::GET, "/health");
    let response = service.call(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_jwt_auth_invalid_token() {
    let auth_config = AuthConfig {
        enabled: true,
        provider: AuthProvider::Jwt {
            secret: "test-secret".to_string(),
            issuer: None,
            audience: None,
        },
        excluded_paths: vec![],
    };

    let auth_layer = AuthLayer::new(&auth_config).expect("create auth layer");
    let mut service = auth_layer.layer(MockService);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/protected")
        .header(header::AUTHORIZATION, "Bearer invalid-token")
        .body(())
        .unwrap();

    let response = service.call(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
