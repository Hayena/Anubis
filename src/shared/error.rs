//! Shared error types for the Adaptive Multi-Proxy Router.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum RouterError {
    #[error("Config error: {0}")]
    Config(#[from] ConfigError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Upgrade error: {0}")]
    Upgrade(#[from] UpgradeError),

    #[error("Routing error: {0}")]
    Routing(#[from] RoutingError),

    #[error("Decode error: {0}")]
    Decode(String),
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Invalid HOCON: {0}")]
    HoconParse(String),

    #[error("Validation failed: {0}")]
    Validation(String),

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("Missing proxy name")]
    MissingProxyName,

    #[error("Missing proxy type")]
    MissingProxyType,

    #[error("Missing proxy host")]
    MissingProxyHost,

    #[error("Missing proxy port")]
    MissingProxyPort,

    #[error("Missing route")]
    MissingRoute,

    #[error("Invalid health config")]
    InvalidHealthConfig,
}

#[derive(Error, Debug)]
pub enum UpgradeError {
    #[error("Unknown upstream: {0}")]
    UnknownUpstream(String),
}

#[derive(Error, Debug)]
pub enum RoutingError {
}
