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
    pub default: DefaultRule,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Listen {
    pub host: String,
    pub port: u16,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HealthConfig {
    #[serde(default = "default_30s")]
    pub interval: std::time::Duration,

    #[serde(default = "default_5s")]
    pub timeout: std::time::Duration,
}

fn default_30s() -> std::time::Duration {
    std::time::Duration::from_secs(30)
}

fn default_5s() -> std::time::Duration {
    std::time::Duration::from_secs(5)
}

/// REST API server configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Api {
    pub host: String,
    #[serde(default = "default_api_port")]
    pub port: u16,
}

fn default_api_port() -> u16 {
    8080
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    #[serde(rename = "domain")]
    pub wildcard_domain: Option<String>,
    #[serde(rename = "host")]
    pub exact_host: Option<String>,
    #[serde(rename = "cidr")]
    pub cidr: Option<String>,
    pub route: RuleTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RuleTarget {
    Direct,
    Proxy(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum DefaultRule {
    #[default]
    Direct,
    Proxy(String),
}
