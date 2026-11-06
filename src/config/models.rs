//! Configuration models.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Forward proxy port (always binds to 127.0.0.1).
    #[serde(default = "default_proxy_port")]
    pub proxy_port: u16,

    /// REST API port (always binds to 127.0.0.1).
    #[serde(default = "default_api_port")]
    pub api_port: u16,

    #[serde(default)]
    pub proxies: Vec<Proxy>,

    #[serde(default)]
    pub rules: Vec<Rule>,

    #[serde(default)]
    pub default: RuleTarget,
}

pub const DEFAULT_PROXY_PORT: u16 = 3128;
pub const DEFAULT_API_PORT: u16 = 8080;

fn default_proxy_port() -> u16 {
    DEFAULT_PROXY_PORT
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

fn default_api_port() -> u16 {
    DEFAULT_API_PORT
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
