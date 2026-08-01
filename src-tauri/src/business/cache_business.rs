//! DNS cache use cases.

use std::sync::Arc;

use crate::dns::{CacheConfig, CacheManager, CacheStats};
use crate::infrastructure::common::error::{AppError, AppResult};
use crate::infrastructure::repository::Database;

const CONFIG_KEY_DEFAULT_TTL: &str = "cache_default_ttl";
const CONFIG_KEY_MAX_ENTRIES: &str = "cache_max_entries";

/// Orchestrates cache inspection and configuration.
pub struct CacheBusiness {
    db: Arc<Database>,
    cache: Arc<CacheManager>,
}

impl CacheBusiness {
    pub fn new(db: Arc<Database>, cache: Arc<CacheManager>) -> Self {
        Self { db, cache }
    }

    pub fn cache(&self) -> Arc<CacheManager> {
        self.cache.clone()
    }

    pub async fn stats(&self) -> CacheStats {
        self.cache.stats().await
    }

    pub async fn config(&self) -> CacheConfig {
        self.cache.get_config().await
    }

    /// Applies the new config to the live cache and persists it. A failed write
    /// is surfaced instead of leaving memory and database out of sync silently.
    pub async fn update_config(
        &self,
        default_ttl: Option<u64>,
        max_entries: Option<usize>,
    ) -> AppResult<CacheConfig> {
        let mut config = self.cache.get_config().await;
        if let Some(ttl) = default_ttl {
            config.default_ttl = ttl;
        }
        if let Some(entries) = max_entries {
            config.max_entries = entries;
        }

        self.cache.update_config(config.clone()).await;

        let repo = self.db.system_config();
        repo.set(CONFIG_KEY_DEFAULT_TTL, &config.default_ttl.to_string())
            .await?;
        repo.set(CONFIG_KEY_MAX_ENTRIES, &config.max_entries.to_string())
            .await?;

        tracing::info!(
            "Cache config updated: ttl={}s, max_entries={}",
            config.default_ttl,
            config.max_entries
        );
        Ok(config)
    }

    pub async fn clear_all(&self) {
        self.cache.clear().await;
    }

    pub async fn clear_domain(&self, domain: &str) -> AppResult<()> {
        if domain.trim().is_empty() {
            return Err(AppError::Validation("Domain cannot be empty".to_string()));
        }
        self.cache.clear_domain(domain).await;
        Ok(())
    }

    /// Drops expired entries and returns the remaining entry count.
    pub async fn cleanup_expired(&self) -> usize {
        self.cache.cleanup_expired().await;
        self.cache.stats().await.entries
    }
}
