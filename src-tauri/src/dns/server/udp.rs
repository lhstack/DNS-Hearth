//! UDP DNS Server
//!
//! Implements a standard DNS server over UDP protocol (port 53).

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use tokio::net::UdpSocket;
use tracing::{debug, error, info, warn};

use crate::dns::message::{DnsQuery, DnsResponse};
use crate::dns::resolver::DnsResolver;

/// Largest datagram accepted from a client.
///
/// Matches the largest EDNS(0) buffer this server is willing to honour so a
/// legitimate query is never cut in half by the receive buffer.
const MAX_UDP_DATAGRAM: usize = 4096;

/// UDP DNS Server
///
/// Handles DNS queries over UDP protocol.
pub struct UdpDnsServer {
    /// Bound UDP socket
    socket: UdpSocket,
    /// DNS resolver for processing queries
    resolver: Arc<DnsResolver>,
    /// Server bind address
    bind_addr: SocketAddr,
}

impl UdpDnsServer {
    /// Create a new UDP DNS server
    pub async fn new(bind_addr: SocketAddr, resolver: Arc<DnsResolver>) -> Result<Self> {
        let socket = UdpSocket::bind(bind_addr)
            .await
            .map_err(|e| anyhow!("Failed to bind UDP socket to {}: {}", bind_addr, e))?;

        info!("UDP DNS server bound to {}", bind_addr);

        Ok(Self {
            socket,
            resolver,
            bind_addr,
        })
    }

    /// Create a new UDP DNS server on the default port (53)
    pub async fn new_default(resolver: Arc<DnsResolver>) -> Result<Self> {
        Self::new("0.0.0.0:53".parse()?, resolver).await
    }

