//! Health state storage.

use std::collections::BTreeMap;
use std::sync::{PoisonError, RwLock};
use std::time::SystemTime;

use crate::health::checker::HealthState;

/// Read/write health-store contract.
///
/// The health checker writes state; managers and API handlers read it.
/// Reads return a consistent [`HealthState`] snapshot per proxy.
pub trait HealthStore: Send + Sync {
    /// Record a successful or failed health assessment for a proxy.
    fn set_healthy(&self, name: &str, healthy: bool, latency_ms: Option<f64>);

    /// Snapshot of a proxy's state (None if never checked).
    fn get(&self, name: &str) -> Option<HealthState>;

    /// Whether a proxy is currently considered healthy.
    fn is_healthy(&self, name: &str) -> bool {
        self.get(name).is_some_and(|s| s.healthy)
    }
}

/// In-memory health store (RwLock-protected BTreeMap).
#[derive(Default)]
pub struct InMemoryHealthStore {
    states: RwLock<BTreeMap<String, HealthState>>,
}

impl InMemoryHealthStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl HealthStore for InMemoryHealthStore {
    fn set_healthy(&self, name: &str, healthy: bool, latency_ms: Option<f64>) {
        let now = SystemTime::now();
        let mut map = self.states.write().unwrap_or_else(PoisonError::into_inner);
        let state = map.entry(name.to_string()).or_insert(HealthState {
            healthy,
            latency_ms: None,
            last_success: None,
            last_failure: None,
        });
        state.healthy = healthy;
        state.latency_ms = latency_ms;
        *(if healthy {
            &mut state.last_success
        } else {
            &mut state.last_failure
        }) = Some(now);
    }

    fn get(&self, name: &str) -> Option<HealthState> {
        self.states
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_proxy_is_unhealthy_and_has_no_state() {
        let store = InMemoryHealthStore::new();
        assert!(!store.is_healthy("x"));
        assert!(store.get("x").is_none());
    }

    #[test]
    fn tracks_latency_and_last_success_and_failure() {
        let store = InMemoryHealthStore::new();
        store.set_healthy("p", true, Some(12.5));
        let s = store.get("p").unwrap();
        assert!(s.healthy && s.last_failure.is_none());
        assert_eq!(s.latency_ms, Some(12.5));

        store.set_healthy("p", false, None);
        let s = store.get("p").unwrap();
        assert!(!s.healthy && s.last_success.is_some() && s.last_failure.is_some());
        assert_eq!(s.latency_ms, None);
    }
}
