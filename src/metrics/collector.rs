//! Metrics collector: in-memory counters, snapshots.

use std::collections::BTreeMap;
use std::sync::atomic::AtomicU64;
use std::sync::{Mutex, PoisonError};

/// In-memory metrics collector (ADR §3.9).
///
/// Counters are atomic so observers (API handlers) may read them without
/// locking the collector.
pub struct Metrics {
    connections_opened: AtomicU64,
    connections_failed: AtomicU64,
    active_connections: AtomicU64,
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
    /// Per-proxy selection counts.
    proxy_selections: Mutex<BTreeMap<String, u64>>,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            connections_opened: AtomicU64::new(0),
            connections_failed: AtomicU64::new(0),
            active_connections: AtomicU64::new(0),
            bytes_in: AtomicU64::new(0),
            bytes_out: AtomicU64::new(0),
            proxy_selections: Mutex::new(BTreeMap::new()),
        }
    }

    /// Increment the open connections counter.
    pub fn increment_opened(&self) {
        self.connections_opened.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Count one routing selection of the named proxy.
    pub fn record_selection(&self, proxy: &str) {
        *self
            .proxy_selections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(proxy.to_string())
            .or_default() += 1;
    }

    pub fn increment_failed(&self) {
        self.connections_failed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Record bytes relayed to (`out`) and from (`in_`) the upstream.
    pub fn add_bytes(&self, in_: u64, out: u64) {
        self.bytes_in.fetch_add(in_, std::sync::atomic::Ordering::Relaxed);
        self.bytes_out.fetch_add(out, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn increment_active(&self) {
        self.active_connections.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn decrement_active(&self) {
        self.active_connections.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Snapshot of metrics at a point in time (ADR §3.9).
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            connections_opened: self.connections_opened.load(std::sync::atomic::Ordering::Relaxed),
            connections_failed: self.connections_failed.load(std::sync::atomic::Ordering::Relaxed),
            active_connections: self.active_connections.load(std::sync::atomic::Ordering::Relaxed),
            bytes_in: self.bytes_in.load(std::sync::atomic::Ordering::Relaxed),
            bytes_out: self.bytes_out.load(std::sync::atomic::Ordering::Relaxed),
            proxy_selections: self
                .proxy_selections
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        }
    }
}

/// Snapshot of metrics at a point in time (ADR §3.9).
#[derive(Debug, Clone, serde::Serialize)]
pub struct MetricsSnapshot {
    pub connections_opened: u64,
    pub connections_failed: u64,
    pub active_connections: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub proxy_selections: BTreeMap<String, u64>,
}
