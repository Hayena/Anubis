//! Upstream trait definitions.

use crate::shared::error::ConnectError;
use crate::shared::types::Destination;
use crate::upstream::connection::Connection;

use async_trait::async_trait;

/// Upper bound for establishing an upstream connection (including handshakes).
pub const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// A transport connector for an upstream.
///
/// Implementations establish a connection to a target and return a `Connection`
/// that provides async I/O over the transport.
#[async_trait]
pub trait UpstreamConnector: Send + Sync {
    async fn connect(&self, target: &Destination) -> Result<Box<dyn Connection>, ConnectError>;
}

/// Run `fut` under [`CONNECT_TIMEOUT`], mapping elapsed time to `ConnectError::Timeout`.
pub async fn with_connect_timeout<T>(
    fut: impl std::future::Future<Output = Result<T, ConnectError>>,
) -> Result<T, ConnectError> {
    tokio::time::timeout(CONNECT_TIMEOUT, fut)
        .await
        .unwrap_or(Err(ConnectError::Timeout(CONNECT_TIMEOUT)))
}
