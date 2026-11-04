//! Upstream routing for a destination.

use crate::routing::decision::RoutingDecision;
use crate::shared::error::{ConnectError, RouterError};
use crate::shared::types::Destination;
use crate::upstream::connection::Connection;
use crate::upstream::direct::DirectConn;
use crate::upstream::manager::UpstreamManager;
use crate::upstream::traits::UpstreamConnector;

/// Establish the transport connection for an already-made routing decision.
///
/// * `Direct` uses the direct transport (`DirectConn`) — no upstream proxy.
/// * `ViaProxy(name)` uses the named upstream; an unknown name fails closed.
pub async fn connect_upstream(
    decision: &RoutingDecision,
    dest: &Destination,
    manager: &UpstreamManager,
) -> Result<Box<dyn Connection>, RouterError> {
    let conn = match decision {
        RoutingDecision::Direct => DirectConn.connect(dest).await,
        RoutingDecision::ViaProxy(name) => manager.resolve(name)?.connect(dest).await,
    };
    conn.map_err(|e: ConnectError| RouterError::Upstream(e.into()))
}
