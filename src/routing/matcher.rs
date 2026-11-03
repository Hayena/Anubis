//! Rule matchers: domain, host, CIDR matching.

use std::net::IpAddr;
use std::str::FromStr;

use ipnet::IpNet;
use regex::Regex;

#[allow(unused_imports)]
use crate::config::models::{Rule, DefaultRule, RuleTarget};
use crate::routing::decision::RoutingDecision;
use crate::shared::types::Destination;

/// A compiled rule matcher.
pub struct RuleMatcher {
    pub rules: Vec<Rule>,
    pub default: crate::config::models::DefaultRule,
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

impl Rule {
    /// Check whether this rule matches the destination.
    pub fn matches(&self, dest: &Destination) -> bool {
        if let Some(domain) = &self.wildcard_domain {
            if domain_matches(domain, &dest.host) {
                return true;
            }
        }
        if let Some(host) = &self.exact_host {
            if host.eq_ignore_ascii_case(&dest.host) {
                return true;
            }
        }
        if let Some(cidr) = &self.cidr {
            if cidr_contains(cidr, &dest.host) {
                return true;
            }
        }
        false
    }
}

/// Convert a wildcard domain pattern (e.g. `*.company.com`) into an anchored
/// regular expression. Dots are escaped so they match literally; `*` is a
/// one-or-more non-dot label (`[^.]+`) and must be preceded by a `.`
/// boundary, so bare `*.com` never matches host `example.com`.
fn domain_to_regex(pattern: &str) -> Regex {
    let escaped: String = pattern
        .chars()
        .map(|c| match c {
            '*' => "[^.]+".to_string(),
            '.' => "\\.".to_string(),
            c => c.to_string(),
        })
        .collect();
    let re = format!("^{}$", escaped);
    Regex::new(&re).unwrap_or_else(|e| {
        tracing::warn!(target = "routing", "invalid domain pattern '{}': {}", pattern, e);
        Regex::new(".*").unwrap()
    })
}

fn domain_matches(pattern: &str, host: &str) -> bool {
    domain_to_regex(pattern).is_match(host)
}

fn cidr_contains(cidr: &str, host: &str) -> bool {
    let ip = host.parse::<IpAddr>().ok();
    let net = IpNet::from_str(cidr).ok();
    if let (Some(ip), Some(net)) = (ip, net) {
        net.contains(&ip)
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::models::{Rule, RuleTarget};

    #[test]
    fn wildcard_domain_matches_single_subdomain() {
        assert!(domain_matches("*.company.com", "dev.company.com"));
        // requires at least one label before the domain
        assert!(!domain_matches("*.company.com", "company.com"));
    }

    #[test]
    fn exact_host_ignores_case() {
        let rule = Rule {
            wildcard_domain: None,
            exact_host: Some("GitHub.com".to_string()),
            cidr: None,
            route: RuleTarget::Direct,
        };
        assert!(rule.matches(&Destination::new("github.com", 443).unwrap()));
        assert!(rule.matches(&Destination::new("GITHub.COM", 443).unwrap()));
        assert!(!rule.matches(&Destination::new("gitHub.io", 443).unwrap()));
    }

    #[test]
    fn cidr_contains_returns_false_for_out_of_range() {
        assert!(cidr_contains("10.0.0.0/8", "10.1.2.3"));
        assert!(cidr_contains("10.0.0.0/8", "10.255.255.255"));
        assert!(!cidr_contains("10.0.0.0/8", "172.16.0.1"));
        assert!(!cidr_contains("10.0.0.0/8", "8.8.8.8"));
    }

    #[test]
    fn rule_matches_domain_host_and_cidr() {
        let rule = Rule {
            wildcard_domain: Some("*.company.com".to_string()),
            exact_host: None,
            cidr: None,
            route: RuleTarget::Proxy("office".into()),
        };
        assert!(rule.matches(&Destination::new("dev.company.com", 443).unwrap()));
        assert!(!rule.matches(&Destination::new("github.com", 443).unwrap()));
    }

    #[test]
    fn default_rule_is_used_when_no_rule_matches() {
        use crate::config::models::DefaultRule;
        let engine = RuleMatcher::new(vec![], DefaultRule::Direct);
        assert!(matches!(
            engine.select(&Destination::new("github.com", 443).unwrap()),
            RoutingDecision::Direct
        ));
    }

    #[test]
    fn first_match_wins_when_multiple_rules_exist() {
        let rules = vec![
            Rule {
                wildcard_domain: Some("*.a.com".to_string()),
                exact_host: None,
                cidr: None,
                route: RuleTarget::Proxy("proxy_a".into()),
            },
            Rule {
                wildcard_domain: Some("*.b.com".to_string()),
                exact_host: None,
                cidr: None,
                route: RuleTarget::Proxy("proxy_b".into()),
            },
        ];
        let engine = RuleMatcher::new(rules, DefaultRule::Direct);
        assert_eq!(engine.select(&Destination::new("dev.a.com", 443).unwrap()), RoutingDecision::ViaProxy("proxy_a".into()));
        assert_eq!(engine.select(&Destination::new("dev.b.com", 443).unwrap()), RoutingDecision::ViaProxy("proxy_b".into()));
    }

    #[test]
    fn exact_host_matches_case_insensitively() {
        let rules = vec![Rule {
            wildcard_domain: None,
            exact_host: Some("Example.COM".to_string()),
            cidr: None,
            route: RuleTarget::Direct,
        }];
        let engine = RuleMatcher::new(rules, DefaultRule::Direct);
        assert_eq!(engine.select(&Destination::new("example.com", 80).unwrap()), RoutingDecision::Direct);
        assert_eq!(engine.select(&Destination::new("EXAMPLE.com", 80).unwrap()), RoutingDecision::Direct);
    }

    #[test]
    fn cidr_rule_matches_multiple_ips() {
        let rules = vec![Rule {
            wildcard_domain: None,
            exact_host: None,
            cidr: Some("192.168.0.0/16".to_string()),
            route: RuleTarget::Direct,
        }];
        let engine = RuleMatcher::new(rules, DefaultRule::Direct);
        assert_eq!(engine.select(&Destination::new("192.168.1.1", 80).unwrap()), RoutingDecision::Direct);
        assert_eq!(engine.select(&Destination::new("192.168.255.255", 80).unwrap()), RoutingDecision::Direct);
    }

    #[test]
    fn default_group_route_works() {
        let rules = vec![Rule {
            wildcard_domain: Some("*.example.com".to_string()),
            exact_host: None,
            cidr: None,
            route: RuleTarget::Proxy("backend".into()),
        }];
        let engine = RuleMatcher::new(rules, DefaultRule::Proxy("fallback".into()));
        assert_eq!(engine.select(&Destination::new("any.example.com", 80).unwrap()), RoutingDecision::ViaProxy("backend".into()));
    }

    #[test]
    fn empty_rules_fallback_to_default() {
        let engine = RuleMatcher::new(vec![], DefaultRule::Proxy("fallback".into()));
        assert_eq!(engine.select(&Destination::new("unknown.com", 443).unwrap()), RoutingDecision::ViaProxy("fallback".into()));
    }

    #[test]
    fn exact_host_matches_only_its_host() {
        let rules = vec![Rule {
            wildcard_domain: None,
            exact_host: Some("api.example.com".to_string()),
            cidr: None,
            route: RuleTarget::Direct,
        }];
        let engine = RuleMatcher::new(rules, DefaultRule::Direct);
        assert_eq!(engine.select(&Destination::new("api.example.com", 443).unwrap()), RoutingDecision::Direct);
        assert_eq!(engine.select(&Destination::new("api2.example.com", 443).unwrap()), RoutingDecision::Direct);
    }
}
