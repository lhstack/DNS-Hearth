//! DNS over HTTPS (DoH) Server
//!
//! Implements a DNS server over HTTPS protocol (port 443).
//! Supports both GET and POST methods as per RFC 8484.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{ConnectInfo, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Deserialize;
use tracing::{debug, warn};

use crate::dns::message::{DnsQuery, DnsResponse};
use crate::dns::resolver::DnsResolver;

/// DoH server state
#[derive(Clone)]
pub struct DohState {
    /// DNS resolver
    pub resolver: Arc<DnsResolver>,
}

/// DNS over HTTPS Server
///
/// Provides HTTP routes for DNS queries.
pub struct DohDnsServer {
    /// DNS resolver
    resolver: Arc<DnsResolver>,
}

impl DohDnsServer {
    /// Create a new DoH DNS server
    pub fn new(resolver: Arc<DnsResolver>) -> Self {
        Self { resolver }
    }

    /// Get the Axum router for DoH endpoints
    ///
    /// This router should be mounted at `/dns-query` path.
    pub fn router(&self) -> Router {
        let state = DohState {
            resolver: self.resolver.clone(),
        };

        Router::new()
            .route("/dns-query", get(handle_get_query).post(handle_post_query))
            .with_state(state)
    }

    /// Get the resolver
    pub fn resolver(&self) -> &Arc<DnsResolver> {
        &self.resolver
    }
}

/// Query parameters for GET requests
#[derive(Debug, Deserialize)]
pub struct DohGetParams {
    /// Base64url-encoded DNS query (RFC 8484)
    pub dns: String,
}

/// Handle GET requests for DNS queries
///
/// The DNS query is passed as a base64url-encoded parameter.
async fn handle_get_query(
    State(state): State<DohState>,
    Query(params): Query<DohGetParams>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: http::Request<axum::body::Body>,
) -> Response {
    debug!("DoH GET request received");

    // Get client IP from request headers or connection
    let client_ip = get_client_ip(&request, Some(addr));

    // Decode base64url-encoded DNS query
    let query_bytes = match URL_SAFE_NO_PAD.decode(&params.dns) {
        Ok(bytes) => bytes,
        Err(e) => {
            warn!("Failed to decode base64 DNS query: {}", e);
            return (
                StatusCode::BAD_REQUEST,
                "Invalid base64url-encoded DNS query",
            )
                .into_response();
        }
    };

    process_dns_query(&state.resolver, &query_bytes, &client_ip).await
}

/// Handle POST requests for DNS queries
///
/// The DNS query is passed in the request body as application/dns-message.
async fn handle_post_query(
    State(state): State<DohState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: axum::http::Request<axum::body::Body>,
) -> Response {
    debug!("DoH POST request received");

    // Get client IP from request headers or connection
    let client_ip = get_client_ip(&request, Some(addr));

    // Extract body
    let body = match axum::body::to_bytes(request.into_body(), 65536).await {
        Ok(bytes) => bytes,
        Err(e) => {
            warn!("Failed to read request body: {}", e);
            return (StatusCode::BAD_REQUEST, "Failed to read request body").into_response();
        }
    };

    process_dns_query(&state.resolver, &body, &client_ip).await
}

/// Get client IP from request headers or connection
fn get_client_ip(
    request: &axum::http::Request<axum::body::Body>,
    conn_addr: Option<SocketAddr>,
) -> String {
    // Try X-Forwarded-For header first (for reverse proxy)
    if let Some(forwarded) = request.headers().get("x-forwarded-for") {
        if let Ok(value) = forwarded.to_str() {
            if let Some(ip) = value.split(',').next() {
                return ip.trim().to_string();
            }
        }
    }

    // Try X-Real-IP header
    if let Some(real_ip) = request.headers().get("x-real-ip") {
        if let Ok(value) = real_ip.to_str() {
            return value.to_string();
        }
    }

    // Use connection address if available
    if let Some(addr) = conn_addr {
        return addr.ip().to_string();
    }

    // Default to unknown
    "unknown".to_string()
}

/// Process a DNS query and return an HTTP response
async fn process_dns_query(
    resolver: &DnsResolver,
    query_bytes: &[u8],
    client_ip: &str,
) -> Response {
    // Parse the DNS query
    let query = match DnsQuery::from_bytes(query_bytes) {
        Ok(q) => q,
        Err(e) => {
            warn!("Failed to parse DNS query: {}", e);
            let response = DnsResponse::servfail(0);
            return create_dns_response(
                &response,
                &DnsQuery::new(".", crate::dns::message::RecordType::A),
            );
        }
    };

    debug!(
        "DoH query: {} {} (ID: {})",
        query.name, query.record_type, query.id
    );

    // Resolve the query with client IP for logging
    let result = match resolver.resolve_with_client(&query, client_ip).await {
        Ok(r) => r,
        Err(e) => {
            warn!("Failed to resolve query for {}: {}", query.name, e);
            let response = DnsResponse::servfail(query.id);
            return create_dns_response(&response, &query);
        }
    };

    debug!(
        "DoH resolved {} {}: {} answers, cache_hit={}, time={}ms",
        query.name,
        query.record_type,
        result.response.answers.len(),
        result.metadata.cache_hit,
        result.metadata.response_time_ms
    );

    create_dns_response(&result.response, &query)
}

