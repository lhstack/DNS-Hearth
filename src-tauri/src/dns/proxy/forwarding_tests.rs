//! DNS Proxy Forwarding Property Tests
//!
//! Feature: dns-proxy-service, Property 3: 代理转发正确性
//!
//! This module contains property-based tests that verify DNS proxy forwarding
//! correctness. For any domain query that cannot be resolved locally, the
//! Proxy_Manager should forward the request to upstream servers and return
//! the upstream response unchanged (unless rewrite rules apply).
//!
//! **Validates: Requirements 3.1**

#[cfg(test)]
mod property_tests {
    use std::net::Ipv4Addr;
    use std::sync::Arc;

    use proptest::prelude::*;

    use crate::dns::cache::{CacheConfig, CacheKey, CacheManager};
    use crate::dns::message::{DnsQuery, DnsRecordData, DnsResponse, DnsResponseCode, RecordType};
    use crate::dns::proxy::{ProxyManager, UpstreamManager};
    use crate::dns::resolver::DnsResolver;
    use crate::dns::rewrite::{MatchType, RewriteAction, RewriteEngine, RewriteRule};

    /// Create a test resolver with empty cache and no rewrite rules
    fn create_test_resolver() -> Arc<DnsResolver> {
        let rewrite_engine = Arc::new(RewriteEngine::new());
        let cache = Arc::new(CacheManager::with_config(CacheConfig {
            default_ttl: 60,
            max_entries: 1000,
        }));
        let upstream_manager = Arc::new(UpstreamManager::new());
        let proxy = Arc::new(ProxyManager::new(upstream_manager));

        Arc::new(DnsResolver::new(rewrite_engine, cache, proxy))
    }

