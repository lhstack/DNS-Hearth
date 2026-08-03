//! Rewrite rule use cases.
//!
//! Every mutation reloads the in-memory rewrite engine so the DNS data plane
//! and the database never diverge. A failed reload is reported, not swallowed.

use std::sync::Arc;

use crate::dns::RewriteEngine;
use crate::infrastructure::common::error::{AppError, AppResult};
use crate::infrastructure::repository::{
    CreateRewriteRule, Database, RewriteRule, RewriteRuleStats, UpdateRewriteRule,
};

/// Orchestrates rewrite rule management and engine reloads.
pub struct RewriteBusiness {
    db: Arc<Database>,
    engine: Arc<RewriteEngine>,
}

impl RewriteBusiness {
    pub fn new(db: Arc<Database>, engine: Arc<RewriteEngine>) -> Self {
        Self { db, engine }
    }

    pub async fn list_paged(
        &self,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<RewriteRule>, RewriteRuleStats)> {
        Ok(self.db.rewrite_rules().list_paged(page, page_size).await?)
    }

    pub async fn create(&self, rule: CreateRewriteRule) -> AppResult<RewriteRule> {
        let created = self.db.rewrite_rules().create(rule).await?;
        self.reload_engine().await?;
        Ok(created)
    }

    pub async fn update(&self, id: i64, update: UpdateRewriteRule) -> AppResult<RewriteRule> {
        let updated = self
            .db
            .rewrite_rules()
            .update(id, update)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Rewrite rule with id {} not found", id)))?;
        self.reload_engine().await?;
        Ok(updated)
    }

    pub async fn delete(&self, id: i64) -> AppResult<()> {
        if !self.db.rewrite_rules().delete(id).await? {
            return Err(AppError::NotFound(format!(
                "Rewrite rule with id {} not found",
                id
            )));
        }
        self.reload_engine().await
    }

    pub async fn batch_create(&self, rules: Vec<CreateRewriteRule>) -> AppResult<i64> {
        let count = self.db.rewrite_rules().batch_create(rules).await?;
        self.reload_engine().await?;
        Ok(count)
    }

    async fn reload_engine(&self) -> AppResult<()> {
        self.engine
            .load_rules()
            .await
            .map_err(|e| AppError::Internal(format!("Failed to reload rewrite rules: {}", e)))
    }
}
