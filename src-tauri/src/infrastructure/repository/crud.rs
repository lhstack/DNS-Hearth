//! Database repositories
//!
//! CRUD operations for all database entities.

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::sync::Arc;

use super::models::*;
use super::stats_cache::StatsCache;

/// Repository for DNS records
pub struct DnsRecordRepository {
    pool: SqlitePool,
}

impl DnsRecordRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Create a new DNS record
    pub async fn create(&self, record: CreateDnsRecord) -> Result<DnsRecord> {
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query_as::<_, DnsRecord>(
            r#"
            INSERT INTO dns_records (name, record_type, value, ttl, priority, enabled, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            RETURNING *
            "#,
        )
        .bind(&record.name)
        .bind(&record.record_type)
        .bind(&record.value)
        .bind(record.ttl)
        .bind(record.priority)
        .bind(record.enabled)
        .bind(now)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(result)
    }

    /// Get a DNS record by ID
    pub async fn get_by_id(&self, id: i64) -> Result<Option<DnsRecord>> {
        let result = sqlx::query_as::<_, DnsRecord>("SELECT * FROM dns_records WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(result)
    }

    /// Get DNS records by name
    #[allow(dead_code)]
    pub async fn get_by_name(&self, name: &str) -> Result<Vec<DnsRecord>> {
        let result = sqlx::query_as::<_, DnsRecord>(
            "SELECT * FROM dns_records WHERE name = ? AND enabled = TRUE",
        )
        .bind(name)
        .fetch_all(&self.pool)
        .await?;

        Ok(result)
    }

    /// Get DNS records by name and type
    pub async fn get_by_name_and_type(
        &self,
        name: &str,
        record_type: &str,
    ) -> Result<Vec<DnsRecord>> {
        let result = sqlx::query_as::<_, DnsRecord>(
            "SELECT * FROM dns_records WHERE name = ? AND record_type = ? AND enabled = TRUE",
        )
        .bind(name)
        .bind(record_type)
        .fetch_all(&self.pool)
        .await?;

        Ok(result)
    }

    /// Get DNS records by name and type with wildcard support
    ///
    /// This method first tries exact match, then falls back to wildcard matching.
    /// Wildcard records are stored as `*.example.com` and match any subdomain.
    ///
    /// Matching priority (highest to lowest):
    /// 1. Exact match: `sub.example.com`
    /// 2. Direct wildcard: `*.example.com` matches `sub.example.com`
    /// 3. Parent wildcard: `*.example.com` matches `a.b.example.com`
    pub async fn get_by_name_and_type_with_wildcard(
        &self,
        name: &str,
        record_type: &str,
    ) -> Result<Vec<DnsRecord>> {
        // Build all possible wildcard names
        // For "a.b.example.com", generate: ["*.b.example.com", "*.example.com", "*.com"]
        let parts: Vec<&str> = name.split('.').collect();
        let mut wildcard_names: Vec<String> = Vec::with_capacity(parts.len());

        for i in 1..parts.len() {
            wildcard_names.push(format!("*.{}", parts[i..].join(".")));
        }

        // Query exact match and all possible wildcards in one query
        // Results are ordered: exact match first, then wildcards from most specific to least
        if wildcard_names.is_empty() {
            // No wildcards possible (e.g., "com"), just do exact match
            return self.get_by_name_and_type(name, record_type).await;
        }

        // Build placeholders for IN clause
        let placeholders: Vec<&str> = wildcard_names.iter().map(|_| "?").collect();
        let query = format!(
            r#"
            SELECT * FROM dns_records
            WHERE record_type = ? AND enabled = TRUE AND name IN (?, {})
            ORDER BY
                CASE
                    WHEN name = ? THEN 0
                    ELSE 1
                END ASC,
                LENGTH(name) DESC
            "#,
            placeholders.join(", ")
        );

        let mut query_builder = sqlx::query_as::<_, DnsRecord>(&query)
            .bind(record_type)
            .bind(name); // Exact match

        for wildcard in &wildcard_names {
            query_builder = query_builder.bind(wildcard);
        }

        query_builder = query_builder.bind(name); // For ORDER BY CASE

        let results = query_builder.fetch_all(&self.pool).await?;

        // Return exact match if found, otherwise return the most specific wildcard
        if results.is_empty() {
            return Ok(vec![]);
        }

        // Check if first result is exact match
        if results[0].name == name {
            // Return only exact matches
            return Ok(results.into_iter().filter(|r| r.name == name).collect());
        }

        // Return the most specific wildcard matches (all with the same name)
        let most_specific = results[0].name.clone();
        Ok(results
            .into_iter()
            .filter(|r| r.name == most_specific)
            .collect())
    }

    /// List all DNS records
    pub async fn list(&self) -> Result<Vec<DnsRecord>> {
        let result =
            sqlx::query_as::<_, DnsRecord>("SELECT * FROM dns_records ORDER BY name, record_type")
                .fetch_all(&self.pool)
                .await?;

        Ok(result)
    }

    /// List the distinct record types that have at least one enabled record.
    ///
    /// Used to build the data plane mask so the resolver can skip the local
    /// record lookup for types that cannot possibly match.
    pub async fn list_enabled_record_types(&self) -> Result<Vec<String>> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT DISTINCT record_type FROM dns_records WHERE enabled = TRUE")
                .fetch_all(&self.pool)
                .await?;

        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    /// Update a DNS record
    pub async fn update(&self, id: i64, update: UpdateDnsRecord) -> Result<Option<DnsRecord>> {
        let existing = self.get_by_id(id).await?;
        if existing.is_none() {
            return Ok(None);
        }
        let existing = existing.unwrap();

        let name = update.name.unwrap_or(existing.name);
        let record_type = update.record_type.unwrap_or(existing.record_type);
        let value = update.value.unwrap_or(existing.value);
        let ttl = update.ttl.unwrap_or(existing.ttl);
        let priority = update.priority.unwrap_or(existing.priority);
        let enabled = update.enabled.unwrap_or(existing.enabled);

        let mut tx = self.pool.begin().await?;
        let result = sqlx::query_as::<_, DnsRecord>(
            r#"
            UPDATE dns_records
            SET name = ?, record_type = ?, value = ?, ttl = ?, priority = ?, enabled = ?, updated_at = ?
            WHERE id = ?
            RETURNING *
            "#,
        )
        .bind(&name)
        .bind(&record_type)
        .bind(&value)
        .bind(ttl)
        .bind(priority)
        .bind(enabled)
        .bind(Utc::now())
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(result)
    }

    /// Delete a DNS record
    pub async fn delete(&self, id: i64) -> Result<bool> {
        let result = sqlx::query("DELETE FROM dns_records WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }
}

/// Repository for rewrite rules
pub struct RewriteRuleRepository {
    pool: SqlitePool,
}

impl RewriteRuleRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Create a new rewrite rule
    pub async fn create(&self, rule: CreateRewriteRule) -> Result<RewriteRule> {
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query_as::<_, RewriteRule>(
            r#"
            INSERT INTO rewrite_rules (pattern, match_type, action_type, action_value, priority, enabled, description, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            RETURNING *
            "#,
        )
        .bind(&rule.pattern)
        .bind(&rule.match_type)
        .bind(&rule.action_type)
        .bind(&rule.action_value)
        .bind(rule.priority)
        .bind(rule.enabled)
        .bind(&rule.description)
        .bind(now)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(result)
    }

    /// Get a rewrite rule by ID
    pub async fn get_by_id(&self, id: i64) -> Result<Option<RewriteRule>> {
        let result = sqlx::query_as::<_, RewriteRule>("SELECT * FROM rewrite_rules WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(result)
    }

    /// List all rewrite rules ordered by priority.
    #[allow(dead_code)]
    pub async fn list(&self) -> Result<Vec<RewriteRule>> {
        let result = sqlx::query_as::<_, RewriteRule>(
            "SELECT * FROM rewrite_rules ORDER BY priority DESC, id ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(result)
    }

    /// List one management-page slice and aggregate counts without loading all rows.
    pub async fn list_paged(
        &self,
        page: i64,
        page_size: i64,
    ) -> Result<(Vec<RewriteRule>, RewriteRuleStats)> {
        let offset = (page - 1) * page_size;
        let rules = sqlx::query_as::<_, RewriteRule>(
            "SELECT * FROM rewrite_rules ORDER BY priority DESC, id ASC LIMIT ? OFFSET ?",
        )
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let stats = sqlx::query_as::<_, RewriteRuleStats>(
            r#"
            SELECT
                COUNT(*) AS total,
                COALESCE(SUM(CASE WHEN enabled = TRUE THEN 1 ELSE 0 END), 0) AS enabled,
                COALESCE(SUM(CASE WHEN action_type = 'block' THEN 1 ELSE 0 END), 0) AS blocked,
                COALESCE(SUM(CASE WHEN action_type != 'block' THEN 1 ELSE 0 END), 0) AS mapped
            FROM rewrite_rules
            "#,
        )
        .fetch_one(&self.pool)
        .await?;

        Ok((rules, stats))
    }

    /// List enabled rewrite rules ordered by priority and stable ID order.
    pub async fn list_enabled(&self) -> Result<Vec<RewriteRule>> {
        let result = sqlx::query_as::<_, RewriteRule>(
            "SELECT * FROM rewrite_rules WHERE enabled = TRUE ORDER BY priority DESC, id ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(result)
    }

    /// Update a rewrite rule
    pub async fn update(&self, id: i64, update: UpdateRewriteRule) -> Result<Option<RewriteRule>> {
        let existing = self.get_by_id(id).await?;
        if existing.is_none() {
            return Ok(None);
        }
        let existing = existing.unwrap();

        let pattern = update.pattern.unwrap_or(existing.pattern);
        let match_type = update.match_type.unwrap_or(existing.match_type);
        let action_type = update.action_type.unwrap_or(existing.action_type);
        let action_value = update.action_value.or(existing.action_value);
        let priority = update.priority.unwrap_or(existing.priority);
        let enabled = update.enabled.unwrap_or(existing.enabled);
        let description = update.description.or(existing.description);

        let mut tx = self.pool.begin().await?;
        let result = sqlx::query_as::<_, RewriteRule>(
            r#"
            UPDATE rewrite_rules
            SET pattern = ?, match_type = ?, action_type = ?, action_value = ?, priority = ?, enabled = ?, description = ?, updated_at = ?
            WHERE id = ?
            RETURNING *
            "#,
        )
        .bind(&pattern)
        .bind(&match_type)
        .bind(&action_type)
        .bind(&action_value)
        .bind(priority)
        .bind(enabled)
        .bind(&description)
        .bind(Utc::now())
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(result)
    }

    /// Delete a rewrite rule
    pub async fn delete(&self, id: i64) -> Result<bool> {
        let result = sqlx::query("DELETE FROM rewrite_rules WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Batch create rewrite rules
    /// Returns the number of rules created
    pub async fn batch_create(&self, rules: Vec<CreateRewriteRule>) -> Result<i64> {
        if rules.is_empty() {
            return Ok(0);
        }

        let now = Utc::now();
        let mut count = 0i64;

        // Use transaction for batch insert
        let mut tx = self.pool.begin().await?;

        for rule in rules {
            sqlx::query(
                r#"
                INSERT INTO rewrite_rules (pattern, match_type, action_type, action_value, priority, enabled, description, created_at, updated_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&rule.pattern)
            .bind(&rule.match_type)
            .bind(&rule.action_type)
            .bind(&rule.action_value)
            .bind(rule.priority)
            .bind(rule.enabled)
            .bind(&rule.description)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            count += 1;
        }

        tx.commit().await?;
        Ok(count)
    }
}
pub struct UpstreamServerRepository {
    pool: SqlitePool,
}

impl UpstreamServerRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Create a new upstream server
    pub async fn create(&self, server: CreateUpstreamServer) -> Result<UpstreamServer> {
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query_as::<_, UpstreamServer>(
            r#"
            INSERT INTO upstream_servers (name, address, protocol, timeout, enabled, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            RETURNING *
            "#,
        )
        .bind(&server.name)
        .bind(&server.address)
        .bind(&server.protocol)
        .bind(server.timeout)
        .bind(server.enabled)
        .bind(now)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(result)
    }

    /// Get an upstream server by ID
    pub async fn get_by_id(&self, id: i64) -> Result<Option<UpstreamServer>> {
        let result =
            sqlx::query_as::<_, UpstreamServer>("SELECT * FROM upstream_servers WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;

        Ok(result)
    }

    /// List all upstream servers
    pub async fn list(&self) -> Result<Vec<UpstreamServer>> {
        let result =
            sqlx::query_as::<_, UpstreamServer>("SELECT * FROM upstream_servers ORDER BY id")
                .fetch_all(&self.pool)
                .await?;

        Ok(result)
    }

    /// List upstream servers with pagination
    pub async fn list_paged(
        &self,
        page: i64,
        page_size: i64,
    ) -> Result<(Vec<UpstreamServer>, i64)> {
        let offset = (page - 1) * page_size;

        let result = sqlx::query_as::<_, UpstreamServer>(
            "SELECT * FROM upstream_servers ORDER BY id LIMIT ? OFFSET ?",
        )
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM upstream_servers")
            .fetch_one(&self.pool)
            .await?;

        Ok((result, total.0))
    }

    /// List enabled upstream servers
    pub async fn list_enabled(&self) -> Result<Vec<UpstreamServer>> {
        let result = sqlx::query_as::<_, UpstreamServer>(
            "SELECT * FROM upstream_servers WHERE enabled = TRUE ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(result)
    }

    /// Update an upstream server
    pub async fn update(
        &self,
        id: i64,
        update: UpdateUpstreamServer,
    ) -> Result<Option<UpstreamServer>> {
        let existing = self.get_by_id(id).await?;
        if existing.is_none() {
            return Ok(None);
        }
        let existing = existing.unwrap();

        let name = update.name.unwrap_or(existing.name);
        let address = update.address.unwrap_or(existing.address);
        let protocol = update.protocol.unwrap_or(existing.protocol);
        let timeout = update.timeout.unwrap_or(existing.timeout);
        let enabled = update.enabled.unwrap_or(existing.enabled);

        let mut tx = self.pool.begin().await?;
        let result = sqlx::query_as::<_, UpstreamServer>(
            r#"
            UPDATE upstream_servers
            SET name = ?, address = ?, protocol = ?, timeout = ?, enabled = ?, updated_at = ?
            WHERE id = ?
            RETURNING *
            "#,
        )
        .bind(&name)
        .bind(&address)
        .bind(&protocol)
        .bind(timeout)
        .bind(enabled)
        .bind(Utc::now())
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(result)
    }

    /// Delete an upstream server
    pub async fn delete(&self, id: i64) -> Result<bool> {
        let result = sqlx::query("DELETE FROM upstream_servers WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }
}

/// Repository for query logs
pub struct QueryLogRepository {
    pool: SqlitePool,
    stats_cache: Arc<StatsCache>,
}

impl QueryLogRepository {
    pub fn new(pool: SqlitePool, stats_cache: Arc<StatsCache>) -> Self {
        Self { pool, stats_cache }
    }

    /// Insert many query logs in a single transaction.
    ///
    /// Query logging is the highest-volume write in the system and SQLite
    /// serialises writers, so committing one transaction per query made commit
    /// latency the ceiling on query throughput. Batching amortises that cost
    /// over many rows.
    ///
    /// Returns the number of rows written.
    pub async fn create_batch(&self, logs: &[CreateQueryLog]) -> Result<usize> {
        if logs.is_empty() {
            return Ok(0);
        }

        let now = Utc::now();
        let mut tx = self.pool.begin().await?;

        for log in logs {
            sqlx::query(
                r#"
                INSERT INTO query_logs (client_ip, query_name, query_type, response_code, response_time, cache_hit, upstream_used, created_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&log.client_ip)
            .bind(&log.query_name)
            .bind(&log.query_type)
            .bind(&log.response_code)
            .bind(log.response_time)
            .bind(log.cache_hit)
            .bind(&log.upstream_used)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;

        for log in logs {
            self.stats_cache.record_query(log.cache_hit).await;
        }

        Ok(logs.len())
    }

    /// Get a query log by ID
    #[allow(dead_code)]
    pub async fn get_by_id(&self, id: i64) -> Result<Option<QueryLog>> {
        let result = sqlx::query_as::<_, QueryLog>("SELECT * FROM query_logs WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(result)
    }

    /// List query logs with pagination and filtering
    pub async fn list(&self, filter: QueryLogFilter) -> Result<PaginatedResult<QueryLog>> {
        let limit = filter.limit.unwrap_or(50).min(1000);
        let offset = filter.offset.unwrap_or(0);

        let mut query_builder = sqlx::QueryBuilder::new("SELECT * FROM query_logs WHERE 1=1");
        let mut count_builder =
            sqlx::QueryBuilder::new("SELECT COUNT(*) FROM query_logs WHERE 1=1");

        if let Some(ref name) = filter.query_name {
            let pattern = format!("%{}%", name);
            query_builder.push(" AND query_name LIKE ");
            query_builder.push_bind(pattern.clone());
            count_builder.push(" AND query_name LIKE ");
            count_builder.push_bind(pattern);
        }

        if let Some(ref qtype) = filter.query_type {
            query_builder.push(" AND query_type = ");
            query_builder.push_bind(qtype.clone());
            count_builder.push(" AND query_type = ");
            count_builder.push_bind(qtype);
        }

        if let Some(ref ip) = filter.client_ip {
            let pattern = format!("%{}%", ip);
            query_builder.push(" AND client_ip LIKE ");
            query_builder.push_bind(pattern.clone());
            count_builder.push(" AND client_ip LIKE ");
            count_builder.push_bind(pattern);
        }

        if let Some(cache_hit) = filter.cache_hit {
            query_builder.push(" AND cache_hit = ");
            query_builder.push_bind(cache_hit);
            count_builder.push(" AND cache_hit = ");
            count_builder.push_bind(cache_hit);
        }

        if let Some(ref start) = filter.start_time {
            query_builder.push(" AND created_at >= ");
            query_builder.push_bind(start);
            count_builder.push(" AND created_at >= ");
            count_builder.push_bind(start);
        }

        if let Some(ref end) = filter.end_time {
            query_builder.push(" AND created_at <= ");
            query_builder.push_bind(end);
            count_builder.push(" AND created_at <= ");
            count_builder.push_bind(end);
        }

        // Get count first
        let count_query = count_builder.build_query_as::<(i64,)>();
        let count = count_query.fetch_one(&self.pool).await?.0;

        // Add sorting and pagination
        query_builder.push(" ORDER BY created_at DESC LIMIT ");
        query_builder.push_bind(limit);
        query_builder.push(" OFFSET ");
        query_builder.push_bind(offset);

        let query = query_builder.build_query_as::<QueryLog>();
        let items = query.fetch_all(&self.pool).await?;

        Ok(PaginatedResult {
            items,
            total: count,
            limit,
            offset,
        })
    }

    /// Delete query logs before a specific date
    pub async fn delete_before_date(&self, before_date: &str) -> Result<u64> {
        let result = sqlx::query("DELETE FROM query_logs WHERE created_at < ?")
            .bind(before_date)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    /// Delete all query logs
    pub async fn delete_all(&self) -> Result<u64> {
        let result = sqlx::query("DELETE FROM query_logs")
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    /// Get the oldest log date
    pub async fn get_oldest_date(&self) -> Result<Option<String>> {
        let result: Option<(String,)> = sqlx::query_as("SELECT MIN(created_at) FROM query_logs")
            .fetch_optional(&self.pool)
            .await?;

        Ok(result.and_then(|r| if r.0.is_empty() { None } else { Some(r.0) }))
    }

    /// Get query statistics (fast, from memory)
    pub async fn get_stats(&self) -> Result<QueryStats> {
        let stats = self.stats_cache.get_stats().await;
        Ok(QueryStats::new(
            stats.total_queries,
            stats.cache_hits,
            stats.queries_today,
        ))
    }

    /// Get query statistics (slow, from DB)
    pub async fn get_stats_db(&self) -> Result<QueryStats> {
        let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM query_logs")
            .fetch_one(&self.pool)
            .await?;

        let cache_hits: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM query_logs WHERE cache_hit = TRUE")
                .fetch_one(&self.pool)
                .await?;

        let today: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM query_logs WHERE created_at >= date('now')")
                .fetch_one(&self.pool)
                .await?;

        Ok(QueryStats::new(total.0, cache_hits.0, today.0))
    }
}

/// Query statistics
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QueryStats {
    pub total_queries: i64,
    pub cache_hits: i64,
    pub queries_today: i64,
    pub cache_hit_rate: f64,
}

impl QueryStats {
    pub fn new(total_queries: i64, cache_hits: i64, queries_today: i64) -> Self {
        let cache_hit_rate = if total_queries <= 0 {
            0.0
        } else {
            cache_hits as f64 / total_queries as f64
        };
        Self {
            total_queries,
            cache_hits,
            queries_today,
            cache_hit_rate,
        }
    }
}

/// Repository for system configuration
pub struct SystemConfigRepository {
    pool: SqlitePool,
}

impl SystemConfigRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Get a config value by key
    pub async fn get(&self, key: &str) -> Result<Option<String>> {
        let result: Option<(String,)> =
            sqlx::query_as("SELECT value FROM system_config WHERE key = ?")
                .bind(key)
                .fetch_optional(&self.pool)
                .await?;

        Ok(result.map(|r| r.0))
    }

    /// Set a config value (insert or update)
    pub async fn set(&self, key: &str, value: &str) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO system_config (key, value, updated_at)
            VALUES (?, ?, CURRENT_TIMESTAMP)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Delete a config value
    #[allow(dead_code)]
    pub async fn delete(&self, key: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM system_config WHERE key = ?")
            .bind(key)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// List all config values
    #[allow(dead_code)]
    pub async fn list(&self) -> Result<Vec<SystemConfig>> {
        let result = sqlx::query_as::<_, SystemConfig>("SELECT * FROM system_config ORDER BY key")
            .fetch_all(&self.pool)
            .await?;

        Ok(result)
    }
}

#[cfg(test)]
mod query_stats_tests {
    use super::QueryStats;

    #[test]
    fn zero_queries_have_zero_hit_rate() {
        assert_eq!(QueryStats::new(0, 0, 0).cache_hit_rate, 0.0);
    }

    #[test]
    fn hit_rate_uses_total_query_count() {
        assert!((QueryStats::new(10, 4, 2).cache_hit_rate - 0.4).abs() < f64::EPSILON);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::repository::Database;
    use tempfile::tempdir;

    /// Returns the guard alongside the database so the temp directory outlives
    /// the connection; dropping it early would delete the database file.
    async fn setup_test_db() -> (Database, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db_url = format!("sqlite:{}?mode=rwc", db_path.display());
        let db = Database::new(&db_url).await.unwrap();
        (db, dir)
    }

    #[tokio::test]
    async fn test_dns_record_crud() {
        let (db, _dir) = setup_test_db().await;
        let repo = db.dns_records();

        // Create
        let record = repo
            .create(CreateDnsRecord {
                name: "example.com".to_string(),
                record_type: "A".to_string(),
                value: "192.168.1.1".to_string(),
                ttl: 300,
                priority: 0,
                enabled: true,
            })
            .await
            .unwrap();

        assert_eq!(record.name, "example.com");
        assert_eq!(record.record_type, "A");

        // Read
        let fetched = repo.get_by_id(record.id).await.unwrap().unwrap();
        assert_eq!(fetched.value, "192.168.1.1");

        // Update
        let updated = repo
            .update(
                record.id,
                UpdateDnsRecord {
                    value: Some("192.168.1.2".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.value, "192.168.1.2");

        // Delete
        let deleted = repo.delete(record.id).await.unwrap();
        assert!(deleted);

        let not_found = repo.get_by_id(record.id).await.unwrap();
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_rewrite_rule_crud() {
        let (db, _dir) = setup_test_db().await;
        let repo = db.rewrite_rules();

        // Create
        let rule = repo
            .create(CreateRewriteRule {
                pattern: "*.ads.example.com".to_string(),
                match_type: "wildcard".to_string(),
                action_type: "block".to_string(),
                action_value: None,
                priority: 10,
                enabled: true,
                description: Some("Block ads".to_string()),
            })
            .await
            .unwrap();

        assert_eq!(rule.pattern, "*.ads.example.com");
        assert_eq!(rule.match_type, "wildcard");

        // Read
        let fetched = repo.get_by_id(rule.id).await.unwrap().unwrap();
        assert_eq!(fetched.action_type, "block");

        // Update
        let updated = repo
            .update(
                rule.id,
                UpdateRewriteRule {
                    priority: Some(20),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.priority, 20);

        // Delete
        let deleted = repo.delete(rule.id).await.unwrap();
        assert!(deleted);
    }

    #[tokio::test]
    async fn test_upstream_server_crud() {
        let (db, _dir) = setup_test_db().await;
        let repo = db.upstream_servers();

        // Create
        let server = repo
            .create(CreateUpstreamServer {
                name: "Cloudflare".to_string(),
                address: "1.1.1.1:53".to_string(),
                protocol: "udp".to_string(),
                timeout: 5000,
                enabled: true,
            })
            .await
            .unwrap();

        assert_eq!(server.name, "Cloudflare");

        // Read
        let fetched = repo.get_by_id(server.id).await.unwrap().unwrap();
        assert_eq!(fetched.address, "1.1.1.1:53");

        // Update
        let updated = repo
            .update(
                server.id,
                UpdateUpstreamServer {
                    timeout: Some(3000),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.timeout, 3000);

        // Delete
        let deleted = repo.delete(server.id).await.unwrap();
        assert!(deleted);
    }

    #[tokio::test]
    async fn test_query_log_crud() {
        let (db, _dir) = setup_test_db().await;
        let repo = db.query_logs();

        // Create
        let written = repo
            .create_batch(&[CreateQueryLog {
                client_ip: "192.168.1.100".to_string(),
                query_name: "example.com".to_string(),
                query_type: "A".to_string(),
                response_code: Some("NOERROR".to_string()),
                response_time: Some(50),
                cache_hit: false,
                upstream_used: Some("Cloudflare".to_string()),
            }])
            .await
            .unwrap();

        assert_eq!(written, 1);

        // Read
        let listed = repo.list(QueryLogFilter::default()).await.unwrap();
        let log = listed.items.first().expect("inserted log should be listed");
        assert_eq!(log.query_name, "example.com");

        let fetched = repo.get_by_id(log.id).await.unwrap().unwrap();
        assert_eq!(fetched.client_ip, "192.168.1.100");

        // List with filter
        let result = repo
            .list(QueryLogFilter {
                query_name: Some("example".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(result.items.len(), 1);
    }

    #[tokio::test]
    async fn test_system_config_crud() {
        let (db, _dir) = setup_test_db().await;
        let repo = db.system_config();

        // Set
        repo.set("query_strategy", "concurrent").await.unwrap();

        // Get
        let value = repo.get("query_strategy").await.unwrap().unwrap();
        assert_eq!(value, "concurrent");

        // Update
        repo.set("query_strategy", "round_robin").await.unwrap();
        let updated = repo.get("query_strategy").await.unwrap().unwrap();
        assert_eq!(updated, "round_robin");

        // Delete
        let deleted = repo.delete("query_strategy").await.unwrap();
        assert!(deleted);

        let not_found = repo.get("query_strategy").await.unwrap();
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_stats_cache() {
        let (db, _dir) = setup_test_db().await;
        let repo = db.query_logs();

        // Initial stats should be empty
        let initial_stats = repo.get_stats().await.unwrap();
        assert_eq!(initial_stats.total_queries, 0);

        // Create a log
        repo.create_batch(&[CreateQueryLog {
            client_ip: "127.0.0.1".to_string(),
            query_name: "test.com".to_string(),
            query_type: "A".to_string(),
            response_code: Some("NOERROR".to_string()),
            response_time: Some(10),
            cache_hit: true,
            upstream_used: Some("test".to_string()),
        }])
        .await
        .unwrap();

        // Stats should update immediately (from cache)
        let stats = repo.get_stats().await.unwrap();
        assert_eq!(stats.total_queries, 1);
        assert_eq!(stats.cache_hits, 1);
        assert_eq!(stats.queries_today, 1);

        // Create another log (no cache hit)
        repo.create_batch(&[CreateQueryLog {
            client_ip: "127.0.0.1".to_string(),
            query_name: "test2.com".to_string(),
            query_type: "A".to_string(),
            response_code: Some("NOERROR".to_string()),
            response_time: Some(20),
            cache_hit: false,
            upstream_used: Some("test".to_string()),
        }])
        .await
        .unwrap();

        let stats = repo.get_stats().await.unwrap();
        assert_eq!(stats.total_queries, 2);
        assert_eq!(stats.cache_hits, 1);
        assert_eq!(stats.queries_today, 2);

        // Verify DB stats matches
        let db_stats = repo.get_stats_db().await.unwrap();
        assert_eq!(db_stats.total_queries, 2);
        assert_eq!(db_stats.cache_hits, 1);
        assert_eq!(db_stats.queries_today, 2);
    }
}

/// Server listener repository
pub struct ServerListenerRepository {
    pool: SqlitePool,
}

impl ServerListenerRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Get all server listeners
    pub async fn list(&self) -> Result<Vec<ServerListener>> {
        let listeners =
            sqlx::query_as::<_, ServerListener>("SELECT * FROM server_listeners ORDER BY protocol")
                .fetch_all(&self.pool)
                .await?;
        Ok(listeners)
    }

    /// Get server listener by protocol
    pub async fn get_by_protocol(&self, protocol: &str) -> Result<Option<ServerListener>> {
        let listener = sqlx::query_as::<_, ServerListener>(
            "SELECT * FROM server_listeners WHERE protocol = ?",
        )
        .bind(protocol)
        .fetch_optional(&self.pool)
        .await?;
        Ok(listener)
    }

    /// Update server listener
    pub async fn update(
        &self,
        protocol: &str,
        update: UpdateServerListener,
    ) -> Result<Option<ServerListener>> {
        let existing = self.get_by_protocol(protocol).await?;
        if existing.is_none() {
            return Ok(None);
        }
        let existing = existing.unwrap();

        let enabled = update.enabled.unwrap_or(existing.enabled);
        let bind_address = update.bind_address.unwrap_or(existing.bind_address);
        let port = update.port.unwrap_or(existing.port);
        let tls_cert = match update.tls_cert {
            Some(s) if s.is_empty() => None,
            Some(s) => Some(s),
            None => existing.tls_cert,
        };
        let tls_key = match update.tls_key {
            Some(s) if s.is_empty() => None,
            Some(s) => Some(s),
            None => existing.tls_key,
        };

        let mut tx = self.pool.begin().await?;
        let result = sqlx::query_as::<_, ServerListener>(
            r#"
            UPDATE server_listeners
            SET enabled = ?, bind_address = ?, port = ?, tls_cert = ?, tls_key = ?, updated_at = CURRENT_TIMESTAMP
            WHERE protocol = ?
            RETURNING *
            "#
        )
        .bind(enabled)
        .bind(bind_address)
        .bind(port)
        .bind(tls_cert)
        .bind(tls_key)
        .bind(protocol)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(result)
    }

    /// Get enabled listeners
    pub async fn list_enabled(&self) -> Result<Vec<ServerListener>> {
        let listeners = sqlx::query_as::<_, ServerListener>(
            "SELECT * FROM server_listeners WHERE enabled = TRUE ORDER BY protocol",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(listeners)
    }
}
