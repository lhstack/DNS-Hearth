//! Upstream server use cases.

use std::sync::Arc;

use crate::dns::UpstreamManager;
use crate::infrastructure::common::error::{AppError, AppResult};
use crate::infrastructure::repository::{
    CreateUpstreamServer, Database, UpdateUpstreamServer, UpstreamServer,
};

/// Health and traffic snapshot of a single upstream server.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UpstreamStatus {
    pub id: i64,
    pub name: String,
    pub address: String,
    pub protocol: String,
    pub enabled: bool,
    pub healthy: bool,
    pub queries: u64,
    pub successes: u64,
    pub failures: u64,
    pub success_rate: f64,
    pub avg_response_time_ms: u64,
    pub suspended: bool,
    pub suspension_remaining_secs: Option<u64>,
}

pub struct UpstreamBusiness {
    db: Arc<Database>,
    upstream_manager: Arc<UpstreamManager>,
}

impl UpstreamBusiness {
    pub fn new(db: Arc<Database>, upstream_manager: Arc<UpstreamManager>) -> Self {
        Self {
            db,
            upstream_manager,
        }
    }

    pub async fn list_paged(
        &self,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<UpstreamServer>, i64)> {
        Ok(self
            .db
            .upstream_servers()
            .list_paged(page, page_size)
            .await?)
    }

    pub async fn get(&self, id: i64) -> AppResult<UpstreamServer> {
        self.db
            .upstream_servers()
            .get_by_id(id)
            .await?
            .ok_or_else(|| Self::not_found(id))
    }

    /// Creates a server and republishes the active server set so the change
    /// takes effect on the resolving path without a restart.
    pub async fn create(&self, create: CreateUpstreamServer) -> AppResult<UpstreamServer> {
        let server = self.db.upstream_servers().create(create).await?;
        self.reload_active_servers().await?;
        Ok(server)
    }

    pub async fn update(&self, id: i64, update: UpdateUpstreamServer) -> AppResult<UpstreamServer> {
        let server = self
            .db
            .upstream_servers()
            .update(id, update)
            .await?
            .ok_or_else(|| Self::not_found(id))?;
        self.reload_active_servers().await?;
        Ok(server)
    }

    pub async fn delete(&self, id: i64) -> AppResult<()> {
        if !self.db.upstream_servers().delete(id).await? {
            return Err(Self::not_found(id));
        }
        self.reload_active_servers().await?;
        Ok(())
    }

    pub async fn list_status(&self) -> AppResult<Vec<UpstreamStatus>> {
        let servers = self.db.upstream_servers().list().await?;
        let stats = self.upstream_manager.get_all_stats().await;

        Ok(servers
            .into_iter()
            .map(|server| {
                let stat = stats.get(&server.id);
                UpstreamStatus {
                    id: server.id,
                    name: server.name,
                    address: server.address,
                    protocol: server.protocol,
                    enabled: server.enabled,
                    healthy: stat.map(|s| s.is_healthy()).unwrap_or(server.enabled),
                    queries: stat.map(|s| s.queries).unwrap_or(0),
                    successes: stat.map(|s| s.successes).unwrap_or(0),
                    failures: stat.map(|s| s.failures).unwrap_or(0),
                    success_rate: stat.map(|s| s.success_rate()).unwrap_or(1.0),
                    avg_response_time_ms: stat.map(|s| s.smoothed_latency_ms()).unwrap_or(0),
                    suspended: stat.map(|s| s.is_suspended()).unwrap_or(false),
                    suspension_remaining_secs: stat.and_then(|s| s.suspension_remaining_secs()),
                }
            })
            .collect())
    }

    pub async fn reset_health(&self, id: i64) -> AppResult<()> {
        self.get(id).await?;
        self.upstream_manager.reset_health(id).await;
        Ok(())
    }

    /// Republishes the enabled server set from the database into the in-memory
    /// manager. A failure here means the resolving path still uses the previous
    /// set, so it is surfaced instead of logged and dropped.
    async fn reload_active_servers(&self) -> AppResult<()> {
        self.upstream_manager
            .reload_from_db(&self.db)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to reload upstream servers: {}", e)))
    }

    fn not_found(id: i64) -> AppError {
        AppError::NotFound(format!("Upstream server with id {} not found", id))
    }
}
