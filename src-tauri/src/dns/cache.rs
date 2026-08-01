//! DNS Cache Manager
//!
//! Provides caching functionality for DNS responses with TTL-based expiration,
//! cache statistics, and cache management operations.
//!
//! Optimized with DashMap for high concurrency and approximated LRU for eviction.

use std::sync::atomic::{AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use super::message::{DnsQuery, DnsResponse, RecordType};

/// Cache key for DNS queries
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct CacheKey {
    /// Domain name (lowercase, shared string)
    pub name: Arc<str>,
    /// Record type
    pub record_type: RecordType,
}

impl CacheKey {
    /// Create a new cache key
    ///
    /// Domain names are almost always already lowercase, so the common path
    /// skips the `to_lowercase` allocation and copies straight into the `Arc`.
    /// Building the key used to allocate twice on every query, including cache
    /// lookups.
    pub fn new(name: impl AsRef<str>, record_type: RecordType) -> Self {
        let name = name.as_ref();

        let name = if name.bytes().any(|b| b.is_ascii_uppercase()) {
            Arc::from(name.to_lowercase().as_str())
        } else {
            Arc::from(name)
        };

        Self { name, record_type }
    }

    /// Create a cache key from a DNS query
    pub fn from_query(query: &DnsQuery) -> Self {
        Self::new(&query.name, query.record_type)
    }
}

/// A cached DNS response entry
#[derive(Debug)]
#[allow(dead_code)]
pub struct CacheEntry {
    /// The cached DNS response
    pub response: DnsResponse,
    /// When this entry expires
    pub expires_at: Instant,
    /// When this entry was created
    pub created_at: Instant,
    /// Last access timestamp (Unix timestamp in milliseconds) for LRU
    pub last_accessed: AtomicI64,
}

impl CacheEntry {
    /// Create a new cache entry
    pub fn new(response: DnsResponse, ttl: Duration) -> Self {
        let now = Instant::now();
        Self {
            response,
            expires_at: now + ttl,
            created_at: now,
            last_accessed: AtomicI64::new(Self::now_millis()),
        }
    }

    /// Check if this entry has expired
    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }

    /// Get the remaining TTL in seconds
    #[allow(dead_code)]
    pub fn remaining_ttl(&self) -> u64 {
        let now = Instant::now();
        if now >= self.expires_at {
            0
        } else {
            (self.expires_at - now).as_secs()
        }
    }

    /// Update last accessed time
    pub fn touch(&self) {
        self.last_accessed
            .store(Self::now_millis(), Ordering::Relaxed);
    }

    /// Get current time in milliseconds
    fn now_millis() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64
    }
}

/// Lower bound on a cached entry's lifetime.
///
/// Keeps upstreams that advertise very short TTLs from turning the cache into a
/// pass-through.
const MIN_CACHE_TTL_SECS: u64 = 5;

/// Upper bound on a cached entry's lifetime.
const MAX_CACHE_TTL_SECS: u64 = 3600;

/// Cache configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Default TTL for cached entries in seconds
    pub default_ttl: u64,
    /// Maximum number of entries in the cache
    pub max_entries: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            default_ttl: 60,
            max_entries: 10000,
        }
    }
}

/// Cache statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CacheStats {
    /// Number of cache hits
    pub hits: u64,
    /// Number of cache misses
    pub misses: u64,
    /// Current number of entries in the cache
    pub entries: usize,
    /// Cache hit rate in the inclusive range 0.0..=1.0.
    pub hit_rate: f64,
}

impl CacheStats {
    /// Calculate the cache hit rate (0.0 to 1.0)
    pub fn hit_rate(&self) -> f64 {
        self.hit_rate
    }

    fn from_counts(hits: u64, misses: u64, entries: usize) -> Self {
        let total = hits + misses;
        let hit_rate = if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        };
        Self {
            hits,
            misses,
            entries,
            hit_rate,
        }
    }
}

/// DNS Cache Manager
///
/// Thread-safe cache for DNS responses with TTL-based expiration.
/// Uses DashMap for high concurrency and approximated LRU for eviction.
pub struct CacheManager {
    /// The cache storage
    cache: DashMap<CacheKey, CacheEntry>,
    /// Cache configuration
    config: RwLock<CacheConfig>,
    /// Cache statistics - hits
    hits: AtomicU64,
    /// Cache statistics - misses
    misses: AtomicU64,
    /// Rotating start point for bounded eviction samples.
    eviction_cursor: AtomicUsize,
}

