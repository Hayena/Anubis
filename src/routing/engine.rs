//! Route engine: evaluation of rules and routing decisions.

use crate::routing::decision::RoutingDecision;
use crate::shared::types::Destination;
use crate::routing::matcher::RuleMatcher;

/// Route engine: evaluation of rules and routing decisions.
///
/// The engine iterates rules in order, first match wins.
/// Resolution chain (transparent to rules):
///
/// ```text
/// Direct              → DirectConn
/// ViaProxy("office")  → UpstreamManager::resolve("office") → connector
/// ViaProxy("nope")    → Err(UpstreamError::UnknownUpstream) → 502 to the client
/// ```
///
/// Health does not change which rule matches; it is enforced when connecting
/// (see `proxy::connect::connect_upstream`).
pub struct RouteEngine {
    matcher: RuleMatcher,
}

impl RouteEngine {
    /// Create a new route engine from configured rules.
    pub fn new(rules: Vec<crate::config::models::Rule>, default: crate::config::models::RuleTarget) -> Self {
        Self {
            matcher: RuleMatcher::new(rules, default),
        }
    }

    /// Evaluate the destination against the rules and return a routing decision.
    pub fn select(&self, dest: &Destination) -> RoutingDecision {
        self.matcher.select(dest)
    }
}
