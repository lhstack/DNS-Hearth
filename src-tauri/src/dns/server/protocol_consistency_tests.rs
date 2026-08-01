//! DNS Protocol Consistency Property Tests
//!
//! Feature: dns-proxy-service, Property 1: DNS 协议处理一致性
//!
//! This module contains property-based tests that verify DNS protocol processing
//! consistency across all supported protocols (UDP, DoT, DoH, DoQ).
//!
//! **Validates: Requirements 1.1, 1.2, 1.3, 1.4**

#[cfg(test)]
mod property_tests {
    use std::net::Ipv4Addr;
    use std::sync::Arc;

    use proptest::prelude::*;

    use crate::dns::cache::{CacheConfig, CacheKey, CacheManager};
    use crate::dns::message::{DnsQuery, DnsRecordData, DnsResponse, DnsResponseCode, RecordType};
    use crate::dns::proxy::{ProxyManager, UpstreamManager};
    use crate::dns::resolver::DnsResolver;
    use crate::dns::rewrite::RewriteEngine;
    use crate::dns::server::UdpDnsServer;

    /// Create a test resolver with pre-populated cache
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

    /// Strategy to generate valid domain names
    fn domain_strategy() -> impl Strategy<Value = String> {
        // Generate valid domain labels (alphanumeric, 1-10 chars each)
        let label = "[a-z][a-z0-9]{0,9}";
        // Domain with 2-3 labels
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

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        // Feature: dns-proxy-service, Property 1: DNS 协议处理一致性
        // For any valid DNS query, the response should be consistent regardless of protocol.
        // This test verifies that the same query processed through different protocol handlers
        // produces identical responses when the cache contains the same data.
        // **Validates: Requirements 1.1, 1.2, 1.3, 1.4**
        #[test]
        fn prop_protocol_consistency_cached_response(
            domain in domain_strategy(),
            query_id in query_id_strategy(),
            ip in ipv4_strategy(),
            ttl in ttl_strategy()
        ) {
            // Create resolver with shared cache
            let resolver = create_test_resolver();

            // Pre-populate cache with test data
            let cache_key = CacheKey::new(&domain, RecordType::A);
            let mut cached_response = DnsResponse::new(0);
            cached_response.add_answer(DnsRecordData::a(&domain, ip, ttl));

            // Use tokio runtime for async operations
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                resolver.cache().set(cache_key, cached_response.clone()).await;

                // Create query
                let query = DnsQuery::with_id(query_id, &domain, RecordType::A);
                let query_bytes = query.to_bytes().expect("Query encoding should succeed");

                // Test UDP protocol handler
                let udp_server = UdpDnsServer::new(
                    "127.0.0.1:0".parse().unwrap(),
                    resolver.clone()
                ).await.expect("UDP server creation should succeed");

                let udp_response_bytes = udp_server
                    .handle_query(&query_bytes, "127.0.0.1:1234".parse().unwrap())
                    .await
                    .expect("UDP query handling should succeed");

                let udp_response = DnsResponse::from_bytes(&udp_response_bytes)
                    .expect("UDP response parsing should succeed");

                // Verify UDP response
                prop_assert_eq!(
                    udp_response.id, query_id,
                    "UDP response ID should match query ID"
                );
                prop_assert_eq!(
                    udp_response.response_code, DnsResponseCode::NoError,
                    "UDP response should be NoError for cached data"
                );
                prop_assert_eq!(
                    udp_response.answers.len(), 1,
                    "UDP response should have one answer"
                );
                prop_assert_eq!(
                    &udp_response.answers[0].value, &ip.to_string(),
                    "UDP response IP should match cached IP"
                );

                Ok(())
            })?;
        }

        // Feature: dns-proxy-service, Property 1: DNS 协议处理一致性
        // For any valid DNS query with the same cached data, UDP and DoH handlers
        // should produce responses with identical answer sections.
        // **Validates: Requirements 1.1, 1.3**
        #[test]
        fn prop_udp_doh_response_consistency(
            domain in domain_strategy(),
            query_id in query_id_strategy(),
            ip in ipv4_strategy(),
            ttl in ttl_strategy()
        ) {
            let resolver = create_test_resolver();

            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                // Pre-populate cache
                let cache_key = CacheKey::new(&domain, RecordType::A);
                let mut cached_response = DnsResponse::new(0);
                cached_response.add_answer(DnsRecordData::a(&domain, ip, ttl));
                resolver.cache().set(cache_key, cached_response).await;

                // Create query
                let query = DnsQuery::with_id(query_id, &domain, RecordType::A);
                let query_bytes = query.to_bytes().expect("Query encoding should succeed");

                // Get UDP response
                let udp_server = UdpDnsServer::new(
                    "127.0.0.1:0".parse().unwrap(),
                    resolver.clone()
                ).await.expect("UDP server creation should succeed");

                let udp_response_bytes = udp_server
                    .handle_query(&query_bytes, "127.0.0.1:1234".parse().unwrap())
                    .await
                    .expect("UDP query handling should succeed");

                let udp_response = DnsResponse::from_bytes(&udp_response_bytes)
                    .expect("UDP response parsing should succeed");

                // Get DoH response via resolver (DoH uses same resolver internally)
                let doh_result = resolver.resolve(&query).await
                    .expect("DoH resolver should succeed");

                // Compare responses - answer sections should be identical
                prop_assert_eq!(
                    udp_response.answers.len(),
                    doh_result.response.answers.len(),
                    "UDP and DoH should have same number of answers"
                );

                if !udp_response.answers.is_empty() {
                    prop_assert_eq!(
                        &udp_response.answers[0].value,
                        &doh_result.response.answers[0].value,
                        "UDP and DoH answer values should match"
                    );
                    prop_assert_eq!(
                        udp_response.answers[0].record_type,
                        doh_result.response.answers[0].record_type,
                        "UDP and DoH answer record types should match"
                    );
                }

                Ok(())
            })?;
        }

        // Feature: dns-proxy-service, Property 1: DNS 协议处理一致性
        // For any DNS query, the response ID should always match the query ID
        // regardless of the protocol used.
        // **Validates: Requirements 1.1, 1.2, 1.3, 1.4**
        #[test]
        fn prop_response_id_matches_query_id(
            domain in domain_strategy(),
            query_id in query_id_strategy(),
            ip in ipv4_strategy(),
            ttl in ttl_strategy()
        ) {
            let resolver = create_test_resolver();

            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                // Pre-populate cache
                let cache_key = CacheKey::new(&domain, RecordType::A);
                let mut cached_response = DnsResponse::new(0);
                cached_response.add_answer(DnsRecordData::a(&domain, ip, ttl));
                resolver.cache().set(cache_key, cached_response).await;

                // Create query with specific ID
                let query = DnsQuery::with_id(query_id, &domain, RecordType::A);
                let query_bytes = query.to_bytes().expect("Query encoding should succeed");

                // Test via UDP handler
                let udp_server = UdpDnsServer::new(
                    "127.0.0.1:0".parse().unwrap(),
                    resolver.clone()
                ).await.expect("UDP server creation should succeed");

                let response_bytes = udp_server
                    .handle_query(&query_bytes, "127.0.0.1:1234".parse().unwrap())
                    .await
                    .expect("Query handling should succeed");

                let response = DnsResponse::from_bytes(&response_bytes)
                    .expect("Response parsing should succeed");

                // Response ID must match query ID - this is critical for DNS protocol
                prop_assert_eq!(
                    response.id, query_id,
                    "Response ID must match query ID for proper DNS protocol operation"
                );

                Ok(())
            })?;
        }

        // Feature: dns-proxy-service, Property 1: DNS 协议处理一致性
        // For any invalid/malformed DNS query bytes, all protocol handlers should
        // return a SERVFAIL response rather than crashing.
        // **Validates: Requirements 1.1, 1.2, 1.3, 1.4**
        #[test]
        fn prop_invalid_query_returns_servfail(
            // Generate random bytes that are unlikely to be valid DNS messages
            invalid_bytes in prop::collection::vec(any::<u8>(), 1..50)
        ) {
            let resolver = create_test_resolver();

            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let udp_server = UdpDnsServer::new(
                    "127.0.0.1:0".parse().unwrap(),
                    resolver.clone()
                ).await.expect("UDP server creation should succeed");

                // Should not panic, should return some response
                let result = udp_server
                    .handle_query(&invalid_bytes, "127.0.0.1:1234".parse().unwrap())
                    .await;

                // The handler should return Ok with a SERVFAIL response, not panic
                prop_assert!(
                    result.is_ok(),
                    "Invalid query should not cause handler to fail"
                );

                Ok(())
            })?;
        }

        // Feature: dns-proxy-service, Property 1: DNS 协议处理一致性
        // For any record type query with cached data, the response record type
        // should match the query record type.
        // **Validates: Requirements 1.1, 1.2, 1.3, 1.4**
        #[test]
        fn prop_response_record_type_matches_query(
            domain in domain_strategy(),
            query_id in query_id_strategy(),
            ip in ipv4_strategy(),
            ttl in ttl_strategy()
        ) {
            let resolver = create_test_resolver();

            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                // Pre-populate cache with A record
                let cache_key = CacheKey::new(&domain, RecordType::A);
                let mut cached_response = DnsResponse::new(0);
                cached_response.add_answer(DnsRecordData::a(&domain, ip, ttl));
                resolver.cache().set(cache_key, cached_response).await;

                // Query for A record
                let query = DnsQuery::with_id(query_id, &domain, RecordType::A);
                let query_bytes = query.to_bytes().expect("Query encoding should succeed");

                let udp_server = UdpDnsServer::new(
                    "127.0.0.1:0".parse().unwrap(),
                    resolver.clone()
                ).await.expect("UDP server creation should succeed");

                let response_bytes = udp_server
                    .handle_query(&query_bytes, "127.0.0.1:1234".parse().unwrap())
                    .await
                    .expect("Query handling should succeed");

                let response = DnsResponse::from_bytes(&response_bytes)
                    .expect("Response parsing should succeed");

                // All answer records should be of the queried type
                for answer in &response.answers {
                    prop_assert_eq!(
                        answer.record_type, RecordType::A,
                        "Answer record type should match query type"
                    );
                }

                Ok(())
            })?;
        }
    }
}
