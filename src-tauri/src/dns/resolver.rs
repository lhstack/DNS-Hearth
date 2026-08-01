//! DNS Resolver
//!
//! The DNS Resolver integrates the rewrite engine, cache, and proxy manager
//! to provide a complete DNS resolution pipeline.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use tracing::debug;

use super::cache::{CacheKey, CacheManager};
use super::message::{DnsQuery, DnsRecordData, DnsResponse, DnsResponseCode, RecordType};
use super::plane_state::DataPlaneState;
use super::proxy::ProxyManager;
use super::rewrite::{RewriteAction, RewriteEngine};
use crate::infrastructure::repository::{CreateQueryLog, Database, QueryLogWriter};

/// Formats answer values only when the log record is actually emitted.
///
/// Building a `Vec<String>` of cloned answer values up front cost an allocation
/// per answer on every query, including when the `debug` level was disabled and
/// the message was discarded.
struct JoinedAnswers<'a>(&'a DnsResponse);

impl std::fmt::Display for JoinedAnswers<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (index, answer) in self.0.answers.iter().enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }
            f.write_str(&answer.value)?;
        }
        Ok(())
    }
}

/// Formats the answer values, or the response code when there are none.
struct AnswersOrCode<'a>(&'a DnsResponse);

impl std::fmt::Display for AnswersOrCode<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.answers.is_empty() {
            return write!(f, "{}", self.0.response_code);
        }
        write!(f, "{}", JoinedAnswers(self.0))
    }
}

/// Query metadata returned alongside the DNS response
#[derive(Debug, Clone)]
pub struct QueryMetadata {
    /// Total response time in milliseconds
    pub response_time_ms: u64,
    /// Whether the response was served from cache
    pub cache_hit: bool,
    /// Name of the upstream server used (if any)
    pub upstream_used: Option<String>,
    /// Whether a rewrite rule was applied
    pub rewrite_applied: bool,
    /// The rewrite rule ID that was applied (if any)
    pub rewrite_rule_id: Option<i64>,
}

impl Default for QueryMetadata {
    fn default() -> Self {
        Self {
            response_time_ms: 0,
            cache_hit: false,
            upstream_used: None,
            rewrite_applied: false,
            rewrite_rule_id: None,
        }
    }
}

/// Result of a DNS resolution
#[derive(Debug, Clone)]
pub struct ResolveResult {
    /// The DNS response
    pub response: DnsResponse,
    /// Query metadata
    pub metadata: QueryMetadata,
}

/// DNS Resolver
///
/// Integrates rewrite engine, cache, and proxy manager to provide
/// complete DNS resolution functionality.
pub struct DnsResolver {
    /// Rewrite engine for domain rewriting
    rewrite_engine: Arc<RewriteEngine>,
    /// Cache manager for caching responses
    cache: Arc<CacheManager>,
    /// Proxy manager for upstream queries
    proxy: Arc<ProxyManager>,
    /// Database for query logging (optional)
    db: Option<Arc<Database>>,
    /// Batching writer for query logs (present whenever `db` is)
    log_writer: Option<Arc<QueryLogWriter>>,
    /// Query-path state that avoids per-query database round trips
    plane_state: Arc<DataPlaneState>,
}

#[allow(dead_code)]
impl DnsResolver {
    /// Create a new DNS resolver
    pub fn new(
        rewrite_engine: Arc<RewriteEngine>,
        cache: Arc<CacheManager>,
        proxy: Arc<ProxyManager>,
    ) -> Self {
        Self {
            rewrite_engine,
            cache,
            proxy,
            db: None,
            plane_state: Arc::new(DataPlaneState::new()),
            log_writer: None,
        }
    }

    /// Create a new DNS resolver with database for logging
    pub fn with_db(
        rewrite_engine: Arc<RewriteEngine>,
        cache: Arc<CacheManager>,
        proxy: Arc<ProxyManager>,
        db: Arc<Database>,
        log_writer: Arc<QueryLogWriter>,
    ) -> Self {
        Self {
            rewrite_engine,
            cache,
            proxy,
            db: Some(db),
            plane_state: Arc::new(DataPlaneState::new()),
            log_writer: Some(log_writer),
        }
    }

