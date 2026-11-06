//! HOCON config loading.
//!
//! Parses a HOCON file into the internal Config model using the
//! `hocon-parser` crate's typed getters.

use tracing::{info, warn};

use crate::config::models::{DEFAULT_API_PORT, DEFAULT_PROXY_PORT, Condition, Config, HealthConfig, Proxy, ProxyType, Rule, RuleTarget};
use crate::routing::matcher::validate_rule;
use crate::shared::error::{ConfigError, RouterError};

/// Load configuration from a HOCON file (also used for hot reload).
///
/// Reads the HOCON file, parses it, validates it, and returns the new
/// `Config`. The caller owns swapping the `Arc<Config>` and rebuilding the
/// routing engine. The reload does NOT drop active connections.
pub fn reload(path: &str) -> Result<Config, RouterError> {
    info!(target = "config", "Reloading config from {}", path);
    let content = read_file(path)?;
    let config = parse_hocon(&content)?;
    if config.proxies.is_empty() {
        warn!(target = "config", "No proxies configured");
    }
    info!(
        target = "config",
        "Config reloaded: {} proxies, {} rules",
        config.proxies.len(),
        config.rules.len(),
    );
    Ok(config)
}

fn read_file(path: &str) -> Result<String, RouterError> {
    std::fs::read_to_string(path)
        .map_err(|_| ConfigError::FileNotFound(path.to_string()).into())
}

fn parse_hocon(content: &str) -> Result<Config, RouterError> {
    let hocon_config = hocon::parse(content).map_err(|e| ConfigError::HoconParse(e.to_string()))?;
    let config = Config {
        proxy_port: port_or("proxyPort", hocon_config.get_i64_option("proxyPort"), DEFAULT_PROXY_PORT)?,
        api_port: port_or("apiPort", hocon_config.get_i64_option("apiPort"), DEFAULT_API_PORT)?,
        proxies: parse_proxies(&hocon_config)?,
        rules: parse_rules(&hocon_config)?,
        default: parse_default_rule(&hocon_config)?,
    };
    validate_references(&config)?;
    Ok(config)
}

fn port_or(key: &str, value: Option<i64>, default: u16) -> Result<u16, RouterError> {
    value.map_or(Ok(default), |p| valid_port(key, p))
}

fn valid_port(key: &str, value: i64) -> Result<u16, RouterError> {
    u16::try_from(value)
        .ok()
        .filter(|p| *p != 0)
        .ok_or_else(|| ConfigError::Validation(format!("{key}: port {value} out of range 1-65535")).into())
}

/// `proxies` is an object keyed by proxy name.
fn parse_proxies(config: &hocon::Config) -> Result<Vec<Proxy>, RouterError> {
    let Some(value) = config.get("proxies") else {
        return Ok(Vec::new());
    };
    let entries = value
        .as_object()
        .ok_or_else(|| ConfigError::Validation("proxies must be an object keyed by proxy name".into()))?;
    entries
        .iter()
        .map(|(name, value)| parse_proxy_value(name, value))
        .collect()
}

fn parse_proxy_value(name: &str, value: &hocon::HoconValue) -> Result<Proxy, RouterError> {
    let obj = value
        .as_object()
        .ok_or_else(|| ConfigError::Validation(format!("proxies.{name} must be an object")))?;
    let str_field = |key: &str| obj.get(key).and_then(hocon::HoconValue::as_str);

    let proxy_type = parse_proxy_type(str_field("type").ok_or(ConfigError::MissingProxyType)?)?;
    let host = str_field("host").ok_or(ConfigError::MissingProxyHost)?;
    let port = obj
        .get("port")
        .and_then(hocon::HoconValue::as_i64)
        .ok_or(ConfigError::MissingProxyPort)
        .map_err(RouterError::from)
        .and_then(|p| valid_port(&format!("proxies.{name}.port"), p))?;
    let health = obj
        .get("health")
        .map_or(Ok(HealthConfig::default()), |h| parse_health(name, h))?;

    Ok(Proxy {
        name: name.to_string(),
        r#type: proxy_type,
        host: host.to_string(),
        port,
        health,
    })
}

/// Health settings as HOCON durations (e.g. `30s`); must be positive.
fn parse_health(proxy: &str, value: &hocon::HoconValue) -> Result<HealthConfig, RouterError> {
    let obj = value.as_object().ok_or(ConfigError::InvalidHealthConfig)?;
    let health = hocon::Config::new(obj.clone());
    let duration = |key: &str, default: std::time::Duration| {
        if !health.has(key) {
            return Ok(default);
        }
        health
            .get_duration(key)
            .ok()
            .filter(|d| !d.is_zero())
            .ok_or_else(|| {
                ConfigError::Validation(format!("proxies.{proxy}.health.{key}: expected a positive duration like 30s"))
            })
    };
    let defaults = HealthConfig::default();
    Ok(HealthConfig {
        interval: duration("interval", defaults.interval)?,
        timeout: duration("timeout", defaults.timeout)?,
    })
}

fn parse_proxy_type(value: &str) -> Result<ProxyType, RouterError> {
    match value {
        "http" => Ok(ProxyType::Http),
        "socks5" => Ok(ProxyType::Socks5),
        other => Err(ConfigError::Validation(format!("unknown proxy type: {other}")).into()),
    }
}

fn parse_rules(config: &hocon::Config) -> Result<Vec<Rule>, RouterError> {
    let hocon_list = config
        .get_list_option("rules")
        .unwrap_or_default();
    hocon_list
        .into_iter()
        .map(parse_rule_value)
        .collect()
}

