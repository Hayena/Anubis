//! Health state storage.

use std::collections::BTreeMap;
use std::sync::RwLock;

use crate::health::checker::HealthState;

/// Read/write health-store contract.
///
/// The health checker writes state; managers and API handlers read it.
pub trait HealthStore: Send + Sync {
    /// Record a successful or failed health assessment for a proxy.
    fn set_healthy(&self, name: &str, healthy: bool, latency_ms: Option<f64>);

    /// Whether a proxy is currently considered healthy.
    fn is_healthy(&self, name: &str) -> bool;

    /// Last measured RTT in milliseconds (None if unmeasured).
    fn latency_ms(&self, name: &str) -> Option<f64>;

    /// Time of the last successful health check.
    fn last_success(&self, name: &str) -> Option<std::time::SystemTime>;

    /// Time of the last failed health check.
    fn last_failure(&self, name: &str) -> Option<std::time::SystemTime>;
}

/// In-memory health store ( RwLock-protected BTreeMap ).
pub struct InMemoryHealthStore {
    states: RwLock<BTreeMap<String, HealthState>>,
}

impl InMemoryHealthStore {
    pub fn new() -> Self {
        Self {
            states: RwLock::new(BTreeMap::new()),
        }
    }
}

impl HealthStore for InMemoryHealthStore {
    fn set_healthy(&self, name: &str, healthy: bool, latency_ms: Option<f64>) {
        let mut map = self.states.write().unwrap();
        let now = std::time::SystemTime::now();
        if let Some(state) = map.get_mut(name) {
            state.healthy = healthy;
            state.latency_ms = latency_ms.unwrap_or(f64::NAN);
            if healthy {
                state.last_success = Some(now);
            } else {
                state.last_failure = Some(now);
            }
        } else {
            map.insert(
                name.to_string(),
                HealthState {
                    healthy,
                    latency_ms: latency_ms.unwrap_or(f64::NAN),
                    last_success: if healthy { Some(now) } else { None },
                    last_failure: if !healthy { Some(now) } else { None },
                },
            );
        }
    }

    fn is_healthy(&self, name: &str) -> bool {
        self.states
            .read()
            .unwrap()
            .get(name)
            .map(|s| s.healthy)
            .unwrap_or(false)
    }

    fn latency_ms(&self, name: &str) -> Option<f64> {
        self.states
            .read()
            .unwrap()
            .get(name)
            .map(|s| s.latency_ms)
    }

    fn last_success(&self, name: &str) -> Option<std::time::SystemTime> {
        self.states
            .read()
            .unwrap()
            .get(name)
            .and_then(|s| s.last_success)
    }

    fn last_failure(&self, name: &str) -> Option<std::time::SystemTime> {
        self.states
            .read()
            .unwrap()
            .get(name)
            .and_then(|s| s.last_failure)
    }
}