    /// Create a new DNS resolver wrapped in Arc
    pub fn new_shared(
        rewrite_engine: Arc<RewriteEngine>,
        cache: Arc<CacheManager>,
        proxy: Arc<ProxyManager>,
    ) -> Arc<Self> {
        Arc::new(Self::new(rewrite_engine, cache, proxy))
    }

    /// Get the rewrite engine
    pub fn rewrite_engine(&self) -> &Arc<RewriteEngine> {
        &self.rewrite_engine
    }

    /// Get the cache manager
    pub fn cache(&self) -> &Arc<CacheManager> {
        &self.cache
    }

    /// Get the proxy manager
    pub fn proxy(&self) -> &Arc<ProxyManager> {
        &self.proxy
    }

    /// Get the query-path state, so owners of a mutation can refresh it.
    pub fn plane_state(&self) -> &Arc<DataPlaneState> {
        &self.plane_state
    }

    /// Resolve a DNS query
    ///
    /// This is the main entry point for DNS resolution. It follows this flow:
    /// 1. Validate domain name (reject invalid domains)
    /// 2. Check if record type is disabled
    /// 3. Check rewrite rules
    /// 4. If rewrite matches, apply the action
    /// 5. Check local DNS records from database
    /// 6. Otherwise, check cache
    /// 7. If cache miss, query upstream via proxy
    /// 8. Cache the response
    pub async fn resolve(&self, query: &DnsQuery) -> Result<ResolveResult> {
        let start = Instant::now();
        let mut metadata = QueryMetadata::default();

        // Step 0: Validate domain name - reject invalid domains
        if !Self::is_valid_domain(&query.name) {
            debug!(
                "[DNS Result] {} {} | Invalid domain (rejected) | {}ms",
                query.name,
                query.record_type,
                start.elapsed().as_millis()
            );
            metadata.response_time_ms = start.elapsed().as_millis() as u64;
            return Ok(ResolveResult {
                response: DnsResponse::refused(query.id),
                metadata,
            });
        }

        debug!(
            "[DNS Query] {} {} (ID: {})",
            query.name, query.record_type, query.id
        );

        // Step 1: Check if record type is disabled
        {
            if self.plane_state.is_type_disabled(query.record_type) {
                debug!(
                    "[DNS Result] {} {} | Disabled record type | {}ms",
                    query.name,
                    query.record_type,
                    start.elapsed().as_millis()
                );
                metadata.response_time_ms = start.elapsed().as_millis() as u64;
                return Ok(ResolveResult {
                    response: DnsResponse::nxdomain(query.id),
                    metadata,
                });
            }
        }

        // Step 2: Check rewrite rules
        if let Some(rewrite_result) = self.rewrite_engine.check(&query.name).await {
            metadata.rewrite_applied = true;
            metadata.rewrite_rule_id = Some(rewrite_result.rule_id);

            let response = self
                .apply_rewrite_action(query, &rewrite_result.action)
                .await?;
            metadata.response_time_ms = start.elapsed().as_millis() as u64;

            let action_desc = match &rewrite_result.action {
                RewriteAction::Block => "BLOCKED".to_string(),
                RewriteAction::MapToIp(ip) => format!("-> {}", ip),
                RewriteAction::MapToDomain(domain) => format!("-> {}", domain),
            };
            debug!(
                "[DNS Result] {} {} | Rewrite(rule_id={}) {} | {}ms",
                query.name,
                query.record_type,
                rewrite_result.rule_id,
                action_desc,
                metadata.response_time_ms
            );

            return Ok(ResolveResult { response, metadata });
        }

        // Step 2: Check local DNS records from database.
        //
        // The mask check keeps this off the hot path: with no local records of
        // this type the query cannot match one, so the lookup is skipped rather
        // than issuing a SELECT ahead of the cache.
        if let Some(ref db) = self.db {
            if self.plane_state.may_have_local_record(query.record_type) {
                if let Some(response) = self.check_local_records(db, query).await? {
                    metadata.response_time_ms = start.elapsed().as_millis() as u64;
                    debug!(
                        "[DNS Result] {} {} | LocalRecord | {} | {}ms",
                        query.name,
                        query.record_type,
                        JoinedAnswers(&response),
                        metadata.response_time_ms
                    );
                    return Ok(ResolveResult { response, metadata });
                }
            }
        }

        // Step 3: Check cache
        let cache_key = CacheKey::from_query(query);
        if let Some(cached_response) = self.cache.get(&cache_key).await {
            metadata.cache_hit = true;
            metadata.response_time_ms = start.elapsed().as_millis() as u64;

            // Update response ID to match query
            let mut response = cached_response;
            response.id = query.id;

            debug!(
                "[DNS Result] {} {} | Cache | {} | {}ms",
                query.name,
                query.record_type,
                JoinedAnswers(&response),
                metadata.response_time_ms
            );

            return Ok(ResolveResult { response, metadata });
        }

        debug!("Cache miss for {} {}", query.name, query.record_type);

        // Step 4: Query upstream via proxy
        let query_result = self.proxy.query(query).await?;

        metadata.upstream_used = Some(query_result.server_name.clone());
        metadata.response_time_ms = start.elapsed().as_millis() as u64;

        // Restore original query ID in response (important for DoQ which uses ID=0)
        let mut response = query_result.response;
        response.id = query.id;

        // Step 5: Cache the response (only if successful)
        if response.response_code == DnsResponseCode::NoError {
            self.cache.set(cache_key, response.clone()).await;
        }

        debug!(
            "[DNS Result] {} {} | Upstream({}) | {} | {}ms",
            query.name,
            query.record_type,
            query_result.server_name,
            AnswersOrCode(&response),
            metadata.response_time_ms
        );

        Ok(ResolveResult { response, metadata })
    }