    /// Get the server's bind address
    pub fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }

    /// Get the local address the server is actually bound to
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.socket
            .local_addr()
            .map_err(|e| anyhow!("Failed to get local address: {}", e))
    }

    /// Run the UDP DNS server
    ///
    /// This method runs indefinitely, processing incoming DNS queries.
    pub async fn run(self: Arc<Self>) -> Result<()> {
        info!("UDP DNS server starting on {}", self.bind_addr);

        // EDNS(0) lets clients advertise buffers well past the classic 512
        // bytes; a 4096 byte buffer silently cut larger datagrams.
        let mut buf = vec![0u8; MAX_UDP_DATAGRAM];

        loop {
            match self.socket.recv_from(&mut buf).await {
                Ok((len, src)) => {
                    debug!("Received {} bytes from {}", len, src);
                    let data = buf[..len].to_vec();
                    let server = self.clone();

                    // Handle query in a separate task
                    tokio::spawn(async move {
                        if let Err(e) = server.handle_query_and_respond(data, src).await {
                            warn!("Error handling UDP query from {}: {}", src, e);
                        }
                    });
                }
                Err(e) => {
                    error!("Error receiving UDP packet: {}", e);
                }
            }
        }
    }

    /// Handle a single DNS query and send response
    async fn handle_query_and_respond(&self, data: Vec<u8>, src: SocketAddr) -> Result<()> {
        debug!("Processing query from {}", src);
        let client_ip = src.ip().to_string();
        let response_bytes = Self::handle_query_internal(&self.resolver, &data, &client_ip).await?;

        debug!("Sending {} byte response to {}", response_bytes.len(), src);
        self.socket
            .send_to(&response_bytes, src)
            .await
            .map_err(|e| anyhow!("Failed to send response to {}: {}", src, e))?;

        debug!("Response sent to {}", src);
        Ok(())
    }

    /// Handle a DNS query and return the response bytes
    async fn handle_query_internal(
        resolver: &DnsResolver,
        data: &[u8],
        client_ip: &str,
    ) -> Result<Vec<u8>> {
        // Parse the query
        let query = match DnsQuery::from_bytes(data) {
            Ok(q) => q,
            Err(e) => {
                debug!("Failed to parse DNS query: {}", e);
                // Return FORMERR response
                let response = DnsResponse::servfail(0);
                return response
                    .to_bytes(&DnsQuery::new(".", crate::dns::message::RecordType::A))
                    .map_err(|e| anyhow!("Failed to encode error response: {}", e));
            }
        };

        debug!(
            "Received UDP query: {} {} (ID: {})",
            query.name, query.record_type, query.id
        );

        // Resolve the query with client IP for logging
        let result = match resolver.resolve_with_client(&query, client_ip).await {
            Ok(r) => r,
            Err(e) => {
                warn!("Failed to resolve query for {}: {}", query.name, e);
                let response = DnsResponse::servfail(query.id);
                return response
                    .to_bytes(&query)
                    .map_err(|e| anyhow!("Failed to encode error response: {}", e));
            }
        };

        debug!(
            "Resolved {} {}: {} answers, cache_hit={}, time={}ms",
            query.name,
            query.record_type,
            result.response.answers.len(),
            result.metadata.cache_hit,
            result.metadata.response_time_ms
        );

        // Encode the response, honouring the datagram size limit
        result
            .response
            .to_bytes_for_udp(&query)
            .map_err(|e| anyhow!("Failed to encode response: {}", e))
    }

    /// Handle a single DNS query (for testing)
    pub async fn handle_query(&self, data: &[u8], src: SocketAddr) -> Result<Vec<u8>> {
        let client_ip = src.ip().to_string();
        Self::handle_query_internal(&self.resolver, data, &client_ip).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::cache::{CacheConfig, CacheManager};
    use crate::dns::message::{DnsRecordData, RecordType};
    use crate::dns::proxy::{ProxyManager, UpstreamManager};
    use crate::dns::rewrite::RewriteEngine;
    use crate::dns::CacheKey;
    use std::net::Ipv4Addr;

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
    async fn test_udp_server_creation() {
        let resolver = create_test_resolver();

        // Use a random high port for testing
        let server = UdpDnsServer::new("127.0.0.1:0".parse().unwrap(), resolver).await;
        assert!(server.is_ok());

        let server = server.unwrap();
        let local_addr = server.local_addr().unwrap();
        assert!(local_addr.port() > 0);
    }

    #[tokio::test]
    async fn test_handle_query_with_cache() {
        let resolver = create_test_resolver();

        // Pre-populate cache
        let cache_key = CacheKey::new("test.example.com", RecordType::A);
        let mut response = DnsResponse::new(0);
        response.add_answer(DnsRecordData::a(
            "test.example.com",
            Ipv4Addr::new(192, 168, 1, 100),
            300,
        ));
        resolver.cache().set(cache_key, response).await;

        let server = UdpDnsServer::new("127.0.0.1:0".parse().unwrap(), resolver)
            .await
            .unwrap();

        // Create a query
        let query = DnsQuery::with_id(12345, "test.example.com", RecordType::A);
        let query_bytes = query.to_bytes().unwrap();

        // Handle the query
        let response_bytes = server
            .handle_query(&query_bytes, "127.0.0.1:1234".parse().unwrap())
            .await
            .unwrap();

        // Parse the response
        let response = DnsResponse::from_bytes(&response_bytes).unwrap();
        assert_eq!(response.id, 12345);
        assert_eq!(response.answers.len(), 1);
        assert_eq!(response.answers[0].value, "192.168.1.100");
    }

    #[tokio::test]
    async fn test_handle_invalid_query() {
        let resolver = create_test_resolver();
        let server = UdpDnsServer::new("127.0.0.1:0".parse().unwrap(), resolver)
            .await
            .unwrap();

        // Send invalid data
        let invalid_data = vec![0u8; 10];
        let result = server
            .handle_query(&invalid_data, "127.0.0.1:1234".parse().unwrap())
            .await;

        // Should return a SERVFAIL response
        assert!(result.is_ok());
    }
}