impl CacheManager {
    /// Create a new cache manager with default configuration
    pub fn new() -> Self {
        Self::with_config(CacheConfig::default())
    }

    /// Create a new cache manager with custom configuration
    pub fn with_config(config: CacheConfig) -> Self {
        Self {
            cache: DashMap::new(),
            config: RwLock::new(config),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            eviction_cursor: AtomicUsize::new(0),
        }
    }

    /// Create a new cache manager wrapped in Arc
    #[allow(dead_code)]
    pub fn new_shared() -> Arc<Self> {
        Arc::new(Self::new())
    }

    /// Get a cached response for the given key
    pub async fn get(&self, key: &CacheKey) -> Option<DnsResponse> {
        if let Some(entry) = self.cache.get(key) {
            if !entry.is_expired() {
                // Update hit count
                self.hits.fetch_add(1, Ordering::Relaxed);
                // Update access time for LRU
                entry.touch();

                return Some(entry.response.clone());
            }
        }

        // Update miss count
        self.misses.fetch_add(1, Ordering::Relaxed);

        None
    }

    /// Store a response in the cache
    ///
    /// The lifetime comes from the response itself rather than the configured
    /// default. Using a fixed default discarded the upstream's TTL in both
    /// directions: a record valid for an hour was re-queried every minute, and a
    /// record valid for 30 seconds was served for a full minute after it had
    /// expired.
    pub async fn set(&self, key: CacheKey, response: DnsResponse) {
        let config = self.config.read().await;
        let fallback_ttl = config.default_ttl;
        let max_entries = config.max_entries;
        drop(config);

        let ttl = Self::effective_ttl(&response, fallback_ttl);
        self.set_with_ttl(key, response, ttl, max_entries).await;
    }

    /// Decide how long a response may be cached.
    ///
    /// The answer set's smallest TTL is authoritative, clamped so that a hostile
    /// or misconfigured upstream can neither defeat the cache with near-zero
    /// TTLs nor pin an entry for an unreasonable length of time. Responses
    /// without answers fall back to the configured default, since there is no
    /// record TTL to read.
    fn effective_ttl(response: &DnsResponse, fallback_secs: u64) -> Duration {
        match response.min_answer_ttl() {
            Some(ttl) => {
                Duration::from_secs((ttl as u64).clamp(MIN_CACHE_TTL_SECS, MAX_CACHE_TTL_SECS))
            }
            None => Duration::from_secs(fallback_secs),
        }
    }

    /// Store a response in the cache with a specific TTL.
    pub async fn set_with_ttl(
        &self,
        key: CacheKey,
        response: DnsResponse,
        ttl: Duration,
        max_entries: usize,
    ) {
        if max_entries == 0 {
            return;
        }

        // `insert` returns the previous value, so replacement needs one map
        // lookup instead of a separate `contains_key` followed by `insert`.
        let previous = self
            .cache
            .insert(key.clone(), CacheEntry::new(response, ttl));
        if previous.is_some() {
            return;
        }

        // Concurrent insertions may briefly exceed the approximate limit. The
        // inserting task pays bounded eviction work until this entry fits.
        while self.cache.len() > max_entries {
            let before = self.cache.len();
            self.perform_eviction(max_entries, &key);
            if self.cache.len() >= before {
                break;
            }
        }
    }

    /// Evict entries until a new insertion can remain within `target_size`.
    ///
    /// Sampling rotates across DashMap shards and inspects only a bounded
    /// number of entries in each chosen shard. This avoids both a whole-map
    /// scan and the previous bias toward the iterator's first five entries.
    fn perform_eviction(&self, target_size: usize, protected_key: &CacheKey) {
        const SAMPLE_SIZE: usize = 5;
        const MAX_ATTEMPTS: usize = 5;

        if target_size == 0 {
            return;
        }

        let shards = self.cache.shards();
        for attempt in 0..MAX_ATTEMPTS {
            if self.cache.len() <= target_size {
                break;
            }

            let start = self.eviction_cursor.fetch_add(1, Ordering::Relaxed);
            let mut candidate: Option<(CacheKey, i64)> = None;
            let mut sampled = 0;

            for shard_offset in 0..shards.len() {
                let shard = &shards[(start + shard_offset + attempt) % shards.len()];
                let guard = shard.read();

                // RawTable iteration yields buckets; dereferencing a bucket is
                // safe while its shard's read guard is held.
                for bucket in unsafe { guard.iter() } {
                    let (key, value) = unsafe { bucket.as_ref() };
                    if key == protected_key {
                        continue;
                    }
                    let last_accessed = value.get().last_accessed.load(Ordering::Relaxed);
                    if candidate
                        .as_ref()
                        .is_none_or(|(_, oldest)| last_accessed < *oldest)
                    {
                        candidate = Some((key.clone(), last_accessed));
                    }
                    sampled += 1;
                    if sampled == SAMPLE_SIZE {
                        break;
                    }
                }

                if sampled == SAMPLE_SIZE {
                    break;
                }
            }

            let Some((oldest_key, _)) = candidate else {
                break;
            };
            self.cache.remove(&oldest_key);
        }
    }

