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
/// ViaProxy("office")  → UpstreamManager::resolve("office") → Arc<DirectConnector>
/// ViaProxy("europe")  → UpstreamManager::resolve("europe") → group strategy → Arc<ConcreteConnector>
/// ViaProxy("nope")    → Err(RouterError::UnknownUpstream)  → proxied as failed connection
/// ```
///
/// The group resolution is transparent to rules: rules only ever name a
/// proxy or a group. Strategy (failover etc.) is stored per group, not per rule.
pub struct RouteEngine {
    matcher: RuleMatcher,
}

impl RouteEngine {
    /// Create a new route engine from configured rules.
    pub fn new(rules: Vec<crate::config::models::Rule>, default: crate::config::models::DefaultRule) -> Self {
        Self {
            matcher: RuleMatcher::new(rules, default),
        }
    }

    /// Evaluate the destination against the rules and return a routing decision.
    pub fn select(&self, dest: &Destination) -> RoutingDecision {
        self.matcher.select(dest)
    }
}