    /// Check if domain name is valid according to DNS standards
    ///
    /// Valid domain names must:
    /// - Only contain ASCII letters (a-z, A-Z), digits (0-9), hyphens (-), dots (.), and underscores (_)
    /// - Not contain protocol prefixes (://)
    /// - Not contain special characters or Unicode (browsers convert IDN to Punycode)
    /// - Be between 1-253 characters total
    /// - Have labels (parts between dots) of 1-63 characters each
    fn is_valid_domain(name: &str) -> bool {
        // Check length
        if name.is_empty() || name.len() > 253 {
            return false;
        }

        // Check for protocol prefixes (http://, https://, etc.)
        if name.contains("://") {
            return false;
        }

        // Check for escaped slashes (common in malformed queries)
        if name.contains("\\/") || name.contains("\\\\") {
            return false;
        }

        // Check for other invalid characters
        // Valid DNS characters: a-z, A-Z, 0-9, hyphen (-), dot (.), underscore (_)
        // Note: Underscore is technically not standard but widely used (e.g., _dmarc, _spf)
        for ch in name.chars() {
            match ch {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '.' | '_' => continue,
                _ => {
                    // Reject any non-ASCII or special characters
                    // This includes Chinese characters, emojis, spaces, etc.
                    // Browsers should convert IDN domains to Punycode (xn--...) before DNS query
                    return false;
                }
            }
        }

        // Check each label (part between dots)
        for label in name.split('.') {
            // Empty labels are invalid (consecutive dots or leading/trailing dot)
            // Exception: trailing dot is valid for FQDN (e.g., "example.com.")
            if label.is_empty() {
                // Allow single trailing dot (FQDN)
                if name.ends_with('.') && name.matches('.').count() == name.split('.').count() - 1 {
                    continue;
                }
                return false;
            }

            // Label length must be 1-63 characters
            if label.len() > 63 {
                return false;
            }

            // Labels cannot start or end with hyphen
            if label.starts_with('-') || label.ends_with('-') {
                return false;
            }
        }

        true
    }

    /// Resolve a DNS query with client IP for logging
    ///
    /// This method wraps resolve() and saves the query log to database.
    pub async fn resolve_with_client(
        &self,
        query: &DnsQuery,
        client_ip: &str,
    ) -> Result<ResolveResult> {
        let result = self.resolve(query).await;

        // Hand the log entry to the batching writer. Enqueueing never blocks and
        // never fails, so resolution is not affected by logging throughput.
        if let Some(ref writer) = self.log_writer {
            let log = match &result {
                Ok(r) => CreateQueryLog {
                    client_ip: client_ip.to_string(),
                    query_name: query.name.clone(),
                    query_type: query.record_type.to_string(),
                    response_code: Some(r.response.response_code.to_string()),
                    response_time: Some(r.metadata.response_time_ms as i32),
                    cache_hit: r.metadata.cache_hit,
                    upstream_used: r.metadata.upstream_used.clone(),
                },
                Err(e) => CreateQueryLog {
                    client_ip: client_ip.to_string(),
                    query_name: query.name.clone(),
                    query_type: query.record_type.to_string(),
                    response_code: Some(format!("ERROR: {}", e)),
                    response_time: None,
                    cache_hit: false,
                    upstream_used: None,
                },
            };

            writer.enqueue(log);
        }

        result
    }

