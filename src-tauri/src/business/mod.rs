//! Business layer
//!
//! Use-case boundary for every entry point. Business modules orchestrate
//! repositories and DNS engine components, and return `AppError` so the
//! adapters stay free of transport concerns.

pub mod cache_business;
pub mod dns_query_business;
pub mod listener_business;
pub mod log_business;
pub mod record_business;
pub mod rewrite_business;
pub mod setting_business;
pub mod status_business;
pub mod strategy_business;
pub mod upstream_business;
