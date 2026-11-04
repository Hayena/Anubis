//! Shared public types for the Adaptive Multi-Proxy Router.

use crate::shared::error::DestinationError;

/// A validated connection target. Fields are private so every instance has
/// passed [`Destination::new`] (non-empty host, non-zero port).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    host: String,
    port: u16,
}

impl Destination {
    pub fn new(host: &str, port: u16) -> Result<Self, DestinationError> {
        if host.is_empty() {
            return Err(DestinationError::EmptyHost);
        }
        if port == 0 {
            return Err(DestinationError::InvalidPort);
        }
        Ok(Self {
            host: host.to_string(),
            port,
        })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}