    /// Check local DNS records from database
    async fn check_local_records(
        &self,
        db: &Database,
        query: &DnsQuery,
    ) -> Result<Option<DnsResponse>> {
        use std::net::{Ipv4Addr, Ipv6Addr};
        use std::str::FromStr;

        let record_type_str = query.record_type.to_string();
        let records = db
            .dns_records()
            .get_by_name_and_type_with_wildcard(&query.name, &record_type_str)
            .await?;
        if records.is_empty() {
            return Ok(None);
        }

        let mut response = DnsResponse::new(query.id);

        for record in records {
            if !record.enabled {
                continue;
            }

            // For wildcard records, use the queried name instead of the record name
            let response_name = if record.name.starts_with("*.") {
                &query.name
            } else {
                &record.name
            };

            let dns_record = match query.record_type {
                RecordType::A => {
                    if let Ok(ip) = Ipv4Addr::from_str(&record.value) {
                        Some(DnsRecordData::a(response_name, ip, record.ttl as u32))
                    } else {
                        debug!("Invalid IPv4 address in DNS record: {}", record.value);
                        None
                    }
                }
                RecordType::AAAA => {
                    if let Ok(ip) = Ipv6Addr::from_str(&record.value) {
                        Some(DnsRecordData::aaaa(response_name, ip, record.ttl as u32))
                    } else {
                        debug!("Invalid IPv6 address in DNS record: {}", record.value);
                        None
                    }
                }
                RecordType::CNAME => Some(DnsRecordData::cname(
                    response_name,
                    &record.value,
                    record.ttl as u32,
                )),
                RecordType::MX => Some(DnsRecordData::mx(
                    response_name,
                    &record.value,
                    record.priority as u16,
                    record.ttl as u32,
                )),
                RecordType::TXT => Some(DnsRecordData::txt(
                    response_name,
                    &record.value,
                    record.ttl as u32,
                )),
                RecordType::PTR => Some(DnsRecordData::ptr(
                    response_name,
                    &record.value,
                    record.ttl as u32,
                )),
                RecordType::NS => Some(DnsRecordData::ns(
                    response_name,
                    &record.value,
                    record.ttl as u32,
                )),
                _ => None,
            };

            if let Some(dns_record) = dns_record {
                response.add_answer(dns_record);
            }
        }

        if response.answers.is_empty() {
            Ok(None)
        } else {
            Ok(Some(response))
        }
    }

    /// Resolve a DNS query by name and record type
    pub async fn resolve_with_type(
        &self,
        name: &str,
        record_type: RecordType,
    ) -> Result<ResolveResult> {
        let query = DnsQuery::new(name, record_type);
        self.resolve(&query).await
    }

    /// Apply a rewrite action and generate a response
    async fn apply_rewrite_action(
        &self,
        query: &DnsQuery,
        action: &RewriteAction,
    ) -> Result<DnsResponse> {
        match action {
            RewriteAction::MapToIp(ip) => self.create_ip_response(query, *ip),
            RewriteAction::MapToDomain(target_domain) => {
                // Resolve the target domain
                let target_query = DnsQuery::new(target_domain, query.record_type);
                let result = self.resolve_without_rewrite(&target_query).await?;

                // Return response with original query ID
                let mut response = result.response;
                response.id = query.id;
                Ok(response)
            }
            RewriteAction::Block => Ok(DnsResponse::nxdomain(query.id)),
        }
    }

