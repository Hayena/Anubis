//! Direct connection implementation.

use crate::shared::error::ConnectError;
use crate::shared::types::Destination;
use crate::upstream::connection::Connection;
use crate::upstream::traits::{UpstreamConnector, with_connect_timeout};

use async_trait::async_trait;
use tokio::net::TcpStream;

/// Direct upstream connector: connects straight to the target with no proxy.
pub struct DirectConn;

#[async_trait]
impl UpstreamConnector for DirectConn {
    async fn connect(&self, target: &Destination) -> Result<Box<dyn Connection>, ConnectError> {
        let stream = with_connect_timeout(async {
            TcpStream::connect((target.host(), target.port()))
                .await
                .map_err(ConnectError::io("connect failed"))
        })
        .await?;
        Ok(Box::new(stream))
    }
}
