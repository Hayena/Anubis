//! Routing decision types.

/// A routing decision: route to a proxy or directly.
#[derive(Debug, Clone, PartialEq)]
pub enum RoutingDecision {
    Direct,
    ViaProxy(String),
}

impl RoutingDecision {
}
