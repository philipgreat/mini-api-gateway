use bytes::Bytes;
use http::Request;
use http_body::Body;
use http_body_util::Full;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::{Layer, Service};
use tracing::trace;

use crate::metrics::RequestMetrics;

#[derive(Clone)]
pub struct MetricsLayer {
    route_id: String,
}

impl MetricsLayer {
    pub fn new(route_id: String) -> Self {
        Self { route_id }
    }
}

impl<S> Layer<S> for MetricsLayer {
    type Service = MetricsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MetricsService {
            inner,
            route_id: self.route_id.clone(),
        }
    }
}

#[derive(Clone)]
pub struct MetricsService<S> {
    inner: S,
    route_id: String,
}

impl<S, B> Service<Request<B>> for MetricsService<S>
where
    S: Service<Request<B>, Response = hyper::Response<Full<Bytes>>>,
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
        let method = req.method().to_string();
        let path = req.uri().path().to_string();
        let route_id = self.route_id.clone();
        let metrics = RequestMetrics::new(&method, &path, &route_id);

        let future = self.inner.call(req);

        Box::pin(async move {
            let start = std::time::Instant::now();
            let result = future.await;
            let duration = start.elapsed();

            match &result {
                Ok(response) => {
                    let status = response.status().as_u16();
                    let body_len = response.body().size_hint().exact().unwrap_or(0);
                    metrics.record_success(status, body_len);
                }
                Err(_) => {
                    trace!("Request failed for route {}", route_id);
                }
            }

            result
        })
    }
}
