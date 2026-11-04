//! HOCON config loading.
//!
//! Parses a HOCON file into the internal Config model using the
//! `hocon-parser` crate's typed getters.

use tracing::{info, warn};

use crate::config::models::{Api, Condition, Config, HealthConfig, Listen, Proxy, ProxyType, Rule, RuleTarget};
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
        listen: parse_listen(&hocon_config)?,
        api: parse_api(&hocon_config)?,
        proxies: parse_proxies(&hocon_config)?,
        rules: parse_rules(&hocon_config)?,
        default: parse_default_rule(&hocon_config)?,
    };
    validate_references(&config)?;
    Ok(config)
}

fn parse_listen(config: &hocon::Config) -> Result<Listen, RouterError> {
    let defaults = Listen::default();
    Ok(Listen {
        host: config.get_string_option("listen.host").unwrap_or(defaults.host),
        port: port_or("listen.port", config.get_i64_option("listen.port"), defaults.port)?,
    })
}

fn parse_api(config: &hocon::Config) -> Result<Api, RouterError> {
    let defaults = Api::default();
    Ok(Api {
        host: config.get_string_option("api.host").unwrap_or(defaults.host),
        port: port_or("api.port", config.get_i64_option("api.port"), defaults.port)?,
    })
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

fn parse_proxies(config: &hocon::Config) -> Result<Vec<Proxy>, RouterError> {
    let hocon_list = config
        .get_list_option("proxies")
        .unwrap_or_default();
    hocon_list
        .into_iter()
        .map(parse_proxy_value)
        .collect()
}

fn parse_proxy_value(value: hocon::HoconValue) -> Result<Proxy, RouterError> {
    let mut name: Option<String> = None;
    let mut proxy_type: Option<ProxyType> = None;
    let mut host: Option<String> = None;
    let mut port: Option<u16> = None;
    let mut interval: Option<std::time::Duration> = None;
    let mut timeout: Option<std::time::Duration> = None;

    if let hocon::HoconValue::Object(map) = value {
        for (key, v) in map {
            match key.as_str() {
                "name" => name = v.as_str().map(|s| s.to_string()),
                "type" => proxy_type = parse_proxy_type(v),
                "host" => host = v.as_str().map(|s| s.to_string()),
                "port" => {
                    if let Some(p) = v.as_i64() {
                        port = Some(valid_port("proxies[].port", p)?);
                    }
                }
                "health" => {
                    if let hocon::HoconValue::Object(hmap) = v {
                        for (hk, hv) in hmap {
                            match hk.as_str() {
                                "interval" => {
                                    if let Some(s) = hv.as_i64() {
                                        if s > 0 {
                                            interval = Some(std::time::Duration::from_secs(s as u64));
                                        }
                                    }
                                }
                                "timeout" => {
                                    if let Some(s) = hv.as_i64() {
                                        if s > 0 {
                                            timeout = Some(std::time::Duration::from_secs(s as u64));
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    } else {
                        return Err(ConfigError::InvalidHealthConfig.into());
                    }
                }
                _ => {}
            }
        }
    }

    let name = name.ok_or(ConfigError::MissingProxyName)?;
    let proxy_type = proxy_type.ok_or(ConfigError::MissingProxyType)?;
    let host = host.ok_or(ConfigError::MissingProxyHost)?;
    let port = port.ok_or(ConfigError::MissingProxyPort)?;

    Ok(Proxy {
        name,
        r#type: proxy_type,
        host,
        port,
        health: HealthConfig {
            interval: interval.unwrap_or_else(|| std::time::Duration::from_secs(30)),
            timeout: timeout.unwrap_or_else(|| std::time::Duration::from_secs(5)),
        },
    })
}

fn parse_proxy_type(value: hocon::HoconValue) -> Option<ProxyType> {
    match value.as_str() {
        Some("http") => Some(ProxyType::Http),
        Some("socks5") => Some(ProxyType::Socks5),
        Some(t) => {
            tracing::warn!(target = "config", "Unknown proxy type: {}", t);
            None
        }
        None => None,
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
    let mut conditions = Vec::new();
    let mut route: Option<RuleTarget> = None;

    if let hocon::HoconValue::Object(map) = value {
        for (key, v) in map {
            match (key.as_str(), v.as_str()) {
                ("domain", Some(d)) => conditions.push(Condition::Domain(d.to_string())),
                ("host", Some(h)) => conditions.push(Condition::Host(h.to_string())),
                ("cidr", Some(c)) => {
                    let net = c
                        .parse()
                        .map_err(|e| ConfigError::Validation(format!("invalid cidr '{c}': {e}")))?;
                    conditions.push(Condition::Cidr(net));
                }
                ("route", _) => route = parse_route_value(v),
                _ => {}
            }
        }
    }

    let rule = Rule {
        conditions,
        route: route.ok_or(ConfigError::MissingRoute)?,
    };
    validate_rule(&rule).map_err(ConfigError::Validation)?;
    Ok(rule)
}

fn parse_route_value(value: hocon::HoconValue) -> Option<RuleTarget> {
    value.as_str().and_then(parse_target)
}

fn parse_target(s: &str) -> Option<RuleTarget> {
    if s.eq_ignore_ascii_case("direct") {
        return Some(RuleTarget::Direct);
    }
    // "proxy:name", "group:name", or a bare proxy name
    match s.split_once(':') {
        Some(("proxy" | "group", name)) if !name.is_empty() => Some(RuleTarget::Proxy(name.to_string())),
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
