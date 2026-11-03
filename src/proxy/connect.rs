//! CONNECT request handling and upstream routing.

use std::sync::Arc;

use crate::routing::engine::RouteEngine;
use crate::shared::types::Destination;
use crate::upstream::direct::DirectConn;
use crate::upstream::manager::UpstreamManager;
use crate::upstream::traits::UpstreamConnector;
use crate::upstream::connection::Connection;
use crate::shared::error::RouterError;

/// Route a destination to a concrete upstream connector and establish the
/// transport connection.
///
/// * `Direct` routes to the direct transport (`DirectConn`) — no upstream proxy.
/// * `ViaProxy(name)` routes through the named upstream after manager
///   resolution (health-aware, fail-closed).
pub async fn route_upstream(
    dest: &Destination,
    engine: Arc<RouteEngine>,
    manager: Arc<UpstreamManager>,
) -> Result<Box<dyn Connection>, RouterError> {
    use crate::routing::decision::RoutingDecision;

    let decision = engine.select(dest);
    let connector = match decision {
        RoutingDecision::Direct => Arc::new(DirectConn) as Arc<dyn UpstreamConnector>,
        RoutingDecision::ViaProxy(name) => manager.resolve(&name)?,
    };
    connector
        .connect(dest)
        .await
        .map_err(|e| RouterError::Decode(format!("upstream connect failed: {e}")))
}