    /// Clear all entries from the cache
    pub async fn clear(&self) {
        self.cache.clear();
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
    }

    /// Clear cache entries for a specific domain
    pub async fn clear_domain(&self, domain: &str) {
        let domain_lower = domain.to_lowercase();
        self.cache
            .retain(|key, _| !key.name.eq_ignore_ascii_case(&domain_lower));
    }

    /// Get current cache statistics
    pub async fn stats(&self) -> CacheStats {
        CacheStats::from_counts(
            self.hits.load(Ordering::Relaxed),
            self.misses.load(Ordering::Relaxed),
            self.cache.len(),
        )
    }

    /// Update the default TTL
    #[allow(dead_code)]
    pub async fn set_ttl(&self, ttl_seconds: u64) {
        let mut config = self.config.write().await;
        config.default_ttl = ttl_seconds;
    }

    /// Get the current default TTL
    #[allow(dead_code)]
    pub async fn get_ttl(&self) -> u64 {
        let config = self.config.read().await;
        config.default_ttl
    }

    /// Update the maximum number of entries
    #[allow(dead_code)]
    pub async fn set_max_entries(&self, max_entries: usize) {
        let mut config = self.config.write().await;
        config.max_entries = max_entries;
    }

    /// Get the current configuration
    pub async fn get_config(&self) -> CacheConfig {
        self.config.read().await.clone()
    }

    /// Update the configuration
    pub async fn update_config(&self, config: CacheConfig) {
        let mut current = self.config.write().await;
        *current = config;
    }

    /// Remove expired entries from the cache
    pub async fn cleanup_expired(&self) {
        self.cache.retain(|_, entry| !entry.is_expired());
    }
}

