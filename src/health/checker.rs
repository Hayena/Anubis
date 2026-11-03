//! Health checker background task.

use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::time;
use tracing;

/// Per-proxy health state.
#[derive(Debug, Clone)]
pub struct HealthState {
    pub healthy: bool,
    pub latency_ms: f64,
    pub last_success: Option<std::time::SystemTime>,
    pub last_failure: Option<std::time::SystemTime>,
}

impl HealthState {
}

/// Background health checker for a single upstream proxy.
pub struct HealthChecker {
    /// Name of the proxy being checked.
    pub name: String,
    /// Address the checker TCP-connects to (the upstream proxy itself).
    pub check_host: String,
    pub check_port: u16,
    pub interval: Duration,
    pub timeout: Duration,
    pub store: std::sync::Arc<dyn crate::health::store::HealthStore>,
}

impl HealthChecker {
    pub fn new(
        name: String,
        check_host: String,
        check_port: u16,
        interval: Duration,
        timeout: Duration,
        store: std::sync::Arc<dyn crate::health::store::HealthStore>,
    ) -> Self {
        Self {
            name,
            check_host,
            check_port,
            interval,
            timeout,
            store,
        }
    }

    /// Run the checker loop (runs until the process exits).
    pub async fn run(&self) {
        let mut interval = time::interval(self.interval);
        loop {
            interval.tick().await;
            self.tick().await;
        }
    }

    async fn tick(&self) {
        // TCP-connect with a timeout; on success measure RTT, on failure mark
        // unhealthy. Never panics on TCP/timeout errors.
        let start = Instant::now();
        let outcome = tokio::time::timeout(self.timeout, async {
            TcpStream::connect((self.check_host.as_str(), self.check_port)).await
        })
        .await;

        match outcome {
            Ok(Ok(_stream)) => {
                let elapsed = start.elapsed();
                let latency = elapsed.as_secs_f64() * 1000.0;
                self.store.set_healthy(&self.name, true, Some(latency));
                tracing::info!(
                    target = "health",
                    proxy = %self.name,
                    latency_ms = %latency,
                    "health check passed"
                );
            }
            _ => {
                self.store.set_healthy(&self.name, false, None);
                tracing::info!(
                    target = "health",
                    proxy = %self.name,
                    "health check failed (unreachable or timeout)"
                );
            }
        }
    }
}
