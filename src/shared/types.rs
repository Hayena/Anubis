//! Shared public types for the Adaptive Multi-Proxy Router.

use crate::shared::error::RouterError;

#[derive(Debug, Clone)]
pub struct Destination {
    pub host: String,
    pub port: u16,
}

impl Destination {
    pub fn new(host: &str, port: u16) -> Result<Self, RouterError> {
        if host.is_empty() {
            return Err(RouterError::Config(
                crate::shared::error::ConfigError::Validation("host is empty".into()),
            ));
        }
        if port == 0 {
            return Err(RouterError::Config(
                crate::shared::error::ConfigError::Validation(
                    "port must be 1-65535".into(),
                ),
            ));
        }
        Ok(Self {
            host: host.to_string(),
            port,
        })
    }
}
