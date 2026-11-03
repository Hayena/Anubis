//! Rule definitions and matching logic.

use crate::routing::decision::RoutingDecision;
use crate::shared::types::Destination;
use crate::config::models::{Rule, RuleTarget};

/// Evaluate a destination against a set of rules in order.
pub struct RuleMatcher {
    rules: Vec<Rule>,
    default: crate::config::models::DefaultRule,
}

impl RuleMatcher {
    pub fn new(rules: Vec<Rule>, default: crate::config::models::DefaultRule) -> Self {
        Self { rules, default }
    }

    pub fn select(&self, dest: &Destination) -> RoutingDecision {
        for rule in &self.rules {
            if rule.matches(dest) {
                return match &rule.route {
                    RuleTarget::Direct => RoutingDecision::Direct,
                    RuleTarget::Proxy(name) => RoutingDecision::ViaProxy(name.clone()),
                };
            }
        }

        match &self.default {
            crate::config::models::DefaultRule::Direct => RoutingDecision::Direct,
            crate::config::models::DefaultRule::Proxy(name) => RoutingDecision::ViaProxy(name.clone()),
        }
    }
}
