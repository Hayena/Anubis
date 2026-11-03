//! Metrics collector: in-memory counters, snapshots.

use std::collections::BTreeMap;
use std::sync::atomic::AtomicU64;

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
    proxy_selections: BTreeMap<String, AtomicU64>,
    /// EWMA per-proxy latency (updated by the health checker).
    avg_latency_ms: BTreeMap<String, f64>,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            connections_opened: AtomicU64::new(0),
            connections_failed: AtomicU64::new(0),
            active_connections: AtomicU64::new(0),
            bytes_in: AtomicU64::new(0),
            bytes_out: AtomicU64::new(0),
            proxy_selections: BTreeMap::new(),
            avg_latency_ms: BTreeMap::new(),
        }
    }

    /// Increment the open connections counter.
    pub fn increment_opened(&self) {
        self.connections_opened.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
                .iter()
                .map(|(k, v)| (k.clone(), v.load(std::sync::atomic::Ordering::Relaxed)))
                .collect(),
            avg_latency_ms: self.avg_latency_ms.clone(),
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
    pub avg_latency_ms: BTreeMap<String, f64>,
}
