use bytes::Bytes;
use http::{header, Request, Response, StatusCode};
use http_body_util::Full;
use hyper::body::Incoming;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::{Layer, Service};

#[derive(Clone)]
pub struct CorsLayer {
    allow_origins: Vec<String>,
    allow_methods: Vec<String>,
    allow_headers: Vec<String>,
    allow_credentials: bool,
    max_age: Option<u64>,
    expose_headers: Vec<String>,
}

impl CorsLayer {
    pub fn new(
        allow_origins: Vec<String>,
        allow_methods: Vec<String>,
        allow_headers: Vec<String>,
        allow_credentials: bool,
        max_age: Option<u64>,
        expose_headers: Option<Vec<String>>,
    ) -> Self {
        Self {
            allow_origins,
            allow_methods,
            allow_headers,
            allow_credentials,
            max_age,
            expose_headers: expose_headers.unwrap_or_default(),
        }
    }

    pub fn is_origin_allowed(&self, origin: &str) -> bool {
        self.allow_origins.iter().any(|o| o == "*" || o == origin)
    }

    pub fn build_preflight_response(&self, origin: &str) -> Response<Full<Bytes>> {
        let mut builder = Response::builder()
            .status(StatusCode::NO_CONTENT)
            .header("Access-Control-Allow-Origin", origin);

        if self.allow_credentials {
            builder = builder.header("Access-Control-Allow-Credentials", "true");
        }

        let methods = self.allow_methods.join(", ");
        builder = builder.header("Access-Control-Allow-Methods", methods);

        let headers = self.allow_headers.join(", ");
        builder = builder.header("Access-Control-Allow-Headers", headers);

        if let Some(max_age) = self.max_age {
            builder = builder.header("Access-Control-Max-Age", max_age.to_string());
        }

        if !self.expose_headers.is_empty() {
            let expose = self.expose_headers.join(", ");
            builder = builder.header("Access-Control-Expose-Headers", expose);
        }

        builder.body(Full::new(Bytes::new())).unwrap()
    }

    pub fn add_cors_headers(&self, response: &mut Response<Full<Bytes>>, origin: &str) {
        response.headers_mut().insert(
            "Access-Control-Allow-Origin",
            header::HeaderValue::from_str(origin).unwrap(),
        );

        if self.allow_credentials {
            response.headers_mut().insert(
                "Access-Control-Allow-Credentials",
                header::HeaderValue::from_static("true"),
            );
        }

        if !self.expose_headers.is_empty() {
            let expose = self.expose_headers.join(", ");
            response.headers_mut().insert(
                "Access-Control-Expose-Headers",
                header::HeaderValue::from_str(&expose).unwrap(),
            );
        }
    }
}

impl<S> Layer<S> for CorsLayer {
    type Service = CorsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CorsService {
            inner,
            config: self.clone(),
        }
    }
}

#[derive(Clone)]
pub struct CorsService<S> {
    inner: S,
    config: CorsLayer,
}

impl<S, B> Service<Request<B>> for CorsService<S>
where
    S: Service<Request<B>, Response = Response<Full<Bytes>>>,
    S::Future: Send + 'static,
    B: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        let origin = req
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        // Handle preflight requests
        if req.method() == http::Method::OPTIONS {
            if self.config.is_origin_allowed(&origin) {
                let response = self.config.build_preflight_response(&origin);
                return Box::pin(async move { Ok(response) });
            } else {
                let response = Response::builder()
                    .status(StatusCode::FORBIDDEN)
                    .body(Full::new(Bytes::from("CORS origin not allowed")))
                    .unwrap();
                return Box::pin(async move { Ok(response) });
            }
        }

        let config = self.config.clone();
        let future = self.inner.call(req);

        Box::pin(async move {
            let mut response = future.await?;

            if !origin.is_empty() && config.is_origin_allowed(&origin) {
                config.add_cors_headers(&mut response, &origin);
            }

            Ok(response)
        })
    }
}