impl Default for CacheManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::message::{DnsRecordData, DnsResponse, RecordType};

    fn create_test_response(id: u16) -> DnsResponse {
        let mut response = DnsResponse::new(id);
        response.add_answer(DnsRecordData::a(
            "example.com",
            "93.184.216.34".parse().unwrap(),
            300,
        ));
        response
    }

    #[tokio::test]
    async fn test_cache_set_and_get() {
        let cache = CacheManager::new();
        let key = CacheKey::new("example.com", RecordType::A);
        let response = create_test_response(12345);

        cache.set(key.clone(), response.clone()).await;

        let cached = cache.get(&key).await;
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().id, 12345);
    }

    #[tokio::test]
    async fn test_cache_miss() {
        let cache = CacheManager::new();
        let key = CacheKey::new("nonexistent.com", RecordType::A);

        let cached = cache.get(&key).await;
        assert!(cached.is_none());
    }

    #[tokio::test]
    async fn test_cache_expiration() {
        let cache = CacheManager::new();
        let key = CacheKey::new("example.com", RecordType::A);

        // An expired entry must not be served. The lifetime is passed explicitly
        // because `set` now derives it from the response's own record TTLs.
        cache
            .set_with_ttl(
                key.clone(),
                create_test_response(12345),
                Duration::from_millis(1),
                100,
            )
            .await;

        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(cache.get(&key).await.is_none());
    }

    /// The answer set's smallest TTL decides the lifetime, so a response is
    /// never served past the point where any of its records has expired.
    #[test]
    fn test_effective_ttl_uses_smallest_answer_ttl() {
        let mut response = DnsResponse::new(1);
        response.add_answer(DnsRecordData::a(
            "example.com",
            "93.184.216.34".parse().unwrap(),
            600,
        ));
        response.add_answer(DnsRecordData::a(
            "example.com",
            "93.184.216.35".parse().unwrap(),
            120,
        ));

        assert_eq!(
            CacheManager::effective_ttl(&response, 60),
            Duration::from_secs(120)
        );
    }

    /// A near-zero upstream TTL would otherwise defeat the cache entirely.
    #[test]
    fn test_effective_ttl_clamps_to_bounds() {
        let mut tiny = DnsResponse::new(1);
        tiny.add_answer(DnsRecordData::a(
            "example.com",
            "93.184.216.34".parse().unwrap(),
            1,
        ));
        assert_eq!(
            CacheManager::effective_ttl(&tiny, 60),
            Duration::from_secs(MIN_CACHE_TTL_SECS)
        );

        let mut huge = DnsResponse::new(1);
        huge.add_answer(DnsRecordData::a(
            "example.com",
            "93.184.216.34".parse().unwrap(),
            86_400,
        ));
        assert_eq!(
            CacheManager::effective_ttl(&huge, 60),
            Duration::from_secs(MAX_CACHE_TTL_SECS)
        );
    }

    /// With no answers there is no record TTL to read, so the configured
    /// default applies.
    #[test]
    fn test_effective_ttl_falls_back_without_answers() {
        let empty = DnsResponse::new(1);
        assert_eq!(
            CacheManager::effective_ttl(&empty, 45),
            Duration::from_secs(45)
        );
    }

    #[test]
    fn test_cache_key_normalizes_ascii_case() {
        let lower = CacheKey::new("example.com", RecordType::A);
        let mixed = CacheKey::new("ExAmPlE.CoM", RecordType::A);

        assert_eq!(lower, mixed);
        assert_eq!(&*mixed.name, "example.com");
    }

    #[tokio::test]
    async fn test_full_cache_replaces_existing_key_without_evicting_another() {
        let cache = CacheManager::new();
        let first = CacheKey::new("first.example", RecordType::A);
        let second = CacheKey::new("second.example", RecordType::A);

        cache
            .set_with_ttl(
                first.clone(),
                create_test_response(1),
                Duration::from_secs(60),
                2,
            )
            .await;
        cache
            .set_with_ttl(
                second.clone(),
                create_test_response(2),
                Duration::from_secs(60),
                2,
            )
            .await;
        cache
            .set_with_ttl(
                first.clone(),
                create_test_response(3),
                Duration::from_secs(60),
                2,
            )
            .await;

        assert_eq!(cache.stats().await.entries, 2);
        assert_eq!(cache.get(&first).await.unwrap().id, 3);
        assert!(cache.get(&second).await.is_some());
    }

    #[tokio::test]
    async fn test_inserting_past_capacity_evicts_one_entry() {
        let cache = CacheManager::new();
        let first = CacheKey::new("first.example", RecordType::A);
        let second = CacheKey::new("second.example", RecordType::A);
        let third = CacheKey::new("third.example", RecordType::A);

        for (key, id) in [(first, 1), (second, 2), (third.clone(), 3)] {
            cache
                .set_with_ttl(key, create_test_response(id), Duration::from_secs(60), 2)
                .await;
        }

        assert_eq!(cache.stats().await.entries, 2);
        assert!(cache.get(&third).await.is_some());
    }

    #[tokio::test]
    async fn test_zero_capacity_does_not_store_entries() {
        let cache = CacheManager::new();
        let key = CacheKey::new("example.com", RecordType::A);

        cache
            .set_with_ttl(
                key.clone(),
                create_test_response(1),
                Duration::from_secs(60),
                0,
            )
            .await;

        assert!(cache.get(&key).await.is_none());
    }

    #[tokio::test]
    async fn test_cache_clear() {
        let cache = CacheManager::new();
        let key1 = CacheKey::new("example1.com", RecordType::A);
        let key2 = CacheKey::new("example2.com", RecordType::A);

        cache.set(key1.clone(), create_test_response(1)).await;
        cache.set(key2.clone(), create_test_response(2)).await;

        cache.clear().await;

        assert!(cache.get(&key1).await.is_none());
        assert!(cache.get(&key2).await.is_none());
    }

    #[tokio::test]
    async fn test_cache_stats() {
        let cache = CacheManager::new();
        let key = CacheKey::new("example.com", RecordType::A);
        let response = create_test_response(12345);

        // Miss
        cache.get(&key).await;

        // Set
        cache.set(key.clone(), response).await;

        // Hit
        cache.get(&key).await;
        cache.get(&key).await;

        let stats = cache.stats().await;
        assert_eq!(stats.hits, 2);
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.entries, 1);
        assert!((stats.hit_rate() - 0.666).abs() < 0.01);
    }
}
