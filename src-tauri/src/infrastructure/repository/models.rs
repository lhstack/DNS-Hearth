//! Database models
//!
//! Data structures representing database entities.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// DNS record entity
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DnsRecord {
    pub id: i64,
    pub name: String,
    pub record_type: String,
    pub value: String,
    pub ttl: i32,
    pub priority: i32,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Create DNS record request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDnsRecord {
    pub name: String,
    pub record_type: String,
    pub value: String,
    #[serde(default = "default_ttl")]
    pub ttl: i32,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

/// Update DNS record request
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateDnsRecord {
    pub name: Option<String>,
    pub record_type: Option<String>,
    pub value: Option<String>,
    pub ttl: Option<i32>,
    pub priority: Option<i32>,
    pub enabled: Option<bool>,
}

/// Rewrite rule entity
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct RewriteRule {
    pub id: i64,
    pub pattern: String,
    pub match_type: String,
    pub action_type: String,
    pub action_value: Option<String>,
    pub priority: i32,
    pub enabled: bool,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Create rewrite rule request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRewriteRule {
    pub pattern: String,
    pub match_type: String,
    pub action_type: String,
    pub action_value: Option<String>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub description: Option<String>,
}

/// Update rewrite rule request
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateRewriteRule {
    pub pattern: Option<String>,
    pub match_type: Option<String>,
    pub action_type: Option<String>,
    pub action_value: Option<String>,
    pub priority: Option<i32>,
    pub enabled: Option<bool>,
    pub description: Option<String>,
}

/// Upstream server entity
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct UpstreamServer {
    pub id: i64,
    pub name: String,
    pub address: String,
    pub protocol: String,
    pub timeout: i32,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Create upstream server request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateUpstreamServer {
    pub name: String,
    pub address: String,
    pub protocol: String,
    #[serde(default = "default_timeout")]
    pub timeout: i32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

/// Update upstream server request
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateUpstreamServer {
    pub name: Option<String>,
    pub address: Option<String>,
    pub protocol: Option<String>,
    pub timeout: Option<i32>,
    pub enabled: Option<bool>,
}

/// Query log entity
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct QueryLog {
    pub id: i64,
    pub client_ip: String,
    pub query_name: String,
    pub query_type: String,
    pub response_code: Option<String>,
    pub response_time: Option<i32>,
    pub cache_hit: bool,
    pub upstream_used: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Create query log request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateQueryLog {
    pub client_ip: String,
    pub query_name: String,
    pub query_type: String,
    pub response_code: Option<String>,
    pub response_time: Option<i32>,
    #[serde(default)]
    pub cache_hit: bool,
    pub upstream_used: Option<String>,
}

/// System config entity
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[allow(dead_code)]
pub struct SystemConfig {
    pub key: String,
    pub value: String,
    pub updated_at: DateTime<Utc>,
}

/// Query log filter for pagination and filtering
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryLogFilter {
    pub query_name: Option<String>,
    pub query_type: Option<String>,
    pub client_ip: Option<String>,
    pub cache_hit: Option<bool>,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Pagination result wrapper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedResult<T> {
    pub items: Vec<T>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

// Default value functions
fn default_ttl() -> i32 {
    300
}

fn default_timeout() -> i32 {
    5000
}

fn default_enabled() -> bool {
    true
}

/// Server listener configuration entity
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ServerListener {
    pub id: i64,
    pub protocol: String,
    pub enabled: bool,
    pub bind_address: String,
    pub port: i32,
    pub tls_cert: Option<String>,
    pub tls_key: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Update server listener request
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateServerListener {
    pub enabled: Option<bool>,
    pub bind_address: Option<String>,
    pub port: Option<i32>,
    pub tls_cert: Option<String>,
    pub tls_key: Option<String>,
}
