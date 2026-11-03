//! Direct connection implementation.

use crate::shared::types::Destination;
use crate::upstream::connection::Connection;
use crate::upstream::traits::UpstreamConnector;

use async_trait::async_trait;
use tokio::net::TcpStream;

/// Direct upstream connector: connects straight to the target with no proxy.
pub struct DirectConn;

#[async_trait]
impl UpstreamConnector for DirectConn {
    async fn connect(&self, target: &Destination) -> Result<Box<dyn Connection>, String> {
        let addr = format!("{}:{}", target.host, target.port);
        let stream = TcpStream::connect(&addr)
            .await
            .map_err(|e| format!("connect failed: {e}"))?;
        Ok(Box::new(stream))
    }
}
