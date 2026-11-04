//! SOCKS5 upstream connector (RFC 1928) via tokio-socks.

use crate::shared::error::ConnectError;
use crate::shared::types::Destination;
use crate::upstream::connection::Connection;
use crate::upstream::traits::{UpstreamConnector, with_connect_timeout};

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

impl Connection for Socks5Stream<tokio::net::TcpStream> {}

#[async_trait]
impl UpstreamConnector for Socks5Upstream {
    async fn connect(&self, destination: &Destination) -> Result<Box<dyn Connection>, ConnectError> {
        let proxy_addr = (self.host.as_str(), self.port);
        let target = (destination.host(), destination.port());
        let stream = with_connect_timeout(async {
            Socks5Stream::connect(proxy_addr, target)
                .await
                .map_err(|e| ConnectError::Handshake(format!("socks5: {e}")))
        })
        .await?;
        Ok(Box::new(stream))
    }
}