/// Create an HTTP response with DNS message content
fn create_dns_response(response: &DnsResponse, query: &DnsQuery) -> Response {
    match response.to_bytes(query) {
        Ok(bytes) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/dns-message")],
            bytes,
        )
            .into_response(),
        Err(e) => {
            warn!("Failed to encode DNS response: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to encode DNS response",
            )
                .into_response()
        }
    }
}

/// DoH JSON response format (alternative format)
#[derive(Debug, serde::Serialize)]
pub struct DohJsonResponse {
    /// Response status (0 = NOERROR)
    #[serde(rename = "Status")]
    pub status: u16,
    /// Whether truncation occurred
    #[serde(rename = "TC")]
    pub truncated: bool,
    /// Whether recursion was desired
    #[serde(rename = "RD")]
    pub recursion_desired: bool,
    /// Whether recursion is available
    #[serde(rename = "RA")]
    pub recursion_available: bool,
    /// Whether the response is authenticated
    #[serde(rename = "AD")]
    pub authenticated_data: bool,
    /// Whether checking is disabled
    #[serde(rename = "CD")]
    pub checking_disabled: bool,
    /// Question section
    #[serde(rename = "Question")]
    pub question: Vec<DohJsonQuestion>,
    /// Answer section
    #[serde(rename = "Answer", skip_serializing_if = "Vec::is_empty")]
    pub answer: Vec<DohJsonRecord>,
}

/// DoH JSON question format
#[derive(Debug, serde::Serialize)]
pub struct DohJsonQuestion {
    /// Domain name
    pub name: String,
    /// Record type
    #[serde(rename = "type")]
    pub record_type: u16,
}

/// DoH JSON record format
#[derive(Debug, serde::Serialize)]
pub struct DohJsonRecord {
    /// Domain name
    pub name: String,
    /// Record type
    #[serde(rename = "type")]
    pub record_type: u16,
    /// TTL
    #[serde(rename = "TTL")]
    pub ttl: u32,
    /// Record data
    pub data: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::cache::{CacheConfig, CacheManager};
    use crate::dns::message::{DnsRecordData, RecordType};
    use crate::dns::proxy::{ProxyManager, UpstreamManager};
    use crate::dns::rewrite::RewriteEngine;
    use crate::dns::CacheKey;
    use axum::body::Body;
    use axum::extract::connect_info::ConnectInfo;
    use axum::http::Request;
    use std::net::{Ipv4Addr, SocketAddr};
    use tower::ServiceExt;

    /// Attaches the connection info that `into_make_service_with_connect_info`
    /// supplies in production but `oneshot` does not.
    fn with_connect_info(mut request: Request<Body>) -> Request<Body> {
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        request.extensions_mut().insert(ConnectInfo(addr));
        request
    }

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

    #[tokio::test]
    async fn test_doh_server_creation() {
        let resolver = create_test_resolver();
        let server = DohDnsServer::new(resolver);
        let _router = server.router();
    }

    #[tokio::test]
    async fn test_doh_post_with_cache() {
        let resolver = create_test_resolver();

        // Pre-populate cache
        let cache_key = CacheKey::new("doh.example.com", RecordType::A);
        let mut response = DnsResponse::new(0);
        response.add_answer(DnsRecordData::a(
            "doh.example.com",
            Ipv4Addr::new(10, 0, 0, 1),
            300,
        ));
        resolver.cache().set(cache_key, response).await;

        let server = DohDnsServer::new(resolver);
        let router = server.router();

        // Create a DNS query
        let query = DnsQuery::with_id(54321, "doh.example.com", RecordType::A);
        let query_bytes = query.to_bytes().unwrap();

        // Make POST request
        let request = Request::builder()
            .method("POST")
            .uri("/dns-query")
            .header("Content-Type", "application/dns-message")
            .body(Body::from(query_bytes))
            .unwrap();

        let response = router.oneshot(with_connect_info(request)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Check content type
        let content_type = response.headers().get("content-type").unwrap();
        assert_eq!(content_type, "application/dns-message");
    }

    #[tokio::test]
    async fn test_doh_get_with_cache() {
        let resolver = create_test_resolver();

        // Pre-populate cache
        let cache_key = CacheKey::new("get.example.com", RecordType::A);
        let mut response = DnsResponse::new(0);
        response.add_answer(DnsRecordData::a(
            "get.example.com",
            Ipv4Addr::new(10, 0, 0, 2),
            300,
        ));
        resolver.cache().set(cache_key, response).await;

        let server = DohDnsServer::new(resolver);
        let router = server.router();

        // Create a DNS query and encode it
        let query = DnsQuery::with_id(11111, "get.example.com", RecordType::A);
        let query_bytes = query.to_bytes().unwrap();
        let encoded = URL_SAFE_NO_PAD.encode(&query_bytes);

        // Make GET request
        let request = Request::builder()
            .method("GET")
            .uri(format!("/dns-query?dns={}", encoded))
            .body(Body::empty())
            .unwrap();

        let response = router.oneshot(with_connect_info(request)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_doh_invalid_base64() {
        let resolver = create_test_resolver();
        let server = DohDnsServer::new(resolver);
        let router = server.router();

        // Make GET request with invalid base64
        let request = Request::builder()
            .method("GET")
            .uri("/dns-query?dns=invalid!!base64")
            .body(Body::empty())
            .unwrap();

        let response = router.oneshot(with_connect_info(request)).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
