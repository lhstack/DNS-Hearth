//! Database connection and schema migrations.

use anyhow::Result;

use super::crud::*;
use super::stats_cache::StatsCache;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

use std::sync::Arc;

/// Database wrapper providing connection pool and repositories
pub struct Database {
    pool: SqlitePool,
    stats_cache: Arc<StatsCache>,
}

impl Database {
    /// Create a new database connection
    pub async fn new(database_url: &str) -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;

        // Enable WAL mode for concurrent read/write
        // Without this, reads are blocked when query_logs is being written
        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&pool)
            .await?;

        // Under WAL, the default `synchronous=FULL` fsyncs on every commit, which
        // put an fsync in the path of every batch of query logs. `NORMAL` is the
        // recommended pairing for WAL: durability is preserved across process
        // crashes, and only an OS-level crash can lose the most recent commits.
        sqlx::query("PRAGMA synchronous=NORMAL")
            .execute(&pool)
            .await?;

        let stats_cache = Arc::new(StatsCache::empty());
        let db = Self { pool, stats_cache };
        db.run_migrations().await?;
        db.init_stats_cache().await?; // Initial population from DB

        Ok(db)
    }

    /// Get the connection pool
    #[allow(dead_code)]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Get DNS records repository
    pub fn dns_records(&self) -> DnsRecordRepository {
        DnsRecordRepository::new(self.pool.clone())
    }

    /// Get rewrite rules repository
    pub fn rewrite_rules(&self) -> RewriteRuleRepository {
        RewriteRuleRepository::new(self.pool.clone())
    }

    /// Get upstream servers repository
    pub fn upstream_servers(&self) -> UpstreamServerRepository {
        UpstreamServerRepository::new(self.pool.clone())
    }

    /// Get query logs repository
    pub fn query_logs(&self) -> QueryLogRepository {
        QueryLogRepository::new(self.pool.clone(), self.stats_cache.clone())
    }

    /// Get system config repository
    pub fn system_config(&self) -> SystemConfigRepository {
        SystemConfigRepository::new(self.pool.clone())
    }

    /// Get server listeners repository
    pub fn server_listeners(&self) -> ServerListenerRepository {
        ServerListenerRepository::new(self.pool.clone())
    }

    /// Force WAL checkpoint to ensure all writes are visible to readers
    pub async fn checkpoint(&self) -> Result<()> {
        sqlx::query("PRAGMA wal_checkpoint(PASSIVE)")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Run database migrations
    async fn run_migrations(&self) -> Result<()> {
        // DNS records table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS dns_records (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name VARCHAR(255) NOT NULL,
                record_type VARCHAR(10) NOT NULL,
                value TEXT NOT NULL,
                ttl INTEGER DEFAULT 300,
                priority INTEGER DEFAULT 0,
                enabled BOOLEAN DEFAULT TRUE,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(r#"CREATE INDEX IF NOT EXISTS idx_dns_records_name ON dns_records(name)"#)
            .execute(&self.pool)
            .await?;

        sqlx::query(
            r#"CREATE INDEX IF NOT EXISTS idx_dns_records_type ON dns_records(record_type)"#,
        )
        .execute(&self.pool)
        .await?;

        // Composite index for wildcard DNS queries: WHERE name IN (...) AND record_type = ? AND enabled = TRUE
        sqlx::query(
            r#"CREATE INDEX IF NOT EXISTS idx_dns_records_name_type_enabled ON dns_records(name, record_type, enabled)"#,
        )
        .execute(&self.pool)
        .await?;

        // Rewrite rules table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS rewrite_rules (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                pattern VARCHAR(255) NOT NULL,
                match_type VARCHAR(20) NOT NULL,
                action_type VARCHAR(20) NOT NULL,
                action_value TEXT,
                priority INTEGER DEFAULT 0,
                enabled BOOLEAN DEFAULT TRUE,
                description TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"CREATE INDEX IF NOT EXISTS idx_rewrite_rules_enabled ON rewrite_rules(enabled)"#,
        )
        .execute(&self.pool)
        .await?;

        // Index for rewrite rule management and engine loading order.
        sqlx::query(
            r#"CREATE INDEX IF NOT EXISTS idx_rewrite_rules_priority_id ON rewrite_rules(priority DESC, id ASC)"#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"CREATE INDEX IF NOT EXISTS idx_rewrite_rules_enabled_priority_id ON rewrite_rules(enabled, priority DESC, id ASC)"#,
        )
        .execute(&self.pool)
        .await?;

        // Upstream servers table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS upstream_servers (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name VARCHAR(100) NOT NULL,
                address VARCHAR(255) NOT NULL,
                protocol VARCHAR(10) NOT NULL,
                timeout INTEGER DEFAULT 5000,
                enabled BOOLEAN DEFAULT TRUE,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Index for upstream servers query: WHERE enabled = TRUE ORDER BY id
        sqlx::query(
            r#"CREATE INDEX IF NOT EXISTS idx_upstream_servers_enabled ON upstream_servers(enabled)"#,
        )
        .execute(&self.pool)
        .await?;

        // Query logs table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS query_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                client_ip VARCHAR(45) NOT NULL,
                query_name VARCHAR(255) NOT NULL,
                query_type VARCHAR(10) NOT NULL,
                response_code VARCHAR(20),
                response_time INTEGER,
                cache_hit BOOLEAN DEFAULT FALSE,
                upstream_used VARCHAR(100),
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"CREATE INDEX IF NOT EXISTS idx_query_logs_created_at ON query_logs(created_at)"#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"CREATE INDEX IF NOT EXISTS idx_query_logs_query_name ON query_logs(query_name)"#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"CREATE INDEX IF NOT EXISTS idx_query_logs_cache_hit ON query_logs(cache_hit)"#,
        )
        .execute(&self.pool)
        .await?;

        // Server listeners configuration table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS server_listeners (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                protocol VARCHAR(10) NOT NULL UNIQUE,
                enabled BOOLEAN NOT NULL DEFAULT FALSE,
                bind_address VARCHAR(255) NOT NULL DEFAULT '0.0.0.0',
                port INTEGER NOT NULL,
                tls_cert TEXT,
                tls_key TEXT,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Insert default server listener configurations
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO server_listeners (protocol, enabled, bind_address, port)
            VALUES
                ('udp', FALSE, '127.0.0.1', 53),
                ('doh', FALSE, '0.0.0.0', 443),
                ('dot', FALSE, '0.0.0.0', 853),
                ('doq', FALSE, '0.0.0.0', 853),
                ('doh3', FALSE, '0.0.0.0', 443)
            "#,
        )
        .execute(&self.pool)
        .await?;

        // System config table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS system_config (
                key VARCHAR(100) PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Repair the original bundled Alibaba DoH3 seed, which used an IP as
        // the TLS SNI. Alibaba's HTTP/3 endpoint requires dns.alidns.com.

        // Seed default upstream servers if none exist
        self.seed_default_upstreams().await?;

        Ok(())
    }

    /// Seed default upstream DNS servers if the table is empty
    async fn seed_default_upstreams(&self) -> Result<()> {
        // Check if any upstream servers exist
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM upstream_servers")
            .fetch_one(&self.pool)
            .await?;

        if count.0 == 0 {
            // Insert default upstream servers
            let default_servers = [
                ("阿里云H3", "https://223.6.6.6/dns-query", "doh3"),
                ("阿里云Quic", "223.5.5.5:853", "doq"),
            ];

            for (name, address, protocol) in default_servers {
                sqlx::query(
                    r#"
                    INSERT INTO upstream_servers (name, address, protocol, timeout, enabled, created_at, updated_at)
                    VALUES (?, ?, ?, 5000, TRUE, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
                    "#,
                )
                .bind(name)
                .bind(address)
                .bind(protocol)
                .execute(&self.pool)
                .await?;
            }

            tracing::info!(
                "Seeded {} default upstream DNS servers",
                default_servers.len()
            );
        }

        Ok(())
    }

    /// Initialize stats cache from database
    async fn init_stats_cache(&self) -> Result<()> {
        let repo = self.query_logs();
        // We use the slow query once at startup
        let stats = repo.get_stats_db().await?;
        self.stats_cache
            .initialize(stats.total_queries, stats.cache_hits, stats.queries_today)
            .await;
        Ok(())
    }
}
