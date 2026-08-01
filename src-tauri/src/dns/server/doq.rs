//! DNS over QUIC (DoQ) Server
//!
//! Implements a DNS server over QUIC protocol (port 853).
//! Based on RFC 9250.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use quinn::{Endpoint, ServerConfig};
use tracing::{debug, info, warn};

use super::dot::{TlsConfig, ALPN_DOQ};
use crate::dns::message::{DnsQuery, DnsResponse};
use crate::dns::resolver::DnsResolver;

/// DNS over QUIC Server
///
/// Handles DNS queries over QUIC protocol.
pub struct DoqDnsServer {
    /// QUIC endpoint
    endpoint: Endpoint,
    /// DNS resolver
    resolver: Arc<DnsResolver>,
    /// Server bind address
    bind_addr: SocketAddr,
}

impl DoqDnsServer {
    /// Create a new DoQ DNS server
    pub async fn new(
        bind_addr: SocketAddr,
        tls_config: TlsConfig,
        resolver: Arc<DnsResolver>,
    ) -> Result<Self> {
        let server_config = Self::create_server_config(&tls_config)?;

        let endpoint = Endpoint::server(server_config, bind_addr)
            .map_err(|e| anyhow!("Failed to create QUIC endpoint: {}", e))?;

        info!("DoQ DNS server bound to {}", bind_addr);

        Ok(Self {
            endpoint,
            resolver,
            bind_addr,
        })
    }

    /// Create QUIC server configuration from TLS config
    ///
    /// RFC 9250 requires the `doq` ALPN identifier; without it conforming
    /// clients abort the handshake, which made this listener unreachable.
    fn create_server_config(tls_config: &TlsConfig) -> Result<ServerConfig> {
        let crypto = tls_config.load(&[ALPN_DOQ])?;

        let server_config = ServerConfig::with_crypto(Arc::new(
            quinn::crypto::rustls::QuicServerConfig::try_from(crypto)
                .map_err(|e| anyhow!("Failed to create QUIC server config: {}", e))?,
        ));

        Ok(server_config)
    }

    /// Create a new DoQ DNS server on the default port (853)
    pub async fn new_default(tls_config: TlsConfig, resolver: Arc<DnsResolver>) -> Result<Self> {
        Self::new("0.0.0.0:853".parse()?, tls_config, resolver).await
    }

    /// Get the server's bind address
    pub fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }

    /// Get the local address the server is actually bound to
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.endpoint
            .local_addr()
            .map_err(|e| anyhow!("Failed to get local address: {}", e))
    }

    /// Run the DoQ DNS server
    ///
    /// This method runs indefinitely, processing incoming QUIC connections.
    pub async fn run(&self) -> Result<()> {
        info!("DoQ DNS server starting on {}", self.bind_addr);

        while let Some(connecting) = self.endpoint.accept().await {
            let resolver = self.resolver.clone();

            tokio::spawn(async move {
                match connecting.await {
                    Ok(connection) => {
                        let peer_addr = connection.remote_address();
                        debug!("New DoQ connection from {}", peer_addr);

                        if let Err(e) = Self::handle_connection(resolver, connection).await {
                            warn!("Error handling DoQ connection from {}: {}", peer_addr, e);
                        }
                    }
                    Err(e) => {
                        warn!("Failed to accept QUIC connection: {}", e);
                    }
                }
            });
        }

        Ok(())
    }

    /// Handle a single QUIC connection
    async fn handle_connection(
        resolver: Arc<DnsResolver>,
        connection: quinn::Connection,
    ) -> Result<()> {
        let peer_addr = connection.remote_address();

        // Handle multiple streams on the same connection
        loop {
            match connection.accept_bi().await {
                Ok((send, recv)) => {
                    let resolver = resolver.clone();
                    let peer = peer_addr;

                    tokio::spawn(async move {
                        if let Err(e) = Self::handle_stream(resolver, send, recv, peer).await {
                            debug!("Error handling DoQ stream from {}: {}", peer, e);
                        }
                    });
                }
                Err(quinn::ConnectionError::ApplicationClosed(_)) => {
                    debug!("DoQ connection closed by {}", peer_addr);
                    break;
                }
                Err(e) => {
                    warn!("Error accepting stream from {}: {}", peer_addr, e);
                    break;
                }
            }
        }

        Ok(())
    }

    /// Handle a single QUIC stream (one DNS query/response)
    async fn handle_stream(
        resolver: Arc<DnsResolver>,
        mut send: quinn::SendStream,
        mut recv: quinn::RecvStream,
        peer_addr: SocketAddr,
    ) -> Result<()> {
        // Read query length (2 bytes, big-endian)
        let mut len_buf = [0u8; 2];
        recv.read_exact(&mut len_buf)
            .await
            .map_err(|e| anyhow!("Failed to read query length: {}", e))?;

        let query_len = u16::from_be_bytes(len_buf) as usize;
        if query_len == 0 || query_len > 65535 {
            return Err(anyhow!("Invalid query length: {}", query_len));
        }

        // Read query data
        let mut query_buf = vec![0u8; query_len];
        recv.read_exact(&mut query_buf)
            .await
            .map_err(|e| anyhow!("Failed to read query data: {}", e))?;

        // Process the query
        let client_ip = peer_addr.ip().to_string();
        let response_bytes = Self::handle_query(&resolver, &query_buf, &client_ip).await?;

        // Write response length
        let response_len = (response_bytes.len() as u16).to_be_bytes();
        send.write_all(&response_len)
            .await
            .map_err(|e| anyhow!("Failed to write response length: {}", e))?;

        // Write response data
        send.write_all(&response_bytes)
            .await
            .map_err(|e| anyhow!("Failed to write response data: {}", e))?;

        // Finish the stream (no longer async in quinn 0.11)
        send.finish()
            .map_err(|e| anyhow!("Failed to finish stream: {}", e))?;

        Ok(())
    }

    /// Handle a DNS query and return the response bytes
    async fn handle_query(resolver: &DnsResolver, data: &[u8], client_ip: &str) -> Result<Vec<u8>> {
        // Parse the query
        let query = match DnsQuery::from_bytes(data) {
            Ok(q) => q,
            Err(e) => {
                debug!("Failed to parse DNS query: {}", e);
                let response = DnsResponse::servfail(0);
                return response
                    .to_bytes(&DnsQuery::new(".", crate::dns::message::RecordType::A))
                    .map_err(|e| anyhow!("Failed to encode error response: {}", e));
            }
        };

        debug!(
            "Received DoQ query: {} {} (ID: {})",
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

        // Encode the response
        result
            .response
            .to_bytes(&query)
            .map_err(|e| anyhow!("Failed to encode response: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_doq_server_config() {
        // Note: Full DoQ server tests require valid TLS certificates
        // which are not available in unit tests. Integration tests
        // should be used for full server testing.

        let tls_config = TlsConfig::new("/path/to/cert.pem", "/path/to/key.pem");
        assert_eq!(tls_config.cert_path, "/path/to/cert.pem");
        assert_eq!(tls_config.key_path, "/path/to/key.pem");
    }
}
