//! Stats Cache module
//!
//! Provides in-memory caching for query statistics to avoid expensive COUNT(*) queries.

use chrono::{Local, NaiveDate};
use std::sync::atomic::{AtomicI64, Ordering};
use tokio::sync::RwLock;

/// In-memory cache for query statistics
///
/// This cache maintains atomic counters that are updated when query logs are written,
/// avoiding the need for expensive COUNT(*) queries on every stats request.
pub struct StatsCache {
    /// Total number of queries ever recorded
    total_queries: AtomicI64,
    /// Total number of cache hits
    cache_hits: AtomicI64,
    /// Number of queries recorded today
    queries_today: AtomicI64,
    /// The date for which queries_today is valid
    current_date: RwLock<NaiveDate>,
}

impl StatsCache {
    /// Create a new stats cache with initial values
    pub fn new(total_queries: i64, cache_hits: i64, queries_today: i64) -> Self {
        Self {
            total_queries: AtomicI64::new(total_queries),
            cache_hits: AtomicI64::new(cache_hits),
            queries_today: AtomicI64::new(queries_today),
            current_date: RwLock::new(Local::now().date_naive()),
        }
    }

    /// Create an empty cache (will be initialized from database)
    pub fn empty() -> Self {
        Self::new(0, 0, 0)
    }

    /// Initialize cache from database values
    pub async fn initialize(&self, total_queries: i64, cache_hits: i64, queries_today: i64) {
        self.total_queries.store(total_queries, Ordering::SeqCst);
        self.cache_hits.store(cache_hits, Ordering::SeqCst);
        self.queries_today.store(queries_today, Ordering::SeqCst);
        *self.current_date.write().await = Local::now().date_naive();
    }

    /// Record a new query
    ///
    /// Increments total_queries and queries_today.
    /// If cache_hit is true, also increments cache_hits.
    pub async fn record_query(&self, cache_hit: bool) {
        // Check if we need to reset the daily counter
        let today = Local::now().date_naive();
        {
            let current = self.current_date.read().await;
            if *current != today {
                drop(current);
                // Date changed, need to reset queries_today
                let mut date_guard = self.current_date.write().await;
                // Double-check after acquiring write lock
                if *date_guard != today {
                    *date_guard = today;
                    self.queries_today.store(0, Ordering::SeqCst);
                }
            }
        }

        // Increment counters
        self.total_queries.fetch_add(1, Ordering::SeqCst);
        self.queries_today.fetch_add(1, Ordering::SeqCst);

        if cache_hit {
            self.cache_hits.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Get current statistics
    pub async fn get_stats(&self) -> CachedQueryStats {
        // Check if we need to reset the daily counter
        let today = Local::now().date_naive();
        {
            let current = self.current_date.read().await;
            if *current != today {
                drop(current);
                let mut date_guard = self.current_date.write().await;
                if *date_guard != today {
                    *date_guard = today;
                    self.queries_today.store(0, Ordering::SeqCst);
                }
            }
        }

        CachedQueryStats {
            total_queries: self.total_queries.load(Ordering::SeqCst),
            cache_hits: self.cache_hits.load(Ordering::SeqCst),
            queries_today: self.queries_today.load(Ordering::SeqCst),
        }
    }
}

/// Cached query statistics
#[derive(Debug, Clone, Default)]
pub struct CachedQueryStats {
    pub total_queries: i64,
    pub cache_hits: i64,
    pub queries_today: i64,
}
