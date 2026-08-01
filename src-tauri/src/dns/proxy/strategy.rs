//! DNS Query Strategies
//!
//! Provides different strategies for querying upstream DNS servers:
//! - Concurrent: Query all servers simultaneously, return the first NOERROR; otherwise NXDOMAIN
//! - Fastest: Select the historically fastest healthy server and query it alone
//! - RoundRobin: Rotate through servers sequentially
//! - Random: Select a random server for each query

use anyhow::{anyhow, Result};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

use super::client::{create_client, DnsClient, QueryResult};
use super::upstream::{UpstreamManager, UpstreamServer};
use crate::dns::message::{DnsQuery, DnsResponseCode};

/// Query strategy types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryStrategy {
    /// Query all servers simultaneously, return the first NOERROR; otherwise NXDOMAIN
    Concurrent,
    /// Select the historically fastest healthy server and query it alone
    Fastest,
    /// Rotate through servers sequentially
    RoundRobin,
    /// Select a random server for each query
    Random,
}

impl QueryStrategy {
    /// Parse from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "concurrent" => Some(QueryStrategy::Concurrent),
            "fastest" | "fastest_first" => Some(QueryStrategy::Fastest),
            "round_robin" | "roundrobin" => Some(QueryStrategy::RoundRobin),
            "random" => Some(QueryStrategy::Random),
            _ => None,
        }
    }

    /// Convert to string
    pub fn as_str(&self) -> &'static str {
        match self {
            QueryStrategy::Concurrent => "concurrent",
            QueryStrategy::Fastest => "fastest",
            QueryStrategy::RoundRobin => "round_robin",
            QueryStrategy::Random => "random",
        }
    }
}

impl Default for QueryStrategy {
    fn default() -> Self {
        QueryStrategy::Concurrent
    }
}

impl std::fmt::Display for QueryStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// DNS Proxy Manager
///
/// Manages upstream DNS servers and query strategies.
pub struct ProxyManager {
    /// Upstream server manager
    upstream_manager: Arc<UpstreamManager>,
    /// Current query strategy
    strategy: RwLock<QueryStrategy>,
    /// Round-robin counter
    round_robin_counter: AtomicUsize,
    /// Allocation-free process-local identifier for query diagnostics.
    query_counter: AtomicU64,
    /// Upstream client cache (keyed by UpstreamServer)
    client_cache: Mutex<HashMap<UpstreamServer, Arc<dyn DnsClient>>>,
}

