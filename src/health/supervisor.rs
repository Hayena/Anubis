//! Owns the background health-checker tasks and re-spawns them on reload.

use std::sync::{Arc, Mutex, PoisonError};

use tokio::task::JoinHandle;

use crate::config::models::Config;
use crate::health::checker::HealthChecker;
use crate::health::store::HealthStore;

pub struct HealthSupervisor {
    store: Arc<dyn HealthStore>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl HealthSupervisor {
    pub fn new(store: Arc<dyn HealthStore>) -> Self {
        Self {
            store,
            tasks: Mutex::new(Vec::new()),
        }
    }

    /// Stop all running checkers and start one per proxy in `config`.
    /// Must be called from within a Tokio runtime.
    pub fn apply(&self, config: &Config) {
        let spawned = config
            .proxies
            .iter()
            .map(|proxy| {
                let checker = HealthChecker::new(
                    proxy.name.clone(),
                    proxy.host.clone(),
                    proxy.port,
                    proxy.health.interval,
                    proxy.health.timeout,
                    Arc::clone(&self.store),
                );
                tracing::info!(target = "health", proxy = %proxy.name, "health checker started");
                tokio::spawn(async move { checker.run().await })
            })
            .collect();

        let previous = std::mem::replace(
            &mut *self.tasks.lock().unwrap_or_else(PoisonError::into_inner),
            spawned,
        );
        previous.iter().for_each(JoinHandle::abort);
    }
}

impl Drop for HealthSupervisor {
    fn drop(&mut self) {
        let tasks = self.tasks.get_mut().unwrap_or_else(PoisonError::into_inner);
        tasks.iter().for_each(JoinHandle::abort);
    }
}