    /// Create a response with an IP address
    fn create_ip_response(&self, query: &DnsQuery, ip: IpAddr) -> Result<DnsResponse> {
        let mut response = DnsResponse::new(query.id);

        match (ip, query.record_type) {
            (IpAddr::V4(ipv4), RecordType::A) => {
                response.add_answer(DnsRecordData::a(&query.name, ipv4, 300));
            }
            (IpAddr::V6(ipv6), RecordType::AAAA) => {
                response.add_answer(DnsRecordData::aaaa(&query.name, ipv6, 300));
            }
            (IpAddr::V4(ipv4), RecordType::AAAA) => {
                // Requested AAAA but have IPv4 - return empty response
                debug!("Rewrite has IPv4 {} but query is for AAAA record", ipv4);
            }
            (IpAddr::V6(ipv6), RecordType::A) => {
                // Requested A but have IPv6 - return empty response
                debug!("Rewrite has IPv6 {} but query is for A record", ipv6);
            }
            _ => {
                // For other record types with IP rewrite, return empty response
                debug!(
                    "Rewrite has IP but query is for {} record",
                    query.record_type
                );
            }
        }

        Ok(response)
    }

    /// Resolve without checking rewrite rules (to avoid infinite loops)
    /// This is kept for backward compatibility but now delegates to resolve_with_depth
    async fn resolve_without_rewrite(&self, query: &DnsQuery) -> Result<ResolveResult> {
        // Start with depth 1 since we're already in a rewrite
        self.resolve_with_depth(query, 1).await
    }

    /// Resolve with depth tracking to prevent infinite loops
    /// max_depth is 10 to allow reasonable chaining while preventing infinite loops
    fn resolve_with_depth<'a>(
        &'a self,
        query: &'a DnsQuery,
        depth: u32,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ResolveResult>> + Send + 'a>>
    {
        Box::pin(async move {
            const MAX_DEPTH: u32 = 10;

            if depth > MAX_DEPTH {
                debug!(
                    "Max rewrite depth {} exceeded for {}",
                    MAX_DEPTH, query.name
                );
                return Err(anyhow::anyhow!(
                    "Max rewrite depth exceeded, possible circular reference"
                ));
            }

            let start = Instant::now();
            let mut metadata = QueryMetadata::default();

            debug!(
                "Resolving DNS query (depth {}): {} {} (ID: {})",
                depth, query.name, query.record_type, query.id
            );

            // Step 1: Check rewrite rules (allow chaining)
            if let Some(rewrite_result) = self.rewrite_engine.check(&query.name).await {
                debug!(
                    "Rewrite rule {} matched for {} (depth {})",
                    rewrite_result.rule_id, query.name, depth
                );
                metadata.rewrite_applied = true;
                metadata.rewrite_rule_id = Some(rewrite_result.rule_id);

                let response = self
                    .apply_rewrite_action_with_depth(query, &rewrite_result.action, depth)
                    .await?;
                metadata.response_time_ms = start.elapsed().as_millis() as u64;

                return Ok(ResolveResult { response, metadata });
            }

            // Step 2: Check local DNS records from database
            if let Some(ref db) = self.db {
                if self.plane_state.may_have_local_record(query.record_type) {
                    if let Some(response) = self.check_local_records(db, query).await? {
                        debug!(
                            "Local DNS record found for {} {} (depth {})",
                            query.name, query.record_type, depth
                        );
                        metadata.response_time_ms = start.elapsed().as_millis() as u64;
                        return Ok(ResolveResult { response, metadata });
                    }
                }
            }

            // Step 3: Check cache
            let cache_key = CacheKey::from_query(query);
            if let Some(cached_response) = self.cache.get(&cache_key).await {
                metadata.cache_hit = true;
                metadata.response_time_ms = start.elapsed().as_millis() as u64;

                let mut response = cached_response;
                response.id = query.id;

                return Ok(ResolveResult { response, metadata });
            }

            // Step 4: Query upstream
            let query_result = self.proxy.query(query).await?;

            metadata.upstream_used = Some(query_result.server_name);
            metadata.response_time_ms = start.elapsed().as_millis() as u64;

            // Restore original query ID in response (important for DoQ which uses ID=0)
            let mut response = query_result.response;
            response.id = query.id;

            // Cache the response
            if response.response_code == DnsResponseCode::NoError {
                self.cache.set(cache_key, response.clone()).await;
            }

            Ok(ResolveResult { response, metadata })
        })
    }

