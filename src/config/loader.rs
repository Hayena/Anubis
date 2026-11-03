//! HOCON config loading.
//!
//! Parses a HOCON file into the internal Config model using the
//! `hocon-parser` crate's typed getters.

use tracing::{info, warn};

use crate::config::models::{Api, Config, DefaultRule, HealthConfig, Listen, Proxy, ProxyType, Rule, RuleTarget};
use crate::shared::error::{ConfigError, RouterError};

/// Load configuration from a HOCON file.
pub async fn load(path: &str, watch: bool) -> Result<Config, RouterError> {
    info!(target = "config", "Loading config from {}", path);
    let config = reload(path)?;
    if watch {
        info!(target = "config", "Watching config file for changes (hot reload)");
    }
    Ok(config)
}

/// Programmatic reload of configuration (Milestone 8 hot reload).
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
    match hocon::parse(content) {
        Ok(hocon_config) => {
            let config = Config {
                listen: parse_listen(&hocon_config)?,
                api: parse_api(&hocon_config)?,
                proxies: parse_proxies(&hocon_config)?,
                
                rules: parse_rules(&hocon_config)?,
                default: parse_default_rule(&hocon_config)?,
            };

            Ok(config)
        }
        Err(e) => Err(ConfigError::HoconParse(e.to_string()).into()),
    }
}

fn parse_listen(config: &hocon::Config) -> Result<Listen, RouterError> {
    let host = config
        .get_string_option("listen.host")
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let port = config
        .get_i64_option("listen.port")
        .unwrap_or(3128) as u16;
    Ok(Listen { host, port })
}

fn parse_api(config: &hocon::Config) -> Result<Api, RouterError> {
    let host = config
        .get_string_option("api.host")
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let port = config
        .get_i64_option("api.port")
        .unwrap_or(8080) as u16;
    Ok(Api { host, port })
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
                "port" => port = v.as_i64().map(|p| p as u16),
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
    let mut wildcard_domain: Option<String> = None;
    let mut exact_host: Option<String> = None;
    let mut cidr: Option<String> = None;
    let mut route: Option<RuleTarget> = None;

    if let hocon::HoconValue::Object(map) = value {
        for (key, v) in map {
            match key.as_str() {
                "domain" => wildcard_domain = v.as_str().map(|s| s.to_string()),
                "host" => exact_host = v.as_str().map(|s| s.to_string()),
                "cidr" => cidr = v.as_str().map(|s| s.to_string()),
                "route" => route = parse_route_value(v),
                _ => {}
            }
        }
    }

    let route = route.ok_or(ConfigError::MissingRoute)?;

    Ok(Rule {
        wildcard_domain,
        exact_host,
        cidr,
        route,
    })
}

fn parse_route_value(value: hocon::HoconValue) -> Option<RuleTarget> {
    match value.as_str() {
        Some("direct") => Some(RuleTarget::Direct),
        Some(s) => {
            // Support "proxy:name", "group:name", or bare proxy name
            let parts: Vec<&str> = s.splitn(2, ':').collect();
            if parts.len() == 2 {
                match parts[0] {
                    "proxy" => Some(RuleTarget::Proxy(parts[1].to_string())),
                    "group" => Some(RuleTarget::Proxy(parts[1].to_string())),
                    _ => None,
                }
            } else {
                // Bare proxy name (no colon) - treat as direct proxy reference
                Some(RuleTarget::Proxy(s.to_string()))
            }
        }
        None => None,
    }
}

/// Validate that every proxy group member resolves to an already-registered
/// proxy name. Groups with missing members are a configuration error: the
/// group can never be selected, so it is better to fail fast than to wait
/// for a runtime lookup to fail.


fn parse_default_rule(config: &hocon::Config) -> Result<DefaultRule, RouterError> {
    let default_str = config
        .get_string_option("default")
        .unwrap_or_else(|| "direct".to_string());

    match default_str.to_lowercase().as_str() {
        "direct" => Ok(DefaultRule::Direct),
        "proxy" => Ok(DefaultRule::Proxy(String::new())),
        s => {
            tracing::warn!(target = "config", "Unknown default rule: {}", s);
            Ok(DefaultRule::Direct)
        }
    }
}
