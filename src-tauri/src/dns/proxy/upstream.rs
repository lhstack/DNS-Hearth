//! Upstream Server Management
//!
//! Manages upstream DNS servers with support for multiple protocols,
//! health checking, and statistics tracking.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use super::client::create_client;
use crate::infrastructure::repository::{Database, UpstreamServer as DbUpstreamServer};

/// Supported upstream DNS protocols
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpstreamProtocol {
    /// Standard UDP DNS (port 53)
    Udp,
    /// DNS over TLS (port 853)
    Dot,
    /// DNS over HTTPS (port 443)
    Doh,
    /// DNS over QUIC (port 853)
    Doq,
    /// DNS over HTTP/3 (port 443)
    Doh3,
}

impl UpstreamProtocol {
    /// Parse from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "udp" => Some(UpstreamProtocol::Udp),
            "dot" => Some(UpstreamProtocol::Dot),
            "doh" => Some(UpstreamProtocol::Doh),
            "doq" => Some(UpstreamProtocol::Doq),
            "doh3" | "h3" => Some(UpstreamProtocol::Doh3),
            _ => None,
        }
    }

    /// Convert to string
    pub fn as_str(&self) -> &'static str {
        match self {
            UpstreamProtocol::Udp => "udp",
            UpstreamProtocol::Dot => "dot",
            UpstreamProtocol::Doh => "doh",
            UpstreamProtocol::Doq => "doq",
            UpstreamProtocol::Doh3 => "doh3",
        }
    }

    /// Get default port for this protocol
    pub fn default_port(&self) -> u16 {
        match self {
            UpstreamProtocol::Udp => 53,
            UpstreamProtocol::Dot => 853,
            UpstreamProtocol::Doh => 443,
            UpstreamProtocol::Doq => 853, // RFC 9250: DoQ uses UDP port 853
            UpstreamProtocol::Doh3 => 443, // DoH3 uses UDP port 443
        }
    }
}

impl std::fmt::Display for UpstreamProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Upstream server configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct UpstreamServer {
    /// Server ID from database
    pub id: i64,
    /// Human-readable name
    pub name: String,
    /// Server address (host:port or URL for DoH)
    pub address: String,
    /// Protocol to use
    pub protocol: UpstreamProtocol,
    /// Query timeout in milliseconds
    pub timeout: Duration,
    /// Whether this server is enabled
    pub enabled: bool,
}

#[allow(dead_code)]
impl UpstreamServer {
    /// Create a new upstream server
    pub fn new(
        id: i64,
        name: impl Into<String>,
        address: impl Into<String>,
        protocol: UpstreamProtocol,
        timeout_ms: u32,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            address: address.into(),
            protocol,
            timeout: Duration::from_millis(timeout_ms as u64),
            enabled: true,
        }
    }

    /// Create from database model
    pub fn from_db(db_server: &DbUpstreamServer) -> Option<Self> {
        let protocol = UpstreamProtocol::from_str(&db_server.protocol)?;
        Some(Self {
            id: db_server.id,
            name: db_server.name.clone(),
            address: db_server.address.clone(),
            protocol,
            timeout: Duration::from_millis(db_server.timeout as u64),
            enabled: db_server.enabled,
        })
    }

    /// Get the timeout in milliseconds
    pub fn timeout_ms(&self) -> u32 {
        self.timeout.as_millis() as u32
    }
}

/// Statistics for an upstream server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamStats {
    /// Total number of queries sent
    pub queries: u64,
    /// Number of successful responses
    pub successes: u64,
    /// Number of failures (timeouts, errors)
    pub failures: u64,
    /// Exponential moving average of response time (ms)
    pub ema_response_time_ms: f64,
    /// Last response time in milliseconds
    pub last_response_time_ms: Option<u64>,
    /// Consecutive failures since the last success; drives the circuit breaker
    pub consecutive_failures: u32,
    /// Last successful query time (not serialized)
    #[serde(skip)]
    pub last_success: Option<Instant>,
    /// Last failure time (not serialized)
    #[serde(skip)]
    pub last_failure: Option<Instant>,
    /// Whether the server is currently healthy
    pub healthy: bool,
    /// Suspension end time - server won't be used until this time
    #[serde(skip)]
    pub suspended_until: Option<Instant>,
    /// Current suspension duration in seconds (for exponential backoff)
    #[serde(skip)]
    pub suspension_duration_secs: u64,
}