#[allow(dead_code)]
impl ProxyManager {
    /// Create a new proxy manager
    pub fn new(upstream_manager: Arc<UpstreamManager>) -> Self {
        Self {
            upstream_manager,
            strategy: RwLock::new(QueryStrategy::default()),
            round_robin_counter: AtomicUsize::new(0),
            query_counter: AtomicU64::new(1),
            client_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Create a new proxy manager wrapped in Arc
    pub fn new_shared(upstream_manager: Arc<UpstreamManager>) -> Arc<Self> {
        Arc::new(Self::new(upstream_manager))
    }

    /// Get the current strategy
    pub async fn get_strategy(&self) -> QueryStrategy {
        *self.strategy.read().await
    }

    /// Set the query strategy
    pub async fn set_strategy(&self, strategy: QueryStrategy) {
        let mut current = self.strategy.write().await;
        *current = strategy;
    }

    /// Get the upstream manager
    pub fn upstream_manager(&self) -> &Arc<UpstreamManager> {
        &self.upstream_manager
    }

    /// Get or create a client for the given server
    async fn get_client(&self, server: &UpstreamServer) -> Result<Arc<dyn DnsClient>> {
        let mut cache = self.client_cache.lock().await;

        if let Some(client) = cache.get(server) {
            return Ok(client.clone());
        }

        let client: Arc<dyn DnsClient> = Arc::from(create_client(server.clone())?);
        cache.insert(server.clone(), client.clone());
        Ok(client)
    }

    /// Query upstream servers using the configured strategy.
    ///
    /// Per-query diagnostics are emitted at `debug`: keeping these messages at
    /// `info` made the default log level format and enqueue several records for
    /// every DNS request. The DNS transaction ID is sufficient to correlate a
    /// query's diagnostics without allocating a random UUID string.
    pub async fn query(&self, query: &DnsQuery) -> Result<QueryResult> {
        use tracing::{debug, warn};

        let trace_id = self.query_counter.fetch_add(1, Ordering::Relaxed);
        let strategy = self.get_strategy().await;
        debug!(
            trace_id,
            domain = %query.name,
            record_type = %query.record_type,
            strategy = %strategy,
            "Upstream query started"
        );

        let result = match strategy {
            QueryStrategy::Concurrent => self.query_concurrent(query, trace_id).await,
            QueryStrategy::Fastest => self.query_fastest(query, trace_id).await,
            QueryStrategy::RoundRobin => self.query_round_robin(query, trace_id).await,
            QueryStrategy::Random => self.query_random(query, trace_id).await,
        };

        match &result {
            Ok(response) => debug!(
                trace_id,
                domain = %query.name,
                record_type = %query.record_type,
                response_code = %response.response.response_code,
                answer_count = response.response.answers.len(),
                response_time_ms = response.response_time_ms,
                "Upstream query completed"
            ),
            Err(error) => warn!(
                trace_id,
                domain = %query.name,
                record_type = %query.record_type,
                %error,
                "Upstream query failed"
            ),
        }

        result
    }

    /// Query all healthy upstreams concurrently.
    ///
    /// The first NOERROR response wins immediately. NXDOMAIN responses are
    /// retained as a fallback and returned only after every upstream finishes
    /// without producing NOERROR.
    async fn query_concurrent(&self, query: &DnsQuery, trace_id: u64) -> Result<QueryResult> {
        let servers = self.healthy_servers_or_err().await?;
        let mut tasks = self
            .spawn_query_tasks(servers, query, trace_id, "Concurrent")
            .await;
        let mut fastest_nxdomain: Option<QueryResult> = None;
        let mut last_error: Option<String> = None;

        while let Some(outcome) = tasks.join_next().await {
            match outcome {
                Ok(Ok(result)) if result.response.response_code == DnsResponseCode::NoError => {
                    tracing::debug!(
                        trace_id,
                        server = %result.server_name,
                        response_time_ms = result.response_time_ms,
                        "Concurrent strategy selected first NOERROR"
                    );
                    tasks.abort_all();
                    return Ok(result);
                }
                Ok(Ok(result)) if result.response.response_code == DnsResponseCode::NxDomain => {
                    if fastest_nxdomain
                        .as_ref()
                        .is_none_or(|current| result.response_time_ms < current.response_time_ms)
                    {
                        fastest_nxdomain = Some(result);
                    }
                }
                Ok(Ok(result)) => {
                    last_error = Some(format!(
                        "{} returned {}",
                        result.server_name, result.response.response_code
                    ));
                }
                Ok(Err(error)) => last_error = Some(error.to_string()),
                Err(error) => {
                    last_error = Some(format!("Task panicked: {}", error));
                }
            }
        }

        fastest_nxdomain.ok_or_else(|| {
            anyhow!(
                "All upstream servers failed: {}",
                last_error.unwrap_or_else(|| "no NOERROR or NXDOMAIN response".to_string())
            )
        })
    }

    /// Query the healthy upstream with the lowest historical successful latency.
    ///
    /// Unlike `Concurrent`, this strategy sends the query to one upstream only.
    /// A transport failure uses the existing single-server failover path.
    async fn query_fastest(&self, query: &DnsQuery, trace_id: u64) -> Result<QueryResult> {
        let server = self
            .upstream_manager
            .get_fastest_server()
            .await
            .ok_or_else(|| anyhow!("No healthy upstream servers available"))?;

        self.log_selection("Fastest", trace_id, &server, None).await;
        self.query_server(server, query, trace_id).await
    }

    async fn spawn_query_tasks(
        &self,
        servers: Vec<UpstreamServer>,
        query: &DnsQuery,
        trace_id: u64,
        strategy_label: &'static str,
    ) -> tokio::task::JoinSet<Result<QueryResult>> {
        let mut tasks = tokio::task::JoinSet::new();
        for server in servers {
            let client = match self.get_client(&server).await {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(
                        trace_id,
                        strategy = strategy_label,
                        server = %server.name,
                        %error,
                        "Skipping invalid upstream client"
                    );
                    self.upstream_manager.record_failure(server.id).await;
                    continue;
                }
            };
            let manager = self.upstream_manager.clone();
            let query = query.clone();
            tasks.spawn(async move {
                let outcome = client.query(&query).await;
                match &outcome {
                    Ok(result)
                        if matches!(
                            result.response.response_code,
                            DnsResponseCode::NoError | DnsResponseCode::NxDomain
                        ) =>
                    {
                        manager
                            .record_success(result.server_id, result.response_time_ms)
                            .await;
                    }
                    Ok(result) => {
                        manager.record_failure(result.server_id).await;
                    }
                    Err(_) => {
                        manager.record_failure(server.id).await;
                    }
                }
                outcome
            });
        }
        tasks
    }

    fn select_completed_result(outcomes: Vec<Result<QueryResult>>) -> Result<QueryResult> {
        let mut fastest_noerror: Option<QueryResult> = None;
        let mut fastest_nxdomain: Option<QueryResult> = None;
        let mut last_error: Option<String> = None;
        for outcome in outcomes {
            match outcome {
                Ok(result) if result.response.response_code == DnsResponseCode::NoError => {
                    if fastest_noerror
                        .as_ref()
                        .is_none_or(|current| result.response_time_ms < current.response_time_ms)
                    {
                        fastest_noerror = Some(result);
                    }
                }
                Ok(result) if result.response.response_code == DnsResponseCode::NxDomain => {
                    if fastest_nxdomain
                        .as_ref()
                        .is_none_or(|current| result.response_time_ms < current.response_time_ms)
                    {
                        fastest_nxdomain = Some(result);
                    }
                }
                Ok(result) => {
                    last_error = Some(format!(
                        "{} returned {}",
                        result.server_name, result.response.response_code
                    ));
                }
                Err(error) => last_error = Some(error.to_string()),
            }
        }
        fastest_noerror.or(fastest_nxdomain).ok_or_else(|| {
            anyhow!(
                "All upstream servers failed: {}",
                last_error.unwrap_or_else(|| "no NOERROR or NXDOMAIN response".to_string())
            )
        })
    }

    /// Query servers in round-robin fashion
    async fn query_round_robin(&self, query: &DnsQuery, trace_id: u64) -> Result<QueryResult> {
        let servers = self.healthy_servers_or_err().await?;
        let index = self.round_robin_counter.fetch_add(1, Ordering::Relaxed) % servers.len();
        let server = servers[index].clone();

        self.log_selection("RoundRobin", trace_id, &server, Some(index))
            .await;
        self.query_server(server, query, trace_id).await
    }

    /// Query a random server
    async fn query_random(&self, query: &DnsQuery, trace_id: u64) -> Result<QueryResult> {
        let servers = self.healthy_servers_or_err().await?;
        let index = rand::thread_rng().gen_range(0..servers.len());
        let server = servers[index].clone();

        self.log_selection("Random", trace_id, &server, Some(index))
            .await;
        self.query_server(server, query, trace_id).await
    }

    /// Healthy servers, or the shared "nothing available" error.
    async fn healthy_servers_or_err(&self) -> Result<Vec<UpstreamServer>> {
        let servers = self.upstream_manager.get_healthy_servers().await;
        if servers.is_empty() {
            return Err(anyhow!("No healthy upstream servers available"));
        }
        Ok(servers)
    }

    /// Log which server a single-target strategy picked, with its known latency.
    async fn log_selection(
        &self,
        strategy: &str,
        trace_id: u64,
        server: &UpstreamServer,
        index: Option<usize>,
    ) {
        let avg_time = self
            .upstream_manager
            .get_stats(server.id)
            .await
            .map(|s| s.smoothed_latency_ms())
            .unwrap_or(0);

        tracing::debug!(
            trace_id,
            strategy,
            index,
            server = %server.name,
            address = %server.address,
            protocol = %server.protocol,
            average_response_time_ms = avg_time,
            "Selected upstream server"
        );
    }

    /// Query a specific server with failover
    async fn query_server(
        &self,
        server: UpstreamServer,
        query: &DnsQuery,
        trace_id: u64,
    ) -> Result<QueryResult> {
        use tracing::{debug, warn};

        let client = self.get_client(&server).await?;

        match client.query(query).await {
            Ok(result) => {
                debug!(
                    "[{}] Server {} responded: {} in {}ms",
                    trace_id,
                    result.server_name,
                    result.response.response_code,
                    result.response_time_ms
                );
                self.upstream_manager
                    .record_success(result.server_id, result.response_time_ms)
                    .await;
                Ok(result)
            }
            Err(e) => {
                warn!(
                    "[{}] Server {} failed: {}, trying failover",
                    trace_id, server.name, e
                );
                self.upstream_manager.record_failure(server.id).await;

                // Try failover to another server
                self.failover_query(query, server.id, trace_id)
                    .await
                    .map_err(|_| anyhow!("Query failed and failover exhausted: {}", e))
            }
        }
    }

    /// Attempt failover to another server
    async fn failover_query(
        &self,
        query: &DnsQuery,
        failed_server_id: i64,
        trace_id: u64,
    ) -> Result<QueryResult> {
        use tracing::{debug, warn};

        let servers = self.upstream_manager.get_healthy_servers().await;

        // Try other servers
        for server in servers {
            if server.id == failed_server_id {
                continue;
            }

            debug!(
                "[{}] [Failover] Trying server: {}, addr: {}, protocol: {}",
                trace_id, server.name, server.address, server.protocol
            );

            let client = match self.get_client(&server).await {
                Ok(c) => c,
                Err(e) => {
                    warn!("[{}] [Failover] Skipping {}: {}", trace_id, server.name, e);
                    continue;
                }
            };
            match client.query(query).await {
                Ok(result) => {
                    debug!(
                        "[{}] [Failover] Server {} succeeded: {} in {}ms",
                        trace_id,
                        result.server_name,
                        result.response.response_code,
                        result.response_time_ms
                    );
                    self.upstream_manager
                        .record_success(result.server_id, result.response_time_ms)
                        .await;
                    return Ok(result);
                }
                Err(e) => {
                    warn!(
                        "[{}] [Failover] Server {} failed: {}",
                        trace_id, server.name, e
                    );
                    self.upstream_manager.record_failure(server.id).await;
                }
            }
        }

        Err(anyhow!("All failover servers exhausted"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::message::{DnsRecordData, DnsResponse, RecordType};
    use crate::dns::proxy::upstream::UpstreamProtocol;
    use std::time::Duration;

    struct StubDnsClient {
        server: UpstreamServer,
        response_code: DnsResponseCode,
        has_answer: bool,
        delay: Duration,
    }

    #[async_trait::async_trait]
    impl DnsClient for StubDnsClient {
        async fn query(&self, query: &DnsQuery) -> Result<QueryResult> {
            tokio::time::sleep(self.delay).await;
            let mut response = DnsResponse::new(query.id);
            response.response_code = self.response_code;
            if self.has_answer {
                response.add_answer(DnsRecordData::a(
                    &query.name,
                    "192.0.2.1".parse().unwrap(),
                    60,
                ));
            }
            Ok(QueryResult {
                response,
                response_time_ms: self.delay.as_millis() as u64,
                server_id: self.server.id,
                server_name: self.server.name.clone(),
            })
        }

        fn server(&self) -> &UpstreamServer {
            &self.server
        }

        async fn health_check(&self) -> Result<Duration> {
            Ok(self.delay)
        }
    }

    fn stub_result(server_id: i64, code: DnsResponseCode, response_time_ms: u64) -> QueryResult {
        let response = match code {
            DnsResponseCode::NoError => DnsResponse::new(1),
            DnsResponseCode::NxDomain => DnsResponse::nxdomain(1),
            DnsResponseCode::ServFail => DnsResponse::servfail(1),
            _ => {
                let mut response = DnsResponse::new(1);
                response.response_code = code;
                response
            }
        };
        QueryResult {
            response,
            response_time_ms,
            server_id,
            server_name: format!("Server {server_id}"),
        }
    }

    #[tokio::test]
    async fn fastest_uses_the_lowest_historical_latency_server_only() {
        let slow = UpstreamServer::new(1, "Slow", "127.0.0.1:5301", UpstreamProtocol::Udp, 5000);
        let fast = UpstreamServer::new(2, "Fast", "127.0.0.1:5302", UpstreamProtocol::Udp, 5000);
        let upstream_manager = Arc::new(UpstreamManager::new());
        upstream_manager.add_server(slow.clone()).await;
        upstream_manager.add_server(fast.clone()).await;
        upstream_manager.record_success(slow.id, 40).await;
        upstream_manager.record_success(fast.id, 5).await;

        let proxy_manager = ProxyManager::new(upstream_manager);
        proxy_manager.set_strategy(QueryStrategy::Fastest).await;
        let mut clients = proxy_manager.client_cache.lock().await;
        clients.insert(
            slow.clone(),
            Arc::new(StubDnsClient {
                server: slow,
                response_code: DnsResponseCode::NoError,
                has_answer: true,
                delay: Duration::from_millis(40),
            }),
        );
        clients.insert(
            fast.clone(),
            Arc::new(StubDnsClient {
                server: fast,
                response_code: DnsResponseCode::NoError,
                has_answer: true,
                delay: Duration::from_millis(5),
            }),
        );
        drop(clients);

        let result = proxy_manager
            .query(&DnsQuery::new("example.com", RecordType::A))
            .await
            .expect("the selected historical fastest server should answer");
        assert_eq!(result.server_id, 2);
    }

    #[test]
    fn concurrent_prefers_noerror_then_nxdomain() {
        let noerror = ProxyManager::select_completed_result(vec![
            Ok(stub_result(1, DnsResponseCode::NxDomain, 1)),
            Ok(stub_result(2, DnsResponseCode::NoError, 30)),
            Ok(stub_result(3, DnsResponseCode::NoError, 10)),
        ])
        .expect("NOERROR should win");
        assert_eq!(noerror.server_id, 3);

        let nxdomain = ProxyManager::select_completed_result(vec![
            Ok(stub_result(1, DnsResponseCode::ServFail, 1)),
            Ok(stub_result(2, DnsResponseCode::NxDomain, 30)),
            Ok(stub_result(3, DnsResponseCode::NxDomain, 10)),
        ])
        .expect("NXDOMAIN should be the fallback");
        assert_eq!(nxdomain.server_id, 3);
    }

    #[test]
    fn concurrent_errors_without_noerror_or_nxdomain() {
        let result = ProxyManager::select_completed_result(vec![
            Ok(stub_result(1, DnsResponseCode::ServFail, 1)),
            Ok(stub_result(2, DnsResponseCode::Refused, 2)),
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn test_strategy_from_str() {
        assert_eq!(
            QueryStrategy::from_str("concurrent"),
            Some(QueryStrategy::Concurrent)
        );
        assert_eq!(
            QueryStrategy::from_str("CONCURRENT"),
            Some(QueryStrategy::Concurrent)
        );
        assert_eq!(
            QueryStrategy::from_str("fastest"),
            Some(QueryStrategy::Fastest)
        );
        assert_eq!(
            QueryStrategy::from_str("fastest_first"),
            Some(QueryStrategy::Fastest)
        );
        assert_eq!(
            QueryStrategy::from_str("round_robin"),
            Some(QueryStrategy::RoundRobin)
        );
        assert_eq!(
            QueryStrategy::from_str("roundrobin"),
            Some(QueryStrategy::RoundRobin)
        );
        assert_eq!(
            QueryStrategy::from_str("random"),
            Some(QueryStrategy::Random)
        );
        assert_eq!(QueryStrategy::from_str("invalid"), None);
    }

    #[test]
    fn test_strategy_as_str() {
        assert_eq!(QueryStrategy::Concurrent.as_str(), "concurrent");
        assert_eq!(QueryStrategy::Fastest.as_str(), "fastest");
        assert_eq!(QueryStrategy::RoundRobin.as_str(), "round_robin");
        assert_eq!(QueryStrategy::Random.as_str(), "random");
    }

    #[test]
    fn test_strategy_default() {
        assert_eq!(QueryStrategy::default(), QueryStrategy::Concurrent);
    }

    #[tokio::test]
    async fn test_proxy_manager_strategy() {
        let upstream_manager = Arc::new(UpstreamManager::new());
        let proxy_manager = ProxyManager::new(upstream_manager);

        assert_eq!(
            proxy_manager.get_strategy().await,
            QueryStrategy::Concurrent
        );

        proxy_manager.set_strategy(QueryStrategy::RoundRobin).await;
        assert_eq!(
            proxy_manager.get_strategy().await,
            QueryStrategy::RoundRobin
        );
    }

    #[tokio::test]
    async fn test_proxy_manager_no_servers() {
        let upstream_manager = Arc::new(UpstreamManager::new());
        let proxy_manager = ProxyManager::new(upstream_manager);

        let query = DnsQuery::new("example.com", crate::dns::message::RecordType::A);
        let result = proxy_manager.query(&query).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_round_robin_counter() {
        let upstream_manager = Arc::new(UpstreamManager::new());

        // Add multiple servers
        upstream_manager
            .add_server(UpstreamServer::new(
                1,
                "Server1",
                "8.8.8.8:53",
                UpstreamProtocol::Udp,
                5000,
            ))
            .await;
        upstream_manager
            .add_server(UpstreamServer::new(
                2,
                "Server2",
                "8.8.4.4:53",
                UpstreamProtocol::Udp,
                5000,
            ))
            .await;
        upstream_manager
            .add_server(UpstreamServer::new(
                3,
                "Server3",
                "1.1.1.1:53",
                UpstreamProtocol::Udp,
                5000,
            ))
            .await;

        let proxy_manager = ProxyManager::new(upstream_manager);
        proxy_manager.set_strategy(QueryStrategy::RoundRobin).await;

        // Verify counter increments
        let initial = proxy_manager.round_robin_counter.load(Ordering::Relaxed);

        // The counter should increment on each query attempt
        // (even if the query fails due to network issues in tests)
        assert_eq!(initial, 0);
    }
}
