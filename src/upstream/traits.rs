//! Upstream trait definitions.

use crate::shared::types::Destination;
use crate::upstream::connection::Connection;

use async_trait::async_trait;

/// A transport connector for an upstream.
///
/// Implementations establish a connection to a target and return a `Connection`
/// that provides async I/O over the transport.
#[async_trait]
pub trait UpstreamConnector: Send + Sync {
    async fn connect(&self, target: &Destination) -> Result<Box<dyn Connection>, String>;
}
