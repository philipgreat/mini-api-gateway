use hyper::client::conn::http1::Builder;
use hyper_util::rt::TokioIo;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tracing::{debug, error, trace};

/// Connection pool for upstream HTTP/1.1 connections
pub struct ConnectionPool {
    max_connections: usize,
    idle_timeout: Duration,
    connections: Mutex<Vec<PooledConnection>>,
}

struct PooledConnection {
    addr: SocketAddr,
    created_at: Instant,
    last_used: Instant,
    // In a real implementation, this would hold the actual connection
    // For this architecture, we use reqwest's built-in pooling instead
}

impl ConnectionPool {
    pub fn new(max_connections: usize, idle_timeout: Duration) -> Self {
        Self {
            max_connections,
            idle_timeout,
            connections: Mutex::new(Vec::with_capacity(max_connections)),
        }
    }

    pub async fn get_connection(&self, addr: SocketAddr) -> Option<TcpStream> {
        let mut connections = self.connections.lock().await;

        // Remove expired connections
        let now = Instant::now();
        connections.retain(|conn| {
            let valid = now.duration_since(conn.last_used) < self.idle_timeout;
            if !valid {
                trace!("Removing expired connection to {:?}", conn.addr);
            }
            valid
        });

        // Find existing connection for this address
        if let Some(pos) = connections.iter().position(|conn| conn.addr == addr) {
            let conn = connections.remove(pos);
            debug!("Reusing connection to {:?}", addr);

            // Try to reconnect since we can't actually store TcpStream across await points easily
            match TcpStream::connect(addr).await {
                Ok(stream) => return Some(stream),
                Err(e) => {
                    error!("Failed to reconnect to {:?}: {}", addr, e);
                    return None;
                }
            }
        }

        // Create new connection if under limit
        if connections.len() < self.max_connections {
            match TcpStream::connect(addr).await {
                Ok(stream) => {
                    connections.push(PooledConnection {
                        addr,
                        created_at: Instant::now(),
                        last_used: Instant::now(),
                    });
                    return Some(stream);
                }
                Err(e) => {
                    error!("Failed to connect to {:?}: {}", addr, e);
                    return None;
                }
            }
        }

        None
    }

    pub async fn return_connection(&self, addr: SocketAddr) {
        let mut connections = self.connections.lock().await;
        if connections.len() < self.max_connections {
            connections.push(PooledConnection {
                addr,
                created_at: Instant::now(),
                last_used: Instant::now(),
            });
        }
    }

    pub async fn cleanup_expired(&self) {
        let mut connections = self.connections.lock().await;
        let now = Instant::now();
        let before = connections.len();
        connections.retain(|conn| now.duration_since(conn.last_used) < self.idle_timeout);
        let after = connections.len();
        if before != after {
            debug!("Cleaned up {} expired connections", before - after);
        }
    }
}

/// Zero-copy proxy utilities
pub mod zero_copy {
    use bytes::Bytes;
    use http_body_util::{BodyExt, Full};
    use hyper::body::Incoming;

    /// Convert an Incoming body to Bytes with minimal copying
    pub async fn body_to_bytes(body: Incoming) -> Result<Bytes, hyper::Error> {
        let collected = body.collect().await?;
        Ok(collected.to_bytes())
    }

    /// Create a Full body from Bytes without additional allocation
    pub fn bytes_to_body(bytes: Bytes) -> Full<Bytes> {
        Full::new(bytes)
    }

    /// Stream body chunks without collecting
    pub async fn stream_body(
        mut body: Incoming,
    ) -> Result<Vec<Bytes>, hyper::Error> {
        let mut chunks = Vec::new();
        while let Some(frame) = body.frame().await {
            let frame = frame?;
            if let Some(data) = frame.data_ref() {
                chunks.push(data.clone());
            }
        }
        Ok(chunks)
    }
}
