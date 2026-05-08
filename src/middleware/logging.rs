use bytes::Bytes;
use http::Request;
use http_body_util::Full;
use hyper::body::Incoming;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::{Layer, Service};
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Clone)]
pub struct LoggingLayer {
    request_id_header: Option<String>,
}

impl LoggingLayer {
    pub fn new(request_id_header: Option<String>) -> Self {
        Self { request_id_header }
    }
}

impl<S> Layer<S> for LoggingLayer {
    type Service = LoggingService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        LoggingService {
            inner,
            request_id_header: self.request_id_header.clone(),
        }
    }
}

#[derive(Clone)]
pub struct LoggingService<S> {
    inner: S,
    request_id_header: Option<String>,
}

impl<S, B> Service<Request<B>> for LoggingService<S>
where
    S: Service<Request<B>, Response = hyper::Response<Full<Bytes>>>,
    S::Future: Send + 'static,
    B: std::fmt::Debug,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<B>) -> Self::Future {
        let request_id = Uuid::new_v4().to_string();
        let method = req.method().clone();
        let uri = req.uri().clone();
        let start = std::time::Instant::now();

        if let Some(ref header) = self.request_id_header {
            req.headers_mut().insert(
                http::header::HeaderName::from_bytes(header.as_bytes()).unwrap(),
                http::header::HeaderValue::from_str(&request_id).unwrap(),
            );
        }

        let request_id_log = request_id.clone();
        let future = self.inner.call(req);

        Box::pin(async move {
            let result = future.await;
            let duration = start.elapsed();

            match &result {
                Ok(response) => {
                    let status = response.status();
                    if status.is_server_error() || status.is_client_error() {
                        warn!(
                            request_id = %request_id_log,
                            method = %method,
                            uri = %uri,
                            status = %status,
                            duration_ms = %duration.as_millis(),
                            "Request completed with error status"
                        );
                    } else {
                        info!(
                            request_id = %request_id_log,
                            method = %method,
                            uri = %uri,
                            status = %status,
                            duration_ms = %duration.as_millis(),
                            "Request completed"
                        );
                    }
                }
                Err(_) => {
                    warn!(
                        request_id = %request_id_log,
                        method = %method,
                        uri = %uri,
                        duration_ms = %duration.as_millis(),
                        "Request failed"
                    );
                }
            }

            result
        })
    }
}