impl Default for UpstreamStats {
    fn default() -> Self {
        Self {
            queries: 0,
            successes: 0,
            failures: 0,
            ema_response_time_ms: 0.0,
            last_response_time_ms: None,
            consecutive_failures: 0,
            last_success: None,
            last_failure: None,
            healthy: true,
            suspended_until: None,
            suspension_duration_secs: 0,
        }
    }
}

impl UpstreamStats {
    /// EMA smoothing factor: weight given to the newest sample.
    /// Kept low so `Fastest` does not flip servers on a single slow response.
    const EMA_ALPHA: f64 = 0.3;
    /// Consecutive failures required before a server leaves the rotation.
    const FAILURE_THRESHOLD: u32 = 3;
    /// First suspension length; doubles on each subsequent trip.
    const BASE_SUSPENSION_SECS: u64 = 30;
    /// Upper bound for the suspension backoff.
    const MAX_SUSPENSION_SECS: u64 = 300;

    /// Create new stats with healthy status
    pub fn new() -> Self {
        Self {
            healthy: true,
            ..Default::default()
        }
    }

    /// Exponentially weighted latency in milliseconds.
    ///
    /// Not an arithmetic mean: recent samples dominate, which is what strategy
    /// selection needs. Returns 0 when there is no successful sample yet.
    pub fn smoothed_latency_ms(&self) -> u64 {
        if self.successes == 0 {
            0 // No data yet
        } else {
            self.ema_response_time_ms.round() as u64
        }
    }

    /// Same value as `smoothed_latency_ms`, but ranks unmeasured servers last
    /// instead of first so they never win a latency comparison by default.
    pub fn latency_rank_key(&self) -> u64 {
        if self.successes == 0 {
            u64::MAX // No data = worst priority for sorting
        } else {
            self.ema_response_time_ms.round() as u64
        }
    }

    /// Calculate success rate (0.0 to 1.0)
    pub fn success_rate(&self) -> f64 {
        if self.queries == 0 {
            1.0 // Assume healthy if no queries yet
        } else {
            self.successes as f64 / self.queries as f64
        }
    }

    /// Record a successful query with EMA update
    pub fn record_success(&mut self, response_time_ms: u64) {
        self.queries += 1;
        self.successes += 1;
        self.last_response_time_ms = Some(response_time_ms);
        self.last_success = Some(Instant::now());
        self.healthy = true;
        self.consecutive_failures = 0;

        // Clear suspension on success - server is working again
        if self.suspended_until.is_some() {
            tracing::info!("Server recovered from suspension after successful query");
            self.suspended_until = None;
            // Halve the backoff so a flapping server still escalates faster
            // than one that has been stable all along.
            self.suspension_duration_secs /= 2;
        }

        // Update EMA
        let new_value = response_time_ms as f64;
        if self.successes == 1 {
            // First sample: initialize EMA
            self.ema_response_time_ms = new_value;
        } else {
            // EMA formula: new_ema = alpha * new_value + (1 - alpha) * old_ema
            self.ema_response_time_ms =
                Self::EMA_ALPHA * new_value + (1.0 - Self::EMA_ALPHA) * self.ema_response_time_ms;
        }
    }

