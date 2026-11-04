//! Rule matchers: domain, host, CIDR matching.
//!
//! Rules are compiled once (regex / CIDR parsed) when the matcher is built,
//! so per-connection matching does no parsing or allocation.

use std::net::IpAddr;

use ipnet::IpNet;
use regex::Regex;

use crate::config::models::{Condition, Rule, RuleTarget};
use crate::routing::decision::RoutingDecision;
use crate::shared::types::Destination;

/// A single compiled match condition.
enum Matcher {
    Domain(Regex),
    Host(String),
    Cidr(IpNet),
}

impl Matcher {
    fn matches(&self, host: &str) -> bool {
        match self {
            Matcher::Domain(re) => re.is_match(host),
            Matcher::Host(h) => h.eq_ignore_ascii_case(host),
            Matcher::Cidr(net) => host.parse::<IpAddr>().is_ok_and(|ip| net.contains(&ip)),
        }
    }
}

/// A rule whose patterns have been parsed. Conditions are OR-ed; a rule
/// without conditions is a catch-all.
struct CompiledRule {
    matchers: Vec<Matcher>,
    target: RuleTarget,
}

impl CompiledRule {
    fn matches(&self, dest: &Destination) -> bool {
        self.matchers.is_empty() || self.matchers.iter().any(|m| m.matches(dest.host()))
    }
}

fn compile(rule: &Rule) -> Result<CompiledRule, String> {
    let matchers = rule
        .conditions
        .iter()
        .map(|c| match c {
            Condition::Domain(p) => domain_to_regex(p).map(Matcher::Domain),
            Condition::Host(h) => Ok(Matcher::Host(h.clone())),
            Condition::Cidr(net) => Ok(Matcher::Cidr(*net)),
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(CompiledRule {
        matchers,
        target: rule.route.clone(),
    })
}

/// Check that a rule's patterns are well-formed (used by config validation).
pub fn validate_rule(rule: &Rule) -> Result<(), String> {
    compile(rule).map(|_| ())
}

/// Convert a wildcard domain pattern (e.g. `*.company.com`) into an anchored,
/// case-insensitive regular expression. Literal parts are escaped; `*` matches
/// one or more non-dot characters, so `*.company.com` never matches the bare
/// `company.com`.
fn domain_to_regex(pattern: &str) -> Result<Regex, String> {
    let body = pattern
        .split('*')
        .map(regex::escape)
        .collect::<Vec<_>>()
        .join("[^.]+");
    Regex::new(&format!("(?i)^{body}$"))
        .map_err(|e| format!("invalid domain pattern '{pattern}': {e}"))
}

fn decision_for(target: &RuleTarget) -> RoutingDecision {
    match target {
        RuleTarget::Direct => RoutingDecision::Direct,
        RuleTarget::Proxy(name) => RoutingDecision::ViaProxy(name.clone()),
    }
}

/// Ordered, compiled rule set. First match wins.
pub struct RuleMatcher {
    rules: Vec<CompiledRule>,
    default: RuleTarget,
}

impl RuleMatcher {
    /// Build a matcher. Rules that fail to compile are dropped (fail closed:
    /// they never match) and logged; config loading rejects them up front.
    pub fn new(rules: Vec<Rule>, default: RuleTarget) -> Self {
        let rules = rules
            .iter()
            .filter_map(|r| {
                compile(r)
                    .inspect_err(|e| tracing::warn!(target = "routing", "dropping rule: {e}"))
                    .ok()
            })
            .collect();
        Self { rules, default }
    }

    pub fn select(&self, dest: &Destination) -> RoutingDecision {
        let target = self
            .rules
            .iter()
            .find(|r| r.matches(dest))
            .map_or(&self.default, |r| &r.target);
        decision_for(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(conditions: Vec<Condition>, route: RuleTarget) -> Rule {
        Rule { conditions, route }
    }

    fn domain(p: &str) -> Condition {
        Condition::Domain(p.into())
    }

    fn to_proxy(name: &str) -> RuleTarget {
        RuleTarget::Proxy(name.into())
    }

    fn dest(host: &str) -> Destination {
        Destination::new(host, 443).unwrap()
    }

    fn proxy(name: &str) -> RoutingDecision {
        RoutingDecision::ViaProxy(name.into())
    }

    #[test]
    fn wildcard_domain_needs_a_label_and_ignores_case() {
        let m = RuleMatcher::new(
            vec![rule(vec![domain("*.company.com")], to_proxy("office"))],
            RuleTarget::Direct,
        );
        assert_eq!(m.select(&dest("dev.company.com")), proxy("office"));
        assert_eq!(m.select(&dest("DEV.Company.COM")), proxy("office"));
        assert_eq!(m.select(&dest("company.com")), RoutingDecision::Direct);
        assert_eq!(m.select(&dest("devXcompany.com")), RoutingDecision::Direct);
    }

    #[test]
    fn exact_host_ignores_case_and_matches_only_itself() {
        let m = RuleMatcher::new(
            vec![rule(vec![Condition::Host("GitHub.com".into())], to_proxy("p"))],
            RuleTarget::Direct,
        );
        assert_eq!(m.select(&dest("github.com")), proxy("p"));
        assert_eq!(m.select(&dest("github.io")), RoutingDecision::Direct);
    }

    #[test]
    fn cidr_matches_ips_in_range_only() {
        let m = RuleMatcher::new(
            vec![rule(vec![Condition::Cidr("10.0.0.0/8".parse().unwrap())], to_proxy("p"))],
            RuleTarget::Direct,
        );
        assert_eq!(m.select(&dest("10.255.255.255")), proxy("p"));
        assert_eq!(m.select(&dest("172.16.0.1")), RoutingDecision::Direct);
        assert_eq!(m.select(&dest("not-an-ip")), RoutingDecision::Direct);
    }

    #[test]
    fn conditions_are_ored_and_empty_rule_is_catch_all() {
        let m = RuleMatcher::new(
            vec![
                rule(vec![domain("*.a.com"), Condition::Host("b.org".into())], to_proxy("a")),
                rule(vec![], to_proxy("catch")),
            ],
            RuleTarget::Direct,
        );
        assert_eq!(m.select(&dest("x.a.com")), proxy("a"));
        assert_eq!(m.select(&dest("b.org")), proxy("a"));
        assert_eq!(m.select(&dest("other.net")), proxy("catch"));
    }

    #[test]
    fn first_match_wins_then_default() {
        let m = RuleMatcher::new(
            vec![
                rule(vec![domain("*.a.com")], to_proxy("a")),
                rule(vec![domain("*.a.com")], to_proxy("shadowed")),
            ],
            to_proxy("fallback"),
        );
        assert_eq!(m.select(&dest("x.a.com")), proxy("a"));
        assert_eq!(m.select(&dest("other.org")), proxy("fallback"));
    }
}
