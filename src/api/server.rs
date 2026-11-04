//! REST API server (Milestone 7).

use std::sync::Arc;

use std::time::{SystemTime, UNIX_EPOCH};
use axum::{
    routing::get,
    routing::post,
    Json,
    Router,
    extract::State,
};

use crate::config::holder::ConfigHolder;
use crate::health::store::HealthStore;
use crate::metrics::collector::{Metrics, MetricsSnapshot};
use axum::http::StatusCode;

/// Shared state for the REST API.
pub struct AppState {
    pub config: Arc<ConfigHolder>,
    /// Startup configuration file path; the only file `/reload` will read.
    pub config_path: String,
    pub metrics: Arc<Metrics>,
    pub health: Arc<dyn HealthStore>,
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
            health,
        }
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
    match state.config.reload(&state.config_path) {
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

fn to_iso8601(epoch_seconds: u64) -> String {
    let days = epoch_seconds / 86400;
    let sec_of_day = (epoch_seconds % 86400) as u32;
    let z = days as i64 + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y, m, d, sec_of_day / 3600, (sec_of_day % 3600) / 60, sec_of_day % 60
    )
}

fn to_iso(opt: Option<SystemTime>) -> Option<String> {
    opt.map(|t| {
        let s = t
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        to_iso8601(s)
    })
}

fn now_iso8601() -> String {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    to_iso8601(since)
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