    /// Record a failed query with exponential backoff suspension
    pub fn record_failure(&mut self) {
        self.queries += 1;
        self.failures += 1;
        self.last_failure = Some(Instant::now());

        self.consecutive_failures += 1;

        // Trip on consecutive failures rather than lifetime success rate: a
        // server with thousands of past successes would otherwise need
        // thousands of failures before it was taken out of rotation.
        if self.consecutive_failures >= Self::FAILURE_THRESHOLD {
            self.healthy = false;

            // Calculate suspension duration based on success rate
            // Lower success rate = longer suspension
            let base_duration = Self::BASE_SUSPENSION_SECS;
            let max_duration = Self::MAX_SUSPENSION_SECS;

            // Exponential backoff: double the previous duration, capped at max
            if self.suspension_duration_secs == 0 {
                self.suspension_duration_secs = base_duration;
            } else {
                self.suspension_duration_secs =
                    (self.suspension_duration_secs * 2).min(max_duration);
            }

            let final_duration = self.suspension_duration_secs.min(max_duration);
            self.suspended_until = Some(Instant::now() + Duration::from_secs(final_duration));

            tracing::warn!(
                "Server suspended for {}s after {} consecutive failures (success rate: {:.1}%)",
                final_duration,
                self.consecutive_failures,
                self.success_rate() * 100.0
            );
        }
    }

    /// Check if server is currently suspended
    pub fn is_suspended(&self) -> bool {
        if let Some(until) = self.suspended_until {
            if Instant::now() < until {
                return true;
            }
        }
        false
    }

    /// Get remaining suspension time in seconds
    pub fn suspension_remaining_secs(&self) -> Option<u64> {
        if let Some(until) = self.suspended_until {
            let now = Instant::now();
            if now < until {
                return Some((until - now).as_secs());
            }
        }
        None
    }

    /// Check if server should be considered healthy and available
    pub fn is_healthy(&self) -> bool {
        // Pure routing predicate. Recovery from `healthy == false` is driven by
        // the active health checker, not by suspension expiry: letting the
        // timeout implicitly restore health made `healthy` dead state and kept
        // failing servers in rotation permanently.
        !self.is_suspended() && self.healthy
    }

    /// Reset health status (for manual recovery)
    pub fn reset_health(&mut self) {
        self.healthy = true;
        self.failures = 0;
        self.consecutive_failures = 0;
        self.suspended_until = None;
        self.suspension_duration_secs = 0;
    }
}

/// Health check result
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct HealthCheckResult {
    /// Server ID
    pub server_id: i64,
    /// Whether the check passed
    pub healthy: bool,
    /// Response time if successful
    pub response_time_ms: Option<u64>,
    /// Error message if failed
    pub error: Option<String>,
}

/// Upstream Server Manager
///
/// Manages a collection of upstream DNS servers with health checking
/// and statistics tracking.
pub struct UpstreamManager {
    /// Loaded servers
    servers: RwLock<Vec<UpstreamServer>>,
    /// Statistics per server (keyed by server ID)
    stats: RwLock<HashMap<i64, UpstreamStats>>,
    /// Database connection for persistence
    db: Option<Arc<Database>>,
    /// Health check interval
    health_check_interval: Duration,
}

#[allow(dead_code)]
impl UpstreamManager {
    /// A latency sample older than this no longer reflects current conditions.
    const STATS_STALE_AFTER: Duration = Duration::from_secs(300);

    /// Create a new upstream manager without database
    pub fn new() -> Self {
        Self {
            servers: RwLock::new(Vec::new()),
            stats: RwLock::new(HashMap::new()),
            db: None,
            health_check_interval: Duration::from_secs(30),
        }
    }

    /// Create a new upstream manager with database connection
    pub fn with_db(db: Arc<Database>) -> Self {
        Self {
            servers: RwLock::new(Vec::new()),
            stats: RwLock::new(HashMap::new()),
            db: Some(db),
            health_check_interval: Duration::from_secs(30),
        }
    }

    /// Create a new upstream manager wrapped in Arc
    pub fn new_shared() -> Arc<Self> {
        Arc::new(Self::new())
    }