fn parse_rule_value(value: hocon::HoconValue) -> Result<Rule, RouterError> {
    let obj = value.as_object();
    let str_field = |key: &str| {
        obj.and_then(|o| o.get(key))
            .and_then(hocon::HoconValue::as_str)
    };

    let cidr = str_field("cidr")
        .map(|c| {
            c.parse()
                .map(Condition::Cidr)
                .map_err(|e| ConfigError::Validation(format!("invalid cidr '{c}': {e}")))
        })
        .transpose()?;
    let conditions = str_field("domain")
        .map(|d| Condition::Domain(d.to_string()))
        .into_iter()
        .chain(str_field("host").map(|h| Condition::Host(h.to_string())))
        .chain(cidr)
        .collect();

    let route = obj
        .and_then(|o| o.get("route"))
        .and_then(hocon::HoconValue::as_str)
        .and_then(parse_target)
        .ok_or(ConfigError::MissingRoute)?;

    let rule = Rule { conditions, route };
    validate_rule(&rule).map_err(ConfigError::Validation)?;
    Ok(rule)
}

fn parse_target(s: &str) -> Option<RuleTarget> {
    if s.eq_ignore_ascii_case("direct") {
        return Some(RuleTarget::Direct);
    }
    // "proxy:name" or a bare proxy name
    match s.split_once(':') {
        Some(("proxy", name)) if !name.is_empty() => Some(RuleTarget::Proxy(name.to_string())),
        Some(_) => None,
        None => Some(RuleTarget::Proxy(s.to_string())),
    }
}

/// Every proxy named by a rule or the default rule must be configured.
fn validate_references(config: &Config) -> Result<(), RouterError> {
    let known = |name: &str| config.proxies.iter().any(|p| p.name == name);
    config
        .rules
        .iter()
        .map(|r| &r.route)
        .chain(std::iter::once(&config.default))
        .find_map(|target| match target {
            RuleTarget::Proxy(name) if !known(name) => Some(name),
            _ => None,
        })
        .map_or(Ok(()), |name| {
            Err(ConfigError::Validation(format!("unknown proxy referenced: {name}")).into())
        })
}

fn parse_default_rule(config: &hocon::Config) -> Result<RuleTarget, RouterError> {
    let default_str = config
        .get_string_option("default")
        .unwrap_or_else(|| "direct".to_string());
    parse_target(&default_str)
        .ok_or_else(|| ConfigError::Validation(format!("invalid default rule '{default_str}'")).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROXY: &str = r#"proxies { p { type = "http", host = "h", port = 8080 } }"#;

    fn err(content: &str) -> String {
        parse_hocon(content).unwrap_err().to_string()
    }

    #[test]
    fn parses_full_config_with_defaults() {
        let cfg = parse_hocon(&format!(
            r#"{PROXY}
            rules = [
              {{ domain = "*.a.com", host = "b.org", cidr = "10.0.0.0/8", route = "proxy:p" }},
              {{ route = "direct" }}
            ]
            default = "p""#
        ))
        .unwrap();

        assert_eq!(cfg.proxy_port, 3128);
        assert_eq!(cfg.api_port, 8080);
        assert_eq!(cfg.proxies[0].health.interval.as_secs(), 30);
        assert_eq!(cfg.rules[0].conditions.len(), 3);
        assert!(cfg.rules[1].conditions.is_empty());
        assert_eq!(cfg.default, RuleTarget::Proxy("p".into()));
    }

    #[test]
    fn health_durations_apply_and_must_be_positive() {
        let cfg = parse_hocon(
            r#"proxies { p { type = "socks5", host = "h", port = 1, health { interval = 7s, timeout = 500ms } } }"#,
        )
        .unwrap();
        let h = &cfg.proxies[0].health;
        assert_eq!((h.interval.as_secs(), h.timeout.as_millis()), (7, 500));

        let zero = r#"proxies { p { type = "http", host = "h", port = 1, health { timeout = 0s } } }"#;
        assert!(err(zero).contains("health.timeout"));
        let junk = r#"proxies { p { type = "http", host = "h", port = 1, health { interval = soon } } }"#;
        assert!(err(junk).contains("health.interval"));
    }

    #[test]
    fn rejects_invalid_proxies() {
        assert!(err(r#"proxies = [{ name = "p", type = "http", host = "h", port = 1 }]"#).contains("keyed by proxy name"));
        assert!(err(r#"proxies { p { type = "ftp", host = "h", port = 1 } }"#).contains("ftp"));
        assert!(err(r#"proxies { p { type = "http", host = "h", port = 70000 } }"#).contains("out of range"));
        assert!(err(r#"proxies { p { type = "http", host = "h", port = 1, health = 5 } }"#).contains("health"));
    }

    #[test]
    fn rejects_bad_rules_and_references() {
        assert!(err(r#"rules = [{ cidr = "10.0.0.0/99", route = "direct" }]"#).contains("cidr"));
        assert!(err(r#"rules = [{ domain = "*.a.com" }]"#).contains("route"));
        assert!(err(r#"rules = [{ domain = "*.a.com", route = "ghost" }]"#).contains("ghost"));
        assert!(err(r#"default = "proxy:""#).contains("invalid default"));
    }

    #[test]
    fn route_targets_parse_all_forms() {
        assert_eq!(parse_target("DIRECT"), Some(RuleTarget::Direct));
        assert_eq!(parse_target("proxy:a"), Some(RuleTarget::Proxy("a".into())));
        assert_eq!(parse_target("group:g"), None);
        assert_eq!(parse_target("bare"), Some(RuleTarget::Proxy("bare".into())));
        assert_eq!(parse_target("weird:x"), None);
    }
}