    /// Apply a rewrite action with depth tracking
    fn apply_rewrite_action_with_depth<'a>(
        &'a self,
        query: &'a DnsQuery,
        action: &'a RewriteAction,
        depth: u32,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<DnsResponse>> + Send + 'a>> {
        Box::pin(async move {
            match action {
                RewriteAction::MapToIp(ip) => self.create_ip_response(query, *ip),
                RewriteAction::MapToDomain(target_domain) => {
                    // Resolve the target domain with increased depth
                    let target_query = DnsQuery::new(target_domain, query.record_type);
                    let result = self.resolve_with_depth(&target_query, depth + 1).await?;

                    // Return response with original query ID
                    let mut response = result.response;
                    response.id = query.id;
                    Ok(response)
                }
                RewriteAction::Block => Ok(DnsResponse::nxdomain(query.id)),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::cache::CacheConfig;
    use crate::dns::proxy::UpstreamManager;
    use crate::dns::rewrite::{MatchType, RewriteRule};
    use std::net::Ipv4Addr;

    fn create_test_resolver() -> DnsResolver {
        let rewrite_engine = Arc::new(RewriteEngine::new());
        let cache = Arc::new(CacheManager::with_config(CacheConfig {
            default_ttl: 60,
            max_entries: 1000,
        }));
        let upstream_manager = Arc::new(UpstreamManager::new());
        let proxy = Arc::new(ProxyManager::new(upstream_manager));

        DnsResolver::new(rewrite_engine, cache, proxy)
    }

    #[tokio::test]
    async fn test_resolver_creation() {
        let resolver = create_test_resolver();
        assert!(resolver.db.is_none());
    }

    #[tokio::test]
    async fn test_resolver_rewrite_block() {
        let resolver = create_test_resolver();

        // Add a block rule
        resolver
            .rewrite_engine
            .add_rule(RewriteRule::new(
                1,
                "blocked.com".to_string(),
                MatchType::Exact,
                RewriteAction::Block,
                10,
            ))
            .await;

        let query = DnsQuery::new("blocked.com", RecordType::A);
        let result = resolver.resolve(&query).await.unwrap();

        assert_eq!(result.response.response_code, DnsResponseCode::NxDomain);
        assert!(result.metadata.rewrite_applied);
        assert_eq!(result.metadata.rewrite_rule_id, Some(1));
    }

    #[tokio::test]
    async fn test_resolver_rewrite_map_to_ip() {
        let resolver = create_test_resolver();

        // Add a map-to-ip rule
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        resolver
            .rewrite_engine
            .add_rule(RewriteRule::new(
                1,
                "local.test".to_string(),
                MatchType::Exact,
                RewriteAction::MapToIp(ip),
                10,
            ))
            .await;

        let query = DnsQuery::new("local.test", RecordType::A);
        let result = resolver.resolve(&query).await.unwrap();

        assert_eq!(result.response.response_code, DnsResponseCode::NoError);
        assert!(result.metadata.rewrite_applied);
        assert_eq!(result.response.answers.len(), 1);
        assert_eq!(result.response.answers[0].value, "127.0.0.1");
    }

    #[tokio::test]
    async fn test_resolver_cache_hit() {
        let resolver = create_test_resolver();

        // Pre-populate cache
        let cache_key = CacheKey::new("cached.com", RecordType::A);
        let mut response = DnsResponse::new(12345);
        response.add_answer(DnsRecordData::a(
            "cached.com",
            Ipv4Addr::new(1, 2, 3, 4),
            300,
        ));
        resolver.cache.set(cache_key, response).await;

        // Query should hit cache
        let query = DnsQuery::new("cached.com", RecordType::A);
        let result = resolver.resolve(&query).await.unwrap();

        assert!(result.metadata.cache_hit);
        assert!(!result.metadata.rewrite_applied);
        assert!(result.metadata.upstream_used.is_none());
        assert_eq!(result.response.answers.len(), 1);
    }

    #[tokio::test]
    async fn test_resolver_no_upstream_servers() {
        let resolver = create_test_resolver();

        // Query without any upstream servers should fail
        let query = DnsQuery::new("example.com", RecordType::A);
        let result = resolver.resolve(&query).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_ip_response_a_record() {
        let resolver = create_test_resolver();
        let query = DnsQuery::new("test.com", RecordType::A);
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));

        let response = resolver.create_ip_response(&query, ip).unwrap();

        assert_eq!(response.response_code, DnsResponseCode::NoError);
        assert_eq!(response.answers.len(), 1);
        assert_eq!(response.answers[0].record_type, RecordType::A);
        assert_eq!(response.answers[0].value, "192.168.1.1");
    }

    #[tokio::test]
    async fn test_create_ip_response_mismatched_type() {
        let resolver = create_test_resolver();
        let query = DnsQuery::new("test.com", RecordType::AAAA);
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));

        let response = resolver.create_ip_response(&query, ip).unwrap();

        // Should return empty response when IP type doesn't match query type
        assert_eq!(response.response_code, DnsResponseCode::NoError);
        assert_eq!(response.answers.len(), 0);
    }