    /// Load servers from database
    pub async fn load_servers(&self) -> anyhow::Result<()> {
        if let Some(ref db) = self.db {
            let db_servers = db.upstream_servers().list_enabled().await?;
            let servers: Vec<UpstreamServer> = db_servers
                .iter()
                .filter_map(|s| UpstreamServer::from_db(s))
                .collect();

            // Initialize stats for each server
            let mut stats = self.stats.write().await;
            for server in &servers {
                stats.entry(server.id).or_insert_with(UpstreamStats::new);
            }

            let mut current_servers = self.servers.write().await;
            *current_servers = servers;
        }
        Ok(())
    }

    /// Reload servers from database
    pub async fn reload_servers(&self) -> anyhow::Result<()> {
        self.load_servers().await
    }

    /// Reload servers from a provided database reference
    pub async fn reload_from_db(&self, db: &Database) -> anyhow::Result<()> {
        db.checkpoint().await?;
        let db_servers = db.upstream_servers().list_enabled().await?;
        let servers: Vec<UpstreamServer> = db_servers
            .iter()
            .filter_map(|s| UpstreamServer::from_db(s))
            .collect();

        // Initialize stats for each server
        let mut stats = self.stats.write().await;
        for server in &servers {
            stats.entry(server.id).or_insert_with(UpstreamStats::new);
        }

        let mut current_servers = self.servers.write().await;
        *current_servers = servers;
        Ok(())
    }

    /// Get all enabled servers
    pub async fn get_servers(&self) -> Vec<UpstreamServer> {
        self.servers.read().await.clone()
    }

    /// Get healthy servers only (excludes suspended servers)
    pub async fn get_healthy_servers(&self) -> Vec<UpstreamServer> {
        let servers = self.servers.read().await;
        let stats = self.stats.read().await;

        servers
            .iter()
            .filter(|s| s.enabled && stats.get(&s.id).map(|st| st.is_healthy()).unwrap_or(true))
            .cloned()
            .collect()
    }

    /// Get a server by ID
    pub async fn get_server(&self, id: i64) -> Option<UpstreamServer> {
        self.servers
            .read()
            .await
            .iter()
            .find(|s| s.id == id)
            .cloned()
    }

    /// Add a server (in-memory only)
    pub async fn add_server(&self, server: UpstreamServer) {
        let mut servers = self.servers.write().await;
        let mut stats = self.stats.write().await;

        stats.entry(server.id).or_insert_with(UpstreamStats::new);
        servers.push(server);
    }

    /// Remove a server by ID
    pub async fn remove_server(&self, id: i64) {
        let mut servers = self.servers.write().await;
        let mut stats = self.stats.write().await;

        servers.retain(|s| s.id != id);
        stats.remove(&id);
    }

    /// Get statistics for a server
    pub async fn get_stats(&self, id: i64) -> Option<UpstreamStats> {
        self.stats.read().await.get(&id).cloned()
    }

    /// Get all statistics
    pub async fn get_all_stats(&self) -> HashMap<i64, UpstreamStats> {
        self.stats.read().await.clone()
    }

    /// Record a successful query for a server
    pub async fn record_success(&self, id: i64, response_time_ms: u64) {
        let mut stats = self.stats.write().await;
        if let Some(server_stats) = stats.get_mut(&id) {
            server_stats.record_success(response_time_ms);
        }
    }

    /// Record a failed query for a server
    pub async fn record_failure(&self, id: i64) {
        let mut stats = self.stats.write().await;
        if let Some(server_stats) = stats.get_mut(&id) {
            server_stats.record_failure();
        }
    }

    /// Reset health status for a server
    pub async fn reset_health(&self, id: i64) {
        let mut stats = self.stats.write().await;
        if let Some(server_stats) = stats.get_mut(&id) {
            server_stats.reset_health();
        }
    }

    /// Get the server with the fastest average response time
    pub async fn get_fastest_server(&self) -> Option<UpstreamServer> {
        let servers = self.get_healthy_servers().await;
        let stats = self.stats.read().await;

        servers.into_iter().min_by_key(|s| {
            stats
                .get(&s.id)
                .map(|st| st.latency_rank_key())
                .unwrap_or(u64::MAX)
        })
    }

