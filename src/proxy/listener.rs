//! Proxy listener: accepts TCP connections and routes to tunnels.

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::config::holder::ConfigHolder;
use crate::metrics::collector::Metrics;
use crate::proxy::connect::connect_upstream;
use crate::routing::decision::RoutingDecision;
use crate::proxy::request;
use crate::upstream::manager::UpstreamManager;

/// Upper bound for the client's request head.
const MAX_HEAD_BYTES: usize = 8 * 1024;

/// A TCP forward-proxy that routes each CONNECT request through the route
/// engine and tunnels the resulting transport stream.
///
/// The route engine is read from the [`ConfigHolder`] per connection, so
/// hot-reloaded rules apply to new connections immediately.
pub struct Proxy {
    config: Arc<ConfigHolder>,
    manager: Arc<UpstreamManager>,
    metrics: Arc<Metrics>,
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
    pub fn new(config: Arc<ConfigHolder>, manager: Arc<UpstreamManager>, metrics: Arc<Metrics>) -> Self {
        Self {
            config,
            manager,
            metrics,
        }
    }

    /// Bind the proxy to `addr`, accept clients, and handle each connection.
    pub async fn bind(&self, addr: std::net::SocketAddr) -> Result<(), String> {
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| format!("proxy bind failed on {addr}: {e}"))?;
        tracing::info!("proxy listener bound: {addr}");

        loop {
            let (stream, _peer) = listener
                .accept()
                .await
                .map_err(|e| format!("accept failed: {e}"))?;
            let config = Arc::clone(&self.config);
            let manager = Arc::clone(&self.manager);
            let metrics = Arc::clone(&self.metrics);
            tokio::spawn(async move {
                if let Err(e) = handle_client(stream, &config, &manager, &metrics).await {
                    tracing::warn!("client connection failed: {e}");
                }
            });
        }
    }
}

/// Handle a single client connection: read the CONNECT head, route the
/// destination, connect upstream, acknowledge, and tunnel bidirectionally.
async fn handle_client(
    mut stream: TcpStream,
    config: &ConfigHolder,
    manager: &UpstreamManager,
    metrics: &Arc<Metrics>,
) -> Result<(), String> {
    let (head, leftover) = read_head(&mut stream).await?;
    let dest = request::parse_connect(&head).map_err(|e| e.to_string())?;
    tracing::debug!("client -> route: {}:{}", dest.host(), dest.port());

    let decision = config.current_engine().select(&dest);
    if let RoutingDecision::ViaProxy(name) = &decision {
        metrics.record_selection(name);
    }
    let mut conn = match connect_upstream(&decision, &dest, manager).await {
        Ok(conn) => conn,
        Err(e) => {
            metrics.increment_failed();
            let _ = stream
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                .await;
            return Err(e.to_string());
        }
    };
    metrics.increment_opened();
    let _active = ActiveGuard::new(Arc::clone(metrics));

    stream
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await
        .map_err(|e| format!("write response: {e}"))?;
    // Bytes the client sent right after the head belong to the tunnel.
    if !leftover.is_empty() {
        conn.write_all(&leftover)
            .await
            .map_err(|e| format!("forward early data: {e}"))?;
    }

    let (to_upstream, from_upstream) = tokio::io::copy_bidirectional(&mut stream, &mut conn)
        .await
        .map_err(|e| format!("tunnel: {e}"))?;
    metrics.add_bytes(from_upstream, to_upstream + leftover.len() as u64);
    tracing::debug!("tunnel complete: client->conn {to_upstream} bytes, conn->client {from_upstream} bytes");
    Ok(())
}

/// Read until the end of the request head (`\r\n\r\n`). Returns the head text
/// and any bytes received after it.
async fn read_head(stream: &mut TcpStream) -> Result<(String, Vec<u8>), String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        if let Some(end) = find_head_end(&buf) {
            let leftover = buf.split_off(end);
            return Ok((String::from_utf8_lossy(&buf).into_owned(), leftover));
        }
        if buf.len() >= MAX_HEAD_BYTES {
            return Err("request head too large".to_string());
        }
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|e| format!("read request: {e}"))?;
        if n == 0 {
            return Err("connection closed before request head completed".to_string());
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
