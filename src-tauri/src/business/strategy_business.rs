//! Query strategy use cases.

use std::sync::Arc;

use crate::dns::proxy::{ProxyManager, QueryStrategy};
use crate::infrastructure::common::error::AppResult;
use crate::infrastructure::repository::Database;

const CONFIG_KEY_QUERY_STRATEGY: &str = "query_strategy";

/// Orchestrates reading and switching the upstream query strategy.
pub struct StrategyBusiness {
    db: Arc<Database>,
    proxy: Arc<ProxyManager>,
}

impl StrategyBusiness {
    pub fn new(db: Arc<Database>, proxy: Arc<ProxyManager>) -> Self {
        Self { db, proxy }
    }

    pub async fn current(&self) -> QueryStrategy {
        self.proxy.get_strategy().await
    }

    /// Switches the live strategy and persists it so restarts keep the choice.
    pub async fn update(&self, strategy: QueryStrategy) -> AppResult<QueryStrategy> {
        self.proxy.set_strategy(strategy).await;
        self.db
            .system_config()
            .set(CONFIG_KEY_QUERY_STRATEGY, strategy.as_str())
            .await?;
        tracing::info!("Query strategy switched to {}", strategy);
        Ok(strategy)
    }
}