    /// Check if any server has historical stats (at least one successful query)
    pub async fn has_any_stats(&self) -> bool {
        let stats = self.stats.read().await;
        stats.values().any(|s| s.successes > 0)
    }

    /// Check if all healthy servers have historical stats
    pub async fn all_servers_have_stats(&self) -> bool {
        let servers = self.get_healthy_servers().await;
        if servers.is_empty() {
            return false;
        }

        let stats = self.stats.read().await;
        servers
            .iter()
            .all(|s| stats.get(&s.id).map(|st| st.successes > 0).unwrap_or(false))
    }

    /// Check if any healthy server needs re-probing
    /// Returns true if:
    /// - Any server has no stats
    /// - Any server hasn't been queried in the last 5 minutes
    pub async fn servers_needing_probe(&self) -> Vec<UpstreamServer> {
        let servers = self.get_healthy_servers().await;
        if servers.is_empty() {
            return Vec::new();
        }

        let stats = self.stats.read().await;
        servers
            .into_iter()
            .filter(|s| match stats.get(&s.id) {
                None => true,
                Some(st) => match st.last_success {
                    None => true,
                    Some(last) => last.elapsed() > Self::STATS_STALE_AFTER,
                },
            })
            .collect()
    }

    /// Probe every server that has no usable latency sample, plus every server
    /// currently out of rotation so it has a path back in.
    ///
    /// Probes run concurrently and their outcome is recorded through the normal
    /// success/failure path, so a passing probe clears the unhealthy flag.
    async fn probe_servers(&self, servers: Vec<UpstreamServer>) {
        if servers.is_empty() {
            return;
        }

        let probes = servers.into_iter().map(|server| async move {
            let outcome = match create_client(server.clone()) {
                Ok(client) => client.health_check().await,
                Err(e) => Err(e),
            };
            (server, outcome)
        });

        for (server, outcome) in futures::future::join_all(probes).await {
            match outcome {
                Ok(elapsed) => {
                    let was_unhealthy = self
                        .get_stats(server.id)
                        .await
                        .map(|s| !s.is_healthy())
                        .unwrap_or(false);

                    self.record_success(server.id, elapsed.as_millis() as u64)
                        .await;

                    if was_unhealthy {
                        tracing::info!(
                            "Health probe restored {} ({}) to rotation in {}ms",
                            server.name,
                            server.address,
                            elapsed.as_millis()
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Health probe failed for {} ({}): {}",
                        server.name,
                        server.address,
                        e
                    );
                    self.record_failure(server.id).await;
                }
            }
        }
    }

    /// Servers that need a probe: those lacking a fresh latency sample and
    /// those suspended or flagged unhealthy, which no live query would reach.
    async fn probe_candidates(&self) -> Vec<UpstreamServer> {
        let servers = self.servers.read().await.clone();
        let stats = self.stats.read().await;

        servers
            .into_iter()
            .filter(|s| s.enabled)
            .filter(|s| match stats.get(&s.id) {
                None => true,
                Some(st) => {
                    if !st.is_healthy() {
                        return true;
                    }
                    match st.last_success {
                        None => true,
                        Some(last) => last.elapsed() > Self::STATS_STALE_AFTER,
                    }
                }
            })
            .collect()
    }

    /// Run one health-check sweep over all servers that need it.
    pub async fn run_health_checks(&self) {
        let candidates = self.probe_candidates().await;
        self.probe_servers(candidates).await;
    }

    /// Spawn the background health checker.
    ///
    /// This is the only mechanism that returns an unhealthy server to rotation:
    /// live traffic never reaches a server that `get_healthy_servers` excludes.
    pub fn spawn_health_checker(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        let interval_period = self.health_check_interval;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval_period);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            tracing::info!(
                "Upstream health checker started (interval: {:?})",
                interval_period
            );

            loop {
                ticker.tick().await;
                self.run_health_checks().await;
            }
        })
    }

    /// Get the number of servers
    pub async fn server_count(&self) -> usize {
        self.servers.read().await.len()
    }

    /// Clear all servers
    pub async fn clear(&self) {
        let mut servers = self.servers.write().await;
        let mut stats = self.stats.write().await;
        servers.clear();
        stats.clear();
    }
}

