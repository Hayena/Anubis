//! Upstream manager: manages connection pool and route selection.

use std::collections::HashMap;
use std::sync::Arc;

use crate::shared::error::UpstreamError;
use crate::upstream::traits::UpstreamConnector;

/// Upstream manager: resolves proxy names to concrete connectors.
pub struct UpstreamManager {
    /// Registry of named upstream connectors (Direct / HTTP / SOCKS5).
    connectors: HashMap<String, Arc<dyn UpstreamConnector>>,
}

impl UpstreamManager {
    pub fn new() -> Self {
        Self {
            connectors: HashMap::new(),
        }
    }

    /// Register a named upstream connector.
    pub fn register(&mut self, name: String, connector: Arc<dyn UpstreamConnector>) {
        self.connectors.insert(name, connector);
    }

    /// Resolve a proxy name to a connector.
    pub fn resolve(&self, name: &str) -> Result<Arc<dyn UpstreamConnector>, UpstreamError> {
        self.connectors
            .get(name)
            .cloned()
            .ok_or_else(|| UpstreamError::UnknownUpstream(name.to_string()))
    }
}

impl Default for UpstreamManager {
    fn default() -> Self {
        Self::new()
    }
}
