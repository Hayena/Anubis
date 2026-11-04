//! Configuration models.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub listen: Listen,

    /// REST API server configuration.
    #[serde(default)]
    pub api: Api,

    #[serde(default)]
    pub proxies: Vec<Proxy>,

    #[serde(default)]
    pub rules: Vec<Rule>,

    #[serde(default)]
    pub default: RuleTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Listen {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_listen_port")]
    pub port: u16,
}

impl Default for Listen {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_listen_port(),
        }
    }
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_listen_port() -> u16 {
    3128
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proxy {
    pub name: String,
    pub r#type: ProxyType,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub health: HealthConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProxyType {
    Http,
    Socks5,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthConfig {
    #[serde(default = "default_30s")]
    pub interval: std::time::Duration,

    #[serde(default = "default_5s")]
    pub timeout: std::time::Duration,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            interval: default_30s(),
            timeout: default_5s(),
        }
    }
}

fn default_30s() -> std::time::Duration {
    std::time::Duration::from_secs(30)
}

fn default_5s() -> std::time::Duration {
    std::time::Duration::from_secs(5)
}

/// REST API server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Api {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_api_port")]
    pub port: u16,
}

impl Default for Api {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_api_port(),
        }
    }
}

fn default_api_port() -> u16 {
    8080
}

/// A single match condition of a rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Condition {
    /// Hostname pattern; `*` matches one or more non-dot characters.
    Domain(String),
    /// Exact hostname (case-insensitive).
    Host(String),
    /// IP range.
    Cidr(ipnet::IpNet),
}

/// A routing rule. Conditions are OR-ed; a rule without conditions is a
/// catch-all that matches every destination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub conditions: Vec<Condition>,
    pub route: RuleTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RuleTarget {
    #[default]
    Direct,
    Proxy(String),
}