    #[tokio::test]
    async fn test_query_metadata_default() {
        let metadata = QueryMetadata::default();

        assert_eq!(metadata.response_time_ms, 0);
        assert!(!metadata.cache_hit);
        assert!(metadata.upstream_used.is_none());
        assert!(!metadata.rewrite_applied);
        assert!(metadata.rewrite_rule_id.is_none());
    }

    // Domain validation tests
    #[test]
    fn test_valid_domains() {
        // Standard domains
        assert!(DnsResolver::is_valid_domain("example.com"));
        assert!(DnsResolver::is_valid_domain("sub.example.com"));
        assert!(DnsResolver::is_valid_domain("a.b.c.example.com"));
        assert!(DnsResolver::is_valid_domain("example123.com"));
        assert!(DnsResolver::is_valid_domain("123.com"));
        assert!(DnsResolver::is_valid_domain("my-domain.com"));

        // Punycode (IDN) domains - these are valid
        assert!(DnsResolver::is_valid_domain("xn--4gq62f52gdss.com")); // 一元机场.com
        assert!(DnsResolver::is_valid_domain("xn--n3h.com")); // emoji domain

        // FQDN with trailing dot
        assert!(DnsResolver::is_valid_domain("example.com."));

        // Underscore (used in DNS records like _dmarc, _spf)
        assert!(DnsResolver::is_valid_domain("_dmarc.example.com"));
        assert!(DnsResolver::is_valid_domain("_spf.example.com"));
    }

    #[test]
    fn test_invalid_domains() {
        // Protocol prefixes
        assert!(!DnsResolver::is_valid_domain("http://example.com"));
        assert!(!DnsResolver::is_valid_domain("https://example.com"));
        assert!(!DnsResolver::is_valid_domain("ftp://example.com"));

        // Chinese characters (should use Punycode)
        assert!(!DnsResolver::is_valid_domain("一元机场.com"));
        assert!(!DnsResolver::is_valid_domain("中文.com"));
        assert!(!DnsResolver::is_valid_domain("测试.example.com"));

        // Special characters
        assert!(!DnsResolver::is_valid_domain("example.com/path"));
        assert!(!DnsResolver::is_valid_domain("example.com?query"));
        assert!(!DnsResolver::is_valid_domain("example.com#anchor"));
        assert!(!DnsResolver::is_valid_domain("example.com:8080"));
        assert!(!DnsResolver::is_valid_domain("user@example.com"));
        assert!(!DnsResolver::is_valid_domain("example .com"));
        assert!(!DnsResolver::is_valid_domain("example\t.com"));

        // Escaped characters
        assert!(!DnsResolver::is_valid_domain("example\\.com"));
        assert!(!DnsResolver::is_valid_domain("example\\\\com"));

        // Empty or too long
        assert!(!DnsResolver::is_valid_domain(""));
        assert!(!DnsResolver::is_valid_domain(&"a".repeat(254)));

        // Label too long (>63 chars)
        let long_label = format!("{}.com", "a".repeat(64));
        assert!(!DnsResolver::is_valid_domain(&long_label));

        // Invalid label format
        assert!(!DnsResolver::is_valid_domain("-example.com")); // starts with hyphen
        assert!(!DnsResolver::is_valid_domain("example-.com")); // ends with hyphen
        assert!(!DnsResolver::is_valid_domain("..example.com")); // consecutive dots
        assert!(!DnsResolver::is_valid_domain(".example.com")); // leading dot

        // Emojis
        assert!(!DnsResolver::is_valid_domain("😀.com"));
        assert!(!DnsResolver::is_valid_domain("example😀.com"));
    }
}
