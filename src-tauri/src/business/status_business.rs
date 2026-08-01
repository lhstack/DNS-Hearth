//! System status use cases.

use std::sync::Arc;
use std::time::Instant;

use super::upstream_business::{UpstreamBusiness, UpstreamStatus};
use crate::dns::proxy::{ProxyManager, UpstreamManager};
use crate::dns::{CacheConfig, CacheManager, CacheStats};
use crate::infrastructure::common::error::AppResult;
use crate::infrastructure::repository::{Database, QueryLogWriter, QueryStats};

/// Aggregated system status.
pub struct SystemStatus {
    pub uptime_seconds: u64,
    pub cache_stats: CacheStats,
    pub cache_config: CacheConfig,
    pub query_stats: QueryStats,
    pub upstreams: Vec<UpstreamStatus>,
    pub healthy_upstreams: usize,
    pub strategy: &'static str,
    /// Query log entries discarded because the write queue was full.
    ///
    /// Non-zero means logging could not keep up with query volume; resolution
    /// itself is unaffected.
    pub dropped_query_logs: u64,
}

/// Service health probe result.
pub struct HealthReport {
    pub database: bool,
    pub cache: bool,
    pub upstreams: bool,
}

impl HealthReport {
    /// Overall status label derived from individual probes.
    pub fn status(&self) -> &'static str {
        if self.database && self.cache && self.upstreams {
            "healthy"
        } else {
            "degraded"
        }
    }
}

/// Status and health use cases.
pub struct StatusBusiness {
    db: Arc<Database>,
    cache: Arc<CacheManager>,
    proxy: Arc<ProxyManager>,
    upstream_manager: Arc<UpstreamManager>,
    upstream: Arc<UpstreamBusiness>,
    log_writer: Arc<QueryLogWriter>,
    started_at: Instant,
}

impl StatusBusiness {
    pub fn new(
        db: Arc<Database>,
        cache: Arc<CacheManager>,
        proxy: Arc<ProxyManager>,
        upstream_manager: Arc<UpstreamManager>,
        upstream: Arc<UpstreamBusiness>,
        log_writer: Arc<QueryLogWriter>,
        started_at: Instant,
    ) -> Self {
        Self {
            db,
            cache,
            proxy,
            upstream_manager,
            upstream,
            log_writer,
            started_at,
        }
    }

    /// Collect the full system status snapshot.
    pub async fn system_status(&self) -> AppResult<SystemStatus> {
        let query_stats = self.db.query_logs().get_stats().await?;
        let upstreams = self.upstream.list_status().await?;
        let healthy_upstreams = upstreams.iter().filter(|u| u.enabled && u.healthy).count();

        Ok(SystemStatus {
            uptime_seconds: self.started_at.elapsed().as_secs(),
            cache_stats: self.cache.stats().await,
            cache_config: self.cache.get_config().await,
            query_stats,
            upstreams,
            healthy_upstreams,
            strategy: self.proxy.get_strategy().await.as_str(),
            dropped_query_logs: self.log_writer.dropped_count(),
        })
    }

    /// Probe database reachability and whether any upstream can serve queries.
    pub async fn health_report(&self) -> HealthReport {
        let cache_stats = self.cache.stats().await;
        HealthReport {
            database: self.db.query_logs().get_stats().await.is_ok(),
            cache: cache_stats.hit_rate.is_finite(),
            upstreams: !self.upstream_manager.get_healthy_servers().await.is_empty(),
        }
    }
}
