//! Proxy listener: accepts TCP connections and routes to tunnels.

use std::sync::Arc;

use crate::metrics::collector::Metrics;
use crate::proxy::connect::route_upstream;
use crate::proxy::request;
use crate::routing::engine::RouteEngine;
use crate::shared::types::Destination;
use crate::upstream::connection::Connection;
use crate::upstream::manager::UpstreamManager;

use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::io::AsyncReadExt;

/// A TCP forward-proxy that routes each CONNECT/GET request through the route
/// engine and tunnels the resulting transport stream.
pub struct Proxy {
    engine: Arc<RouteEngine>,
    manager: Arc<UpstreamManager>,
    metrics: Arc<Metrics>,
}

impl Proxy {
    /// Create a new proxy from the route engine, upstream manager, and metrics.
    pub fn new(
        engine: Arc<RouteEngine>,
        manager: Arc<UpstreamManager>,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            engine,
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
            let engine = self.engine.clone();
            let manager = self.manager.clone();
            let metrics = self.metrics.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_client(stream, engine, manager, metrics).await {
                    tracing::warn!("client connection failed: {e}");
                }
            });
        }
    }
}

/// Handle a single client connection: read the request, route the destination,
/// connect through the route engine, and tunnel the data bidirectionally.
async fn handle_client(
    mut stream: TcpStream,
    engine: Arc<RouteEngine>,
    manager: Arc<UpstreamManager>,
    metrics: Arc<Metrics>,
) -> Result<(), String> {
    let req = read_request(&mut stream).await?;
    let (host, port) = request::parse_connect(&req)?;
    let dest = Destination { host: host.clone(), port };
    tracing::debug!("client -> route: {host}:{}", port);

    metrics.increment_active();
    let conn = route_upstream(&dest, engine, manager)
        .await
        .map_err(|e| e.to_string())?;
    metrics.increment_opened();

    tunnel(stream, conn).await?;
    metrics.decrement_active();
    Ok(())
}

/// Read the first line of the HTTP request.
async fn read_request(stream: &mut TcpStream) -> Result<String, String> {
    let mut buf = [0u8; 4096];
    let n = stream
        .read(&mut buf)
        .await
        .map_err(|e| format!("read request: {e}"))?;
    Ok(String::from_utf8_lossy(&buf[..n]).into_owned())
}

/// Tunnel data bidirectionally between the client and the destination
/// connection.
async fn tunnel(client: TcpStream, conn: Box<dyn Connection>) -> Result<(), String> {
    use tokio::io;

    // Split each stream into independent read/write halves so both
    // directions run concurrently.
    let (mut client_read, mut client_write) = io::split(client);
    let (mut conn_read, mut conn_write) = io::split(conn);

    // client -> conn
    let bytes_client_to_conn = io::copy(&mut client_read, &mut conn_write)
        .await
        .map_err(|e| e.to_string())?;
    // conn -> client
    let bytes_conn_to_client = io::copy(&mut conn_read, &mut client_write)
        .await
        .map_err(|e| e.to_string())?;

    tracing::debug!(
        "tunnel complete: client->conn {} bytes, conn->client {} bytes",
        bytes_client_to_conn,
        bytes_conn_to_client
    );
    Ok(())
}
