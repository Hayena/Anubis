//! Proxy listener: accepts TCP connections and routes to tunnels.

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::config::holder::ConfigHolder;
use crate::health::store::HealthStore;
use crate::metrics::collector::Metrics;
use crate::proxy::connect::connect_upstream;
use crate::routing::decision::RoutingDecision;
use crate::proxy::request;
use crate::shared::error::RouterError;

/// Upper bound for the client's request head.
const MAX_HEAD_BYTES: usize = 8 * 1024;

/// A TCP forward-proxy that routes each CONNECT request through the route
/// engine and tunnels the resulting transport stream.
///
/// The route engine is read from the [`ConfigHolder`] per connection, so
/// hot-reloaded rules apply to new connections immediately.
pub struct Proxy {
    config: Arc<ConfigHolder>,
    metrics: Arc<Metrics>,
    health: Arc<dyn HealthStore>,
}

/// Why a client connection ended abnormally.
#[derive(Debug, thiserror::Error)]
enum ClientError {
    #[error("{context}: {source}")]
    Io {
        context: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("bad request: {0}")]
    BadRequest(#[from] request::ParseError),
    #[error("request head too large")]
    HeadTooLarge,
    #[error("connection closed before request head completed")]
    ClosedEarly,
    #[error(transparent)]
    Upstream(#[from] RouterError),
}

impl ClientError {
    fn io(context: &'static str) -> impl FnOnce(std::io::Error) -> Self {
        move |source| Self::Io { context, source }
    }
}

/// Keeps the active-connection gauge correct on every exit path.
struct ActiveGuard(Arc<Metrics>);

impl ActiveGuard {
    fn new(metrics: Arc<Metrics>) -> Self {
        metrics.increment_active();
        Self(metrics)
    }
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.decrement_active();
    }
}

impl Proxy {
    pub fn new(
        config: Arc<ConfigHolder>,
        metrics: Arc<Metrics>,
        health: Arc<dyn HealthStore>,
    ) -> Self {
        Self {
            config,
            metrics,
            health,
        }
    }

    /// Bind a listener on `addr` (use port 0 for an ephemeral port).
    pub async fn bind(addr: std::net::SocketAddr) -> std::io::Result<TcpListener> {
        let listener = TcpListener::bind(addr).await?;
        tracing::info!("proxy listener bound: {}", listener.local_addr()?);
        Ok(listener)
    }

    /// Accept clients on `listener` and handle each connection until accept fails.
    pub async fn serve(&self, listener: TcpListener) -> std::io::Result<()> {
        loop {
            let (stream, _peer) = listener.accept().await?;
            let config = Arc::clone(&self.config);
            let metrics = Arc::clone(&self.metrics);
            let health = Arc::clone(&self.health);
            tokio::spawn(async move {
                if let Err(e) = handle_client(stream, &config, &metrics, health.as_ref()).await {
                    tracing::warn!("client connection failed: {e}");
                }
            });
        }
    }
}

/// Handle a single client connection: read the request head (CONNECT or
/// plain HTTP), route the destination, connect upstream, and relay bytes.
async fn handle_client(
    mut stream: TcpStream,
    config: &ConfigHolder,
    metrics: &Arc<Metrics>,
    health: &dyn HealthStore,
) -> Result<(), ClientError> {
    let (head, leftover) = read_head(&mut stream).await?;
    let request = match request::parse_request(&head) {
        Ok(request) => request,
        Err(e) => {
            let _ = stream
                .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
            return Err(e.into());
        }
    };
    let dest = request.destination();
    tracing::debug!("client -> route: {}:{}", dest.host(), dest.port());

    // One snapshot per connection keeps engine and connectors consistent.
    let snapshot = config.snapshot();
    let decision = snapshot.engine.select(dest);
    if let RoutingDecision::ViaProxy(name) = &decision {
        metrics.record_selection(name);
    }
    let mut conn = match connect_upstream(&decision, dest, &snapshot.manager, health).await {
        Ok(conn) => conn,
        Err(e) => {
            metrics.increment_failed();
            let _ = stream
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                .await;
            return Err(e.into());
        }
    };
    metrics.increment_opened();
    let _active = ActiveGuard::new(Arc::clone(metrics));

    // The tunnel is acknowledged to the client; a forwarded request is
    // instead replayed to the origin (the origin's reply is relayed as is).
    let sent_head = match &request {
        request::Request::Connect(_) => {
            stream
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
                .map_err(ClientError::io("write response"))?;
            0
        }
        request::Request::Forward(forward) => {
            conn.write_all(forward.head.as_bytes())
                .await
                .map_err(ClientError::io("forward request head"))?;
            forward.head.len() as u64
        }
    };
    // Bytes the client sent right after the head (e.g. a request body).
    if !leftover.is_empty() {
        conn.write_all(&leftover)
            .await
            .map_err(ClientError::io("forward early data"))?;
    }

    let (to_upstream, from_upstream) = tokio::io::copy_bidirectional(&mut stream, &mut conn)
        .await
        .map_err(ClientError::io("tunnel"))?;
    metrics.add_bytes(from_upstream, to_upstream + leftover.len() as u64 + sent_head);
    tracing::debug!("tunnel complete: client->conn {to_upstream} bytes, conn->client {from_upstream} bytes");
    Ok(())
}

/// Read until the end of the request head (`\r\n\r\n`). Returns the head text
/// and any bytes received after it.
async fn read_head(stream: &mut TcpStream) -> Result<(String, Vec<u8>), ClientError> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        if let Some(end) = find_head_end(&buf) {
            let leftover = buf.split_off(end);
            return Ok((String::from_utf8_lossy(&buf).into_owned(), leftover));
        }
        if buf.len() >= MAX_HEAD_BYTES {
            return Err(ClientError::HeadTooLarge);
        }
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(ClientError::io("read request"))?;
        if n == 0 {
            return Err(ClientError::ClosedEarly);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// Index just past the blank line that terminates the head.
fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_end_is_found_after_blank_line() {
        let buf = b"CONNECT a:1 HTTP/1.1\r\n\r\nEXTRA";
        let end = find_head_end(buf).unwrap();
        assert_eq!(&buf[end..], b"EXTRA");
        assert_eq!(find_head_end(b"CONNECT a:1 HTTP/1.1\r\n"), None);
    }
}