impl Default for UpstreamManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protocol_from_str() {
        assert_eq!(
            UpstreamProtocol::from_str("udp"),
            Some(UpstreamProtocol::Udp)
        );
        assert_eq!(
            UpstreamProtocol::from_str("UDP"),
            Some(UpstreamProtocol::Udp)
        );
        assert_eq!(
            UpstreamProtocol::from_str("dot"),
            Some(UpstreamProtocol::Dot)
        );
        assert_eq!(
            UpstreamProtocol::from_str("doh"),
            Some(UpstreamProtocol::Doh)
        );
        assert_eq!(
            UpstreamProtocol::from_str("doq"),
            Some(UpstreamProtocol::Doq)
        );
        assert_eq!(
            UpstreamProtocol::from_str("doh3"),
            Some(UpstreamProtocol::Doh3)
        );
        assert_eq!(
            UpstreamProtocol::from_str("h3"),
            Some(UpstreamProtocol::Doh3)
        );
        assert_eq!(UpstreamProtocol::from_str("invalid"), None);
    }

    #[test]
    fn test_protocol_default_port() {
        assert_eq!(UpstreamProtocol::Udp.default_port(), 53);
        assert_eq!(UpstreamProtocol::Dot.default_port(), 853);
        assert_eq!(UpstreamProtocol::Doh.default_port(), 443);
        assert_eq!(UpstreamProtocol::Doq.default_port(), 853); // RFC 9250: DoQ uses UDP port 853
        assert_eq!(UpstreamProtocol::Doh3.default_port(), 443); // DoH3 uses UDP port 443
    }

    #[test]
    fn test_upstream_server_creation() {
        let server =
            UpstreamServer::new(1, "Cloudflare", "1.1.1.1:53", UpstreamProtocol::Udp, 5000);

        assert_eq!(server.id, 1);
        assert_eq!(server.name, "Cloudflare");
        assert_eq!(server.address, "1.1.1.1:53");
        assert_eq!(server.protocol, UpstreamProtocol::Udp);
        assert_eq!(server.timeout_ms(), 5000);
        assert!(server.enabled);
    }

    #[test]
    fn test_upstream_stats_success() {
        let mut stats = UpstreamStats::new();

        stats.record_success(50);
        stats.record_success(100);

        assert_eq!(stats.queries, 2);
        assert_eq!(stats.successes, 2);
        assert_eq!(stats.failures, 0);
        assert_eq!(stats.success_rate(), 1.0);
        assert!(stats.is_healthy());
    }

    #[test]
    fn test_latency_is_ema_not_arithmetic_mean() {
        let mut stats = UpstreamStats::new();

        // First sample seeds the EMA directly.
        stats.record_success(50);
        assert_eq!(stats.smoothed_latency_ms(), 50);

        // Second sample: 0.3 * 100 + 0.7 * 50 = 65, not the arithmetic mean 75.
        stats.record_success(100);
        assert_eq!(stats.smoothed_latency_ms(), 65);
    }

    #[test]
    fn test_latency_reacts_to_sustained_change() {
        let mut stats = UpstreamStats::new();
        stats.record_success(20);
        for _ in 0..20 {
            stats.record_success(200);
        }

        // A sustained shift must converge toward the new level.
        let latency = stats.smoothed_latency_ms();
        assert!(
            latency > 190,
            "expected convergence toward 200, got {}",
            latency
        );
    }

    #[test]
    fn test_upstream_stats_failure() {
        let mut stats = UpstreamStats::new();

        stats.record_failure();
        stats.record_failure();

        assert_eq!(stats.queries, 2);
        assert_eq!(stats.successes, 0);
        assert_eq!(stats.failures, 2);
        assert_eq!(stats.success_rate(), 0.0);
    }

    #[test]
    fn test_upstream_stats_health_degradation() {
        let mut stats = UpstreamStats::new();

        // Need at least 5 queries for health check
        for _ in 0..5 {
            stats.record_failure();
        }

        assert!(!stats.is_healthy());
    }

    #[test]
    fn test_upstream_stats_reset_health() {
        let mut stats = UpstreamStats::new();

        for _ in 0..5 {
            stats.record_failure();
        }
        assert!(!stats.is_healthy());

        stats.reset_health();
        assert!(stats.is_healthy());
    }

    #[tokio::test]
    async fn test_upstream_manager_add_server() {
        let manager = UpstreamManager::new();

        let server = UpstreamServer::new(1, "Test", "8.8.8.8:53", UpstreamProtocol::Udp, 5000);

        manager.add_server(server).await;

        assert_eq!(manager.server_count().await, 1);

        let servers = manager.get_servers().await;
        assert_eq!(servers[0].name, "Test");
    }

    #[tokio::test]
    async fn test_upstream_manager_remove_server() {
        let manager = UpstreamManager::new();

        manager
            .add_server(UpstreamServer::new(
                1,
                "Test1",
                "8.8.8.8:53",
                UpstreamProtocol::Udp,
                5000,
            ))
            .await;
        manager
            .add_server(UpstreamServer::new(
                2,
                "Test2",
                "8.8.4.4:53",
                UpstreamProtocol::Udp,
                5000,
            ))
            .await;

        manager.remove_server(1).await;

        assert_eq!(manager.server_count().await, 1);
        assert!(manager.get_server(1).await.is_none());
        assert!(manager.get_server(2).await.is_some());
    }

    #[tokio::test]
    async fn test_upstream_manager_stats() {
        let manager = UpstreamManager::new();

        manager
            .add_server(UpstreamServer::new(
                1,
                "Test",
                "8.8.8.8:53",
                UpstreamProtocol::Udp,
                5000,
            ))
            .await;

        manager.record_success(1, 50).await;
        manager.record_success(1, 100).await;

        let stats = manager.get_stats(1).await.unwrap();
        assert_eq!(stats.successes, 2);
        assert_eq!(stats.smoothed_latency_ms(), 65);
    }

    #[tokio::test]
    async fn test_upstream_manager_healthy_servers() {
        let manager = UpstreamManager::new();

        manager
            .add_server(UpstreamServer::new(
                1,
                "Healthy",
                "8.8.8.8:53",
                UpstreamProtocol::Udp,
                5000,
            ))
            .await;
        manager
            .add_server(UpstreamServer::new(
                2,
                "Unhealthy",
                "8.8.4.4:53",
                UpstreamProtocol::Udp,
                5000,
            ))
            .await;

        // Make server 2 unhealthy
        for _ in 0..5 {
            manager.record_failure(2).await;
        }

        let healthy = manager.get_healthy_servers().await;
        assert_eq!(healthy.len(), 1);
        assert_eq!(healthy[0].id, 1);
    }

    #[tokio::test]
    async fn test_upstream_manager_fastest_server() {
        let manager = UpstreamManager::new();

        manager
            .add_server(UpstreamServer::new(
                1,
                "Slow",
                "8.8.8.8:53",
                UpstreamProtocol::Udp,
                5000,
            ))
            .await;
        manager
            .add_server(UpstreamServer::new(
                2,
                "Fast",
                "8.8.4.4:53",
                UpstreamProtocol::Udp,
                5000,
            ))
            .await;

        manager.record_success(1, 100).await;
        manager.record_success(2, 50).await;

        let fastest = manager.get_fastest_server().await.unwrap();
        assert_eq!(fastest.id, 2);
    }

    #[tokio::test]
    async fn test_upstream_manager_clear() {
        let manager = UpstreamManager::new();

        manager
            .add_server(UpstreamServer::new(
                1,
                "Test",
                "8.8.8.8:53",
                UpstreamProtocol::Udp,
                5000,
            ))
            .await;

        manager.clear().await;

        assert_eq!(manager.server_count().await, 0);
    }
}
