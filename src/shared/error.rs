//! Shared error types for the Adaptive Multi-Proxy Router.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum RouterError {
    #[error("Config error: {0}")]
    Config(#[from] ConfigError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Upstream error: {0}")]
    Upstream(#[from] UpstreamError),

    #[error("Invalid destination: {0}")]
    Destination(#[from] DestinationError),
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Invalid HOCON: {0}")]
    HoconParse(String),

    #[error("Validation failed: {0}")]
    Validation(String),

    #[error("File not found: {0}")]
    FileNotFound(String),

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
pub enum UpstreamError {
    #[error("Unknown upstream: {0}")]
    UnknownUpstream(String),

    #[error("Upstream {0} failed its last health check")]
    Unhealthy(String),

    #[error("Upstream connect failed: {0}")]
    Connect(#[from] ConnectError),
}

/// Failure to establish a transport connection to a destination.
#[derive(Error, Debug)]
pub enum ConnectError {
    #[error("{context}: {source}")]
    Io {
        context: &'static str,
        #[source]
        source: std::io::Error,
    },

    #[error("handshake failed: {0}")]
    Handshake(String),

    #[error("proxy rejected CONNECT: {0}")]
    Rejected(String),

    #[error("timed out after {0:?}")]
    Timeout(std::time::Duration),
}

impl ConnectError {
    pub fn io(context: &'static str) -> impl FnOnce(std::io::Error) -> Self {
        move |source| Self::Io { context, source }
    }
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum DestinationError {
    #[error("host is empty")]
    EmptyHost,

    #[error("port must be 1-65535")]
    InvalidPort,
}
