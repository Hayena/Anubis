//! REST API server.

use std::sync::Arc;

use std::time::SystemTime;
#[cfg(test)]
use std::time::UNIX_EPOCH;
use axum::{
    routing::get,
    routing::post,
    Json,
    Router,
    extract::State,
};

use crate::config::holder::ConfigHolder;
use crate::config::models::Config;
use crate::health::store::HealthStore;
use crate::health::supervisor::HealthSupervisor;
use crate::shared::error::RouterError;
use crate::metrics::collector::{Metrics, MetricsSnapshot};
use axum::http::StatusCode;

/// Shared state for the REST API.
pub struct AppState {
    pub config: Arc<ConfigHolder>,
    /// Startup configuration file path; the only file `/reload` will read.
    pub config_path: String,
    pub metrics: Arc<Metrics>,
    pub health: Arc<dyn HealthStore>,
    supervisor: HealthSupervisor,
}

impl AppState {
    pub fn new(
        config: Arc<ConfigHolder>,
        metrics: Arc<Metrics>,
        health: Arc<dyn HealthStore>,
        config_path: String,
    ) -> Self {
        Self {
            config,
            config_path,
            metrics,
            supervisor: HealthSupervisor::new(Arc::clone(&health)),
            health,
        }
    }

    /// Start (or restart) the health checkers for the current config.
    pub fn start_health_checks(&self) {
        self.supervisor.apply(&self.config.current_config());
    }

    /// Reload the startup config file: swap routing state and connectors,
    /// then restart health checkers. On error the old state keeps running.
    pub fn reload(&self) -> Result<Arc<Config>, RouterError> {
        let config = self.config.reload(&self.config_path)?;
        self.supervisor.apply(&config);
        Ok(config)
    }
}

// --- /health ---

#[derive(serde::Serialize)]
struct HealthResponse {
    status: String,
    time: String,
}

async fn health_handler(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let config = state.config.current_config();
    let healthy = config.proxies.iter().all(|p| state.health.is_healthy(&p.name));
    Json(HealthResponse {
        status: if healthy { "ok" } else { "degraded" }.to_string(),
        time: now_iso8601(),
    })
}

// --- /proxies ---

#[derive(serde::Serialize)]
struct ProxyHealth {
    name: String,
    healthy: bool,
    latency_ms: Option<f64>,
    last_success: Option<String>,
    last_failure: Option<String>,
}

#[derive(serde::Serialize)]
struct ProxiesResponse {
    proxies: Vec<ProxyHealth>,
}

async fn proxies_handler(State(state): State<Arc<AppState>>) -> Json<ProxiesResponse> {
    let config = state.config.current_config();
    let health = &state.health;
    let items = config
        .proxies
        .iter()
        .map(|p| {
            let state = health.get(&p.name);
            ProxyHealth {
                name: p.name.clone(),
                healthy: state.as_ref().is_some_and(|s| s.healthy),
                latency_ms: state.as_ref().and_then(|s| s.latency_ms),
                last_success: to_iso(state.as_ref().and_then(|s| s.last_success)),
                last_failure: to_iso(state.as_ref().and_then(|s| s.last_failure)),
            }
        })
        .collect();
    Json(ProxiesResponse { proxies: items })
}

// --- /stats ---

async fn stats_handler(State(state): State<Arc<AppState>>) -> Json<MetricsSnapshot> {
    Json(state.metrics.snapshot())
}

// --- /reload ---

#[derive(serde::Serialize)]
struct ReloadResponse {
    reloaded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    config_summary: Option<serde_json::Value>,
}

#[derive(serde::Serialize)]
struct ErrorResponse {
    error: String,
}

async fn reload_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ReloadResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.reload() {
        Ok(new_config) => {
            let summary = serde_json::json!({
                "reloaded": true,
                "proxies": new_config.proxies.len(),
                "rules": new_config.rules.len(),
            });
            Ok(Json(ReloadResponse {
                reloaded: true,
                config_summary: Some(summary),
            }))
        }
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("config reload failed: {e}"),
            }),
        )),
    }
}

// --- timer helpers ---

fn to_iso(opt: Option<SystemTime>) -> Option<String> {
    opt.map(format_rfc3339)
}

fn now_iso8601() -> String {
    format_rfc3339(SystemTime::now())
}

/// UTC timestamp truncated to whole seconds, e.g. `2026-11-04T20:07:22Z`.
fn format_rfc3339(t: SystemTime) -> String {
    use time::format_description::well_known::Rfc3339;

    time::OffsetDateTime::from(t)
        .replace_nanosecond(0)
        .ok()
        .and_then(|dt| dt.format(&Rfc3339).ok())
        .unwrap_or_default()
}

// --- router ---

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/proxies", get(proxies_handler))
        .route("/stats", get(stats_handler))
        .route("/reload", post(reload_handler))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn formats_epoch_seconds_as_utc() {
        assert_eq!(format_rfc3339(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        let t = UNIX_EPOCH + Duration::from_millis(1_793_823_142_987);
        assert_eq!(format_rfc3339(t), "2026-11-04T20:12:22Z");
    }
}