    /// Create a test resolver with a specific rewrite rule
    fn create_resolver_with_rewrite(rule: RewriteRule) -> Arc<DnsResolver> {
        let rewrite_engine = Arc::new(RewriteEngine::new());
        let cache = Arc::new(CacheManager::with_config(CacheConfig {
            default_ttl: 60,
            max_entries: 1000,
        }));
        let upstream_manager = Arc::new(UpstreamManager::new());
        let proxy = Arc::new(ProxyManager::new(upstream_manager));

        let resolver = Arc::new(DnsResolver::new(rewrite_engine, cache, proxy));

        // Add the rewrite rule synchronously via a runtime
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            resolver.rewrite_engine().add_rule(rule).await;
        });

        resolver
    }

    /// Strategy to generate valid domain names
    fn domain_strategy() -> impl Strategy<Value = String> {
        let label = "[a-z][a-z0-9]{0,9}";
        (label, label).prop_map(|(l1, l2)| format!("{}.{}", l1, l2))
    }

    /// Strategy to generate query IDs
    fn query_id_strategy() -> impl Strategy<Value = u16> {
        1u16..65535u16
    }

    /// Strategy to generate IPv4 addresses
    fn ipv4_strategy() -> impl Strategy<Value = Ipv4Addr> {
        (any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>())
            .prop_map(|(a, b, c, d)| Ipv4Addr::new(a, b, c, d))
    }

    /// Strategy to generate TTL values
    fn ttl_strategy() -> impl Strategy<Value = u32> {
        60u32..86400u32
    }

    /// Strategy to generate record types (subset that are commonly forwarded)
    fn record_type_strategy() -> impl Strategy<Value = RecordType> {
        prop_oneof![
            Just(RecordType::A),
            Just(RecordType::AAAA),
            Just(RecordType::CNAME),
            Just(RecordType::MX),
            Just(RecordType::TXT),
            Just(RecordType::NS),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        // Feature: dns-proxy-service, Property 3: 代理转发正确性
        // For any domain query with no cache and no rewrite rules, the resolver
        // should attempt to forward to upstream servers. Without configured upstreams,
        // this should result in an error (no healthy servers).
        // **Validates: Requirements 3.1**
        #[test]
        fn prop_uncached_query_attempts_upstream_forwarding(
            domain in domain_strategy(),
            query_id in query_id_strategy(),
            record_type in record_type_strategy()
        ) {
            let resolver = create_test_resolver();

            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                // Create query for a domain not in cache
                let query = DnsQuery::with_id(query_id, &domain, record_type);

                // Without upstream servers configured, the query should fail
                // This verifies that the resolver attempts to forward to upstream
                let result = resolver.resolve(&query).await;

                // Should fail because no upstream servers are configured
                prop_assert!(
                    result.is_err(),
                    "Query should fail when no upstream servers are available"
                );

                // Verify the error message indicates upstream issue
                let err_msg = result.unwrap_err().to_string();
                prop_assert!(
                    err_msg.contains("upstream") || err_msg.contains("server"),
                    "Error should indicate upstream server issue: {}", err_msg
                );

                Ok(())
            })?;
        }

        // Feature: dns-proxy-service, Property 3: 代理转发正确性
        // For any domain query, if a rewrite rule matches, the response should
        // come from the rewrite action, not from upstream forwarding.
        // **Validates: Requirements 3.1**
        #[test]
        fn prop_rewrite_rule_bypasses_upstream_forwarding(
            domain in domain_strategy(),
            query_id in query_id_strategy(),
            rewrite_ip in ipv4_strategy()
        ) {
            // Create resolver with a rewrite rule that maps the domain to an IP
            let rule = RewriteRule::new(
                1,
                domain.clone(),
                MatchType::Exact,
                RewriteAction::MapToIp(std::net::IpAddr::V4(rewrite_ip)),
                10,
            );
            let resolver = create_resolver_with_rewrite(rule);

            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let query = DnsQuery::with_id(query_id, &domain, RecordType::A);
                let result = resolver.resolve(&query).await;

                // Should succeed because rewrite rule provides the response
                prop_assert!(
                    result.is_ok(),
                    "Query should succeed when rewrite rule matches"
                );

                let resolve_result = result.unwrap();

                // Verify rewrite was applied
                prop_assert!(
                    resolve_result.metadata.rewrite_applied,
                    "Rewrite should be marked as applied"
                );

                // Verify no upstream was used
                prop_assert!(
                    resolve_result.metadata.upstream_used.is_none(),
                    "No upstream should be used when rewrite rule matches"
                );

                // Verify the response contains the rewrite IP
                prop_assert_eq!(
                    resolve_result.response.answers.len(), 1,
                    "Response should have one answer from rewrite"
                );
                prop_assert_eq!(
                    &resolve_result.response.answers[0].value, &rewrite_ip.to_string(),
                    "Response IP should match rewrite rule IP"
                );

                Ok(())
            })?;
        }

        // Feature: dns-proxy-service, Property 3: 代理转发正确性
        // For any domain query, if a blocking rewrite rule matches, the response
        // should be NXDOMAIN without attempting upstream forwarding.
        // **Validates: Requirements 3.1**
        #[test]
        fn prop_block_rule_returns_nxdomain_without_forwarding(
            domain in domain_strategy(),
            query_id in query_id_strategy(),
            record_type in record_type_strategy()
        ) {
            // Create resolver with a blocking rewrite rule
            let rule = RewriteRule::new(
                1,
                domain.clone(),
                MatchType::Exact,
                RewriteAction::Block,
                10,
            );
            let resolver = create_resolver_with_rewrite(rule);

            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let query = DnsQuery::with_id(query_id, &domain, record_type);
                let result = resolver.resolve(&query).await;

                // Should succeed with NXDOMAIN response
                prop_assert!(
                    result.is_ok(),
                    "Query should succeed with NXDOMAIN when block rule matches"
                );

                let resolve_result = result.unwrap();

                // Verify rewrite was applied
                prop_assert!(
                    resolve_result.metadata.rewrite_applied,
                    "Rewrite should be marked as applied"
                );

                // Verify no upstream was used
                prop_assert!(
                    resolve_result.metadata.upstream_used.is_none(),
                    "No upstream should be used when block rule matches"
                );

                // Verify NXDOMAIN response
                prop_assert_eq!(
                    resolve_result.response.response_code, DnsResponseCode::NxDomain,
                    "Response should be NXDOMAIN for blocked domain"
                );

                Ok(())
            })?;
        }

        // Feature: dns-proxy-service, Property 3: 代理转发正确性
        // For any cached domain query, the response should come from cache
        // without attempting upstream forwarding.
        // **Validates: Requirements 3.1**
        #[test]
        fn prop_cached_response_bypasses_upstream_forwarding(
            domain in domain_strategy(),
            query_id in query_id_strategy(),
            cached_ip in ipv4_strategy(),
            ttl in ttl_strategy()
        ) {
            let resolver = create_test_resolver();

            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                // Pre-populate cache
                let cache_key = CacheKey::new(&domain, RecordType::A);
                let mut cached_response = DnsResponse::new(0);
                cached_response.add_answer(DnsRecordData::a(&domain, cached_ip, ttl));
                resolver.cache().set(cache_key, cached_response).await;

                // Query for the cached domain
                let query = DnsQuery::with_id(query_id, &domain, RecordType::A);
                let result = resolver.resolve(&query).await;

                // Should succeed from cache
                prop_assert!(
                    result.is_ok(),
                    "Query should succeed when response is cached"
                );

                let resolve_result = result.unwrap();

                // Verify cache hit
                prop_assert!(
                    resolve_result.metadata.cache_hit,
                    "Response should be marked as cache hit"
                );

                // Verify no upstream was used
                prop_assert!(
                    resolve_result.metadata.upstream_used.is_none(),
                    "No upstream should be used when cache hit"
                );

                // Verify the response matches cached data
                prop_assert_eq!(
                    resolve_result.response.answers.len(), 1,
                    "Response should have one answer from cache"
                );
                prop_assert_eq!(
                    &resolve_result.response.answers[0].value, &cached_ip.to_string(),
                    "Response IP should match cached IP"
                );

                Ok(())
            })?;
        }

        // Feature: dns-proxy-service, Property 3: 代理转发正确性
        // For any domain query, the response ID should always match the query ID,
        // whether the response comes from cache, rewrite, or upstream.
        // **Validates: Requirements 3.1**
        #[test]
        fn prop_response_id_preserved_through_forwarding_path(
            domain in domain_strategy(),
            query_id in query_id_strategy(),
            cached_ip in ipv4_strategy(),
            ttl in ttl_strategy()
        ) {
            let resolver = create_test_resolver();

            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                // Pre-populate cache to ensure we get a response
                let cache_key = CacheKey::new(&domain, RecordType::A);
                let mut cached_response = DnsResponse::new(0); // Different ID
                cached_response.add_answer(DnsRecordData::a(&domain, cached_ip, ttl));
                resolver.cache().set(cache_key, cached_response).await;

                // Query with specific ID
                let query = DnsQuery::with_id(query_id, &domain, RecordType::A);
                let result = resolver.resolve(&query).await;

                prop_assert!(result.is_ok(), "Query should succeed");

                let resolve_result = result.unwrap();

                // Response ID must match query ID
                prop_assert_eq!(
                    resolve_result.response.id, query_id,
                    "Response ID must match query ID regardless of source"
                );

                Ok(())
            })?;
        }

        // Feature: dns-proxy-service, Property 3: 代理转发正确性
        // For any domain query with different record types, each type should be
        // handled independently for cache lookup and forwarding decisions.
        // **Validates: Requirements 3.1**
        #[test]
        fn prop_different_record_types_handled_independently(
            domain in domain_strategy(),
            query_id in query_id_strategy(),
            cached_ip in ipv4_strategy(),
            ttl in ttl_strategy()
        ) {
            let resolver = create_test_resolver();

            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                // Cache only A record
                let cache_key = CacheKey::new(&domain, RecordType::A);
                let mut cached_response = DnsResponse::new(0);
                cached_response.add_answer(DnsRecordData::a(&domain, cached_ip, ttl));
                resolver.cache().set(cache_key, cached_response).await;

                // Query for A record should hit cache
                let a_query = DnsQuery::with_id(query_id, &domain, RecordType::A);
                let a_result = resolver.resolve(&a_query).await;
                prop_assert!(a_result.is_ok(), "A query should succeed from cache");
                prop_assert!(
                    a_result.unwrap().metadata.cache_hit,
                    "A query should be cache hit"
                );

                // Query for AAAA record should miss cache and attempt upstream
                let aaaa_query = DnsQuery::with_id(query_id, &domain, RecordType::AAAA);
                let aaaa_result = resolver.resolve(&aaaa_query).await;

                // Should fail because no upstream servers (cache miss for AAAA)
                prop_assert!(
                    aaaa_result.is_err(),
                    "AAAA query should fail when not cached and no upstream"
                );

                Ok(())
            })?;
        }
    }
}
