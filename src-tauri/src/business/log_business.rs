//! Query log use cases.

use std::sync::Arc;

use crate::infrastructure::common::error::{AppError, AppResult};
use crate::infrastructure::repository::{
    Database, PaginatedResult, QueryLog, QueryLogFilter, QueryStats,
};

const CONFIG_KEY_AUTO_CLEANUP: &str = "log_auto_cleanup_enabled";
const CONFIG_KEY_RETENTION_DAYS: &str = "log_retention_days";

/// Retention configuration for query logs.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RetentionSettings {
    pub auto_cleanup_enabled: bool,
    pub retention_days: i64,
    pub oldest_log_date: Option<String>,
}

/// Orchestrates query log reads, exports and retention.
pub struct LogBusiness {
    db: Arc<Database>,
}

impl LogBusiness {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub async fn list(&self, filter: QueryLogFilter) -> AppResult<PaginatedResult<QueryLog>> {
        Ok(self.db.query_logs().list(filter).await?)
    }

    pub async fn stats(&self) -> AppResult<QueryStats> {
        Ok(self.db.query_logs().get_stats().await?)
    }

    /// Loads logs for export with an explicit row cap to bound memory use.
    pub async fn list_for_export(
        &self,
        mut filter: QueryLogFilter,
        max_rows: i64,
    ) -> AppResult<Vec<QueryLog>> {
        filter.limit = Some(max_rows);
        filter.offset = Some(0);
        Ok(self.db.query_logs().list(filter).await?.items)
    }

    pub async fn cleanup_before_date(&self, before_date: &str) -> AppResult<u64> {
        if before_date.trim().is_empty() {
            return Err(AppError::Validation("Date cannot be empty".to_string()));
        }
        Ok(self.db.query_logs().delete_before_date(before_date).await?)
    }

    pub async fn cleanup_all(&self) -> AppResult<u64> {
        Ok(self.db.query_logs().delete_all().await?)
    }

    pub async fn retention_settings(&self) -> AppResult<RetentionSettings> {
        let config = self.db.system_config();
        let auto_cleanup_enabled =
            config.get(CONFIG_KEY_AUTO_CLEANUP).await?.as_deref() == Some("true");
        let retention_days = match config.get(CONFIG_KEY_RETENTION_DAYS).await? {
            Some(value) => value.parse::<i64>().map_err(|_| {
                AppError::Config(format!("Invalid {}: {}", CONFIG_KEY_RETENTION_DAYS, value))
            })?,
            None => 30,
        };

        Ok(RetentionSettings {
            auto_cleanup_enabled,
            retention_days,
            oldest_log_date: self.db.query_logs().get_oldest_date().await?,
        })
    }

    pub async fn update_retention_settings(
        &self,
        auto_cleanup_enabled: Option<bool>,
        retention_days: Option<i64>,
    ) -> AppResult<RetentionSettings> {
        if let Some(days) = retention_days {
            if days <= 0 {
                return Err(AppError::Validation(
                    "Retention days must be greater than 0".to_string(),
                ));
            }
        }

        let config = self.db.system_config();
        if let Some(enabled) = auto_cleanup_enabled {
            config
                .set(
                    CONFIG_KEY_AUTO_CLEANUP,
                    if enabled { "true" } else { "false" },
                )
                .await?;
        }
        if let Some(days) = retention_days {
            config
                .set(CONFIG_KEY_RETENTION_DAYS, &days.to_string())
                .await?;
        }

        self.retention_settings().await
    }
}
