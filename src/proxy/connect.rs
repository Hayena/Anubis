//! Upstream routing for a destination.

use crate::health::store::HealthStore;
use crate::routing::decision::RoutingDecision;
use crate::shared::error::{ConnectError, RouterError, UpstreamError};
use crate::shared::types::Destination;
use crate::upstream::connection::Connection;
use crate::upstream::direct::DirectConn;
use crate::upstream::manager::UpstreamManager;
use crate::upstream::traits::UpstreamConnector;

/// Establish the transport connection for an already-made routing decision.
///
/// * `Direct` uses the direct transport (`DirectConn`) — no upstream proxy.
/// * `ViaProxy(name)` uses the named upstream; an unknown name fails closed,
///   and so does an upstream whose last health check failed (fast 502 instead
///   of waiting for a connect timeout). Never falls back to a direct connection.
pub async fn connect_upstream(
    decision: &RoutingDecision,
    dest: &Destination,
    manager: &UpstreamManager,
    health: &dyn HealthStore,
) -> Result<Box<dyn Connection>, RouterError> {
    let conn = match decision {
        RoutingDecision::Direct => DirectConn.connect(dest).await,
        RoutingDecision::ViaProxy(name) => {
            let connector = manager.resolve(name)?;
            if health.is_down(name) {
                return Err(UpstreamError::Unhealthy(name.clone()).into());
            }
            connector.connect(dest).await
        }
    };
    conn.map_err(|e: ConnectError| RouterError::Upstream(e.into()))
}
