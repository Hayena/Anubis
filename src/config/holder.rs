//! Hot-reloadable configuration and routing state.

use std::sync::{Arc, PoisonError, RwLock};

use crate::config::loader;
use crate::config::models::Config;
use crate::routing::engine::RouteEngine;
use crate::upstream::manager::UpstreamManager;
use crate::shared::error::RouterError;

/// An immutable config + route engine + upstream connectors. Always swapped
/// as one unit, so readers never observe a mismatched combination.
pub struct Snapshot {
    pub config: Arc<Config>,
    pub engine: Arc<RouteEngine>,
    pub manager: Arc<UpstreamManager>,
}

impl Snapshot {
    fn build(config: Config) -> Arc<Self> {
        let engine = Arc::new(RouteEngine::new(config.rules.clone(), config.default.clone()));
        let manager = Arc::new(UpstreamManager::from_proxies(&config.proxies));
        Arc::new(Self {
            config: Arc::new(config),
            engine,
            manager,
        })
    }
}

/// Swappable holder of the current [`Snapshot`] (hot reload).
pub struct ConfigHolder {
    current: RwLock<Arc<Snapshot>>,
}

impl ConfigHolder {
    pub fn new(config: Config) -> Self {
        Self {
            current: RwLock::new(Snapshot::build(config)),
        }
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        Arc::clone(&self.current.read().unwrap_or_else(PoisonError::into_inner))
    }

    pub fn current_config(&self) -> Arc<Config> {
        Arc::clone(&self.snapshot().config)
    }

    pub fn current_engine(&self) -> Arc<RouteEngine> {
        Arc::clone(&self.snapshot().engine)
    }

    /// Parse and validate `path`, then atomically swap in the new state.
    /// On error the previous state is kept.
    pub fn reload(&self, path: &str) -> Result<Arc<Config>, RouterError> {
        let next = Snapshot::build(loader::reload(path)?);
        let config = Arc::clone(&next.config);
        *self.current.write().unwrap_or_else(PoisonError::into_inner) = next;
        Ok(config)
    }
}
