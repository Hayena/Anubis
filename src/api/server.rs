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

use crate::config::loader;
use crate::config::models::Config;
use crate::health::store::HealthStore;
use crate::health::store::InMemoryHealthStore;
use crate::metrics::collector::{Metrics, MetricsSnapshot};
use crate::routing::engine::RouteEngine;
use crate::shared::error::RouterError;
use axum::http::StatusCode;

/// Shared state for the REST API.
pub struct AppState {
    pub config: ConfigHolder,
    /// Shared configuration file path (used as the `/reload` default).
    pub config_path: String,
    pub metrics: Arc<Metrics>,
    pub health: Arc<InMemoryHealthStore>,
}

impl AppState {
    pub fn new(
        config: ConfigHolder,
        metrics: Arc<Metrics>,
        health: Arc<InMemoryHealthStore>,
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

/// Swappable config + routing state (Milestone 8 hot reload).
pub struct ConfigHolder {
    config: std::sync::Mutex<Arc<Config>>,
    engine: std::sync::Mutex<Arc<RouteEngine>>,
}

impl ConfigHolder {
    pub fn new(config: Config) -> Self {
        let engine = RouteEngine::new(config.rules.clone(), config.default.clone());
        Self {
            config: std::sync::Mutex::new(Arc::new(config)),
            engine: std::sync::Mutex::new(Arc::new(engine)),
        }
    }

    /// Current config (clone of the `Arc<Config>` behind the mutex).
    pub fn current_config(&self) -> Arc<Config> {
        self.config.lock().unwrap().clone()
    }

    /// Current route engine.
    pub fn current_engine(&self) -> Arc<RouteEngine> {
        self.engine.lock().unwrap().clone()
    }

    /// Hot reload: parse + validate a new config, swap it, and rebuild the
    /// route engine. The caller owns `Arc<Config>` replacement; active
    /// connections are not dropped.
    pub fn reload(&self, path: &str) -> Result<Arc<Config>, RouterError> {
        let new_config = loader::reload(path)?;
        let new_arc = Arc::new(new_config);
        *self.config.lock().unwrap() = new_arc.clone();
        *self.engine.lock().unwrap() = Arc::new(RouteEngine::new(new_arc.rules.clone(), new_arc.default.clone()));
        Ok(new_arc)
    }
}

// --- /health ---

#[derive(serde::Serialize)]
struct HealthResponse {
    status: String,
    time: String,
}

async fn health_handler(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let healthy = state.health.is_healthy("all");
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
    latency_ms: f64,
    last_success: Option<String>,
    last_failure: Option<String>,
}

#[derive(serde::Serialize)]
struct ProxiesResponse {
    proxies: Vec<ProxyHealth>,
}

async fn proxies_handler(State(state): State<Arc<AppState>>) -> Json<ProxiesResponse> {
    let config = state.config.current_config();
    let health = state.health.clone();
    let mut items = Vec::with_capacity(config.proxies.len());
    for p in &config.proxies {
        let healthy = health.is_healthy(&p.name);
        let latency_ms = health.latency_ms(&p.name).unwrap_or(0.0);
        let last_success = to_iso(health.last_success(&p.name));
        let last_failure = to_iso(health.last_failure(&p.name));
        items.push(ProxyHealth {
            name: p.name.clone(),
            healthy,
            latency_ms,
            last_success,
            last_failure,
        });
    }
    Json(ProxiesResponse { proxies: items })
}

// --- /stats ---

async fn stats_handler(State(state): State<Arc<AppState>>) -> Json<MetricsSnapshot> {
    Json(state.metrics.clone().snapshot())
}

// --- /reload ---

#[derive(serde::Deserialize)]
struct ReloadBody {
    #[serde(default)]
    path: Option<String>,
}

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
    Json(body): Json<ReloadBody>,
) -> Result<Json<ReloadResponse>, (StatusCode, Json<ErrorResponse>)> {
    // The request body may carry a path; otherwise fall back to the
    // startup config file.
    let path = body.path.unwrap_or(state.config_path.clone());
    match state.config.reload(&path) {
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
    let y = if m <= 2 { y + 1 } else { y } as i64;
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
