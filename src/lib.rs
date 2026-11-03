// Adaptive Multi-Proxy Router
//
// A local forward proxy with adaptive routing.
// Library root: re-exports shared types, public crates.

pub mod config;
pub mod health;
pub mod metrics;
pub mod proxy;
pub mod routing;
pub mod shared;
pub mod upstream;

// Re-exports for convenience
pub use config::models::{DefaultRule, Listen, Proxy, Rule, RuleTarget};
pub use health::checker::{HealthChecker, HealthState};
pub use health::store::{HealthStore, InMemoryHealthStore};
pub use routing::decision::RoutingDecision;
pub use shared::error::{ConfigError, RouterError, UpgradeError, RoutingError};
pub use shared::types::Destination;
