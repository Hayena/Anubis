//! SOCKS5 upstream connector (RFC 1928) via tokio-socks.

use crate::shared::types::Destination;
use crate::upstream::connection::Connection;
use crate::upstream::traits::UpstreamConnector;

use async_trait::async_trait;
use tokio_socks::tcp::Socks5Stream;

/// SOCKS5 upstream connector: performs the RFC 1928 handshake and tunnels to
/// the destination through the SOCKS5 proxy.
pub struct Socks5Upstream {
    pub host: String,
    pub port: u16,
}

impl Socks5Upstream {
    pub fn new(host: &str, port: u16) -> Self {
        Self {
            host: host.to_string(),
            port,
        }
    }
}

impl Connection for Socks5Stream<tokio::net::TcpStream> {
}

#[async_trait]
impl UpstreamConnector for Socks5Upstream {
    async fn connect(&self, destination: &Destination) -> Result<Box<dyn Connection>, String> {
        // 1. Resolve and connect to the SOCKS5 proxy.
        let proxy_addr = (self.host.as_str(), self.port);

        // 2. Perform the RFC 1928 handshake and request a connection to the
        //    ultimate destination through the proxy.
        let target = (destination.host.as_str(), destination.port);
        let stream = Socks5Stream::connect(proxy_addr, target)
            .await
            .map_err(|e| format!("socks5 handshake failed: {e}"))?;

        // 3. Return the tunneled stream as the connection to the destination.
        Ok(Box::new(stream))
    }
}
