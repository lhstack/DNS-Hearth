//! DNS over TLS (DoT) Server
//!
//! Implements a DNS server over TLS protocol (port 853).

#![allow(dead_code)]

use std::fs::File;
use std::io::BufReader;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use rustls::pki_types::CertificateDer;
use rustls::ServerConfig;
use rustls_pemfile::{certs, private_key};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tracing::{debug, error, info, warn};

use crate::dns::message::{DnsQuery, DnsResponse};
use crate::dns::resolver::DnsResolver;

/// TLS configuration for the DoT server
#[derive(Clone)]
pub struct TlsConfig {
    /// Path to the certificate file (PEM format)
    pub cert_path: String,
    /// Path to the private key file (PEM format)
    pub key_path: String,
}

/// ALPN protocol identifier required by DoT clients (RFC 7858).
pub const ALPN_DOT: &[u8] = b"dot";
/// ALPN protocol identifier required by DoQ clients (RFC 9250).
pub const ALPN_DOQ: &[u8] = b"doq";
/// ALPN identifiers offered to DoH clients over TLS (RFC 8484 uses plain HTTP).
pub const ALPN_HTTP: &[&[u8]] = &[b"h2", b"http/1.1"];
/// ALPN protocol identifier required by HTTP/3 clients.
pub const ALPN_H3: &[u8] = b"h3";

impl TlsConfig {
    /// Create a new TLS configuration
    pub fn new(cert_path: impl Into<String>, key_path: impl Into<String>) -> Self {
        Self {
            cert_path: cert_path.into(),
            key_path: key_path.into(),
        }
    }

    /// Load the TLS configuration and create a ServerConfig.
    ///
    /// `alpn_protocols` must list the identifiers this listener speaks. Serving
    /// an encrypted DNS transport without ALPN makes conforming clients abort
    /// the handshake, so callers always have to state what they speak.
    pub fn load(&self, alpn_protocols: &[&[u8]]) -> Result<ServerConfig> {
        // Load certificate chain
        let cert_file = File::open(&self.cert_path)
            .map_err(|e| anyhow!("Failed to open certificate file {}: {}", self.cert_path, e))?;
        let mut cert_reader = BufReader::new(cert_file);
        let certs: Vec<CertificateDer<'static>> =
            certs(&mut cert_reader).filter_map(|r| r.ok()).collect();

        if certs.is_empty() {
            return Err(anyhow!("No certificates found in {}", self.cert_path));
        }

        // Load private key
        let key_file = File::open(&self.key_path)
            .map_err(|e| anyhow!("Failed to open key file {}: {}", self.key_path, e))?;
        let mut key_reader = BufReader::new(key_file);

        let key = private_key(&mut key_reader)
            .map_err(|e| anyhow!("Failed to parse private key: {}", e))?
            .ok_or_else(|| anyhow!("No private key found in {}", self.key_path))?;

        // Build server config
        let mut config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| anyhow!("Failed to build TLS config: {}", e))?;

        config.alpn_protocols = alpn_protocols.iter().map(|p| p.to_vec()).collect();

        Ok(config)
    }
}

/// DNS over TLS Server
///
/// Handles DNS queries over TLS protocol.
pub struct DotDnsServer {
    /// TCP listener
    listener: TcpListener,
    /// TLS acceptor
    acceptor: TlsAcceptor,
    /// DNS resolver
    resolver: Arc<DnsResolver>,
    /// Server bind address
    bind_addr: SocketAddr,
}

impl DotDnsServer {
    /// Create a new DoT DNS server
    pub async fn new(
        bind_addr: SocketAddr,
        tls_config: TlsConfig,
        resolver: Arc<DnsResolver>,
    ) -> Result<Self> {
        let server_config = tls_config.load(&[ALPN_DOT])?;
        let acceptor = TlsAcceptor::from(Arc::new(server_config));

        let listener = TcpListener::bind(bind_addr)
            .await
            .map_err(|e| anyhow!("Failed to bind TCP listener to {}: {}", bind_addr, e))?;

        info!("DoT DNS server bound to {}", bind_addr);

        Ok(Self {
            listener,
            acceptor,
            resolver,
            bind_addr,
        })
    }

    /// Create a new DoT DNS server on the default port (853)
    pub async fn new_default(tls_config: TlsConfig, resolver: Arc<DnsResolver>) -> Result<Self> {
        Self::new("0.0.0.0:853".parse()?, tls_config, resolver).await
    }

    /// Get the server's bind address
    pub fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }

    /// Get the local address the server is actually bound to
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.listener
            .local_addr()
            .map_err(|e| anyhow!("Failed to get local address: {}", e))
    }

    /// Run the DoT DNS server
    ///
    /// This method runs indefinitely, processing incoming TLS connections.
    pub async fn run(&self) -> Result<()> {
        info!("DoT DNS server starting on {}", self.bind_addr);

        loop {
            match self.listener.accept().await {
                Ok((stream, peer_addr)) => {
                    let acceptor = self.acceptor.clone();
                    let resolver = self.resolver.clone();

                    tokio::spawn(async move {
                        if let Err(e) =
                            Self::handle_connection(acceptor, resolver, stream, peer_addr).await
                        {
                            warn!("Error handling DoT connection from {}: {}", peer_addr, e);
                        }
                    });
                }
                Err(e) => {
                    error!("Error accepting TCP connection: {}", e);
                }
            }
        }
    }

    /// Handle a single TLS connection
    async fn handle_connection(
        acceptor: TlsAcceptor,
        resolver: Arc<DnsResolver>,
        stream: TcpStream,
        peer_addr: SocketAddr,
    ) -> Result<()> {
        debug!("New DoT connection from {}", peer_addr);

        // Perform TLS handshake
        let mut tls_stream = acceptor
            .accept(stream)
            .await
            .map_err(|e| anyhow!("TLS handshake failed: {}", e))?;

        debug!("TLS handshake completed with {}", peer_addr);

        // Handle multiple queries on the same connection (TCP DNS allows this)
        loop {
            // Read query length (2 bytes, big-endian)
            let mut len_buf = [0u8; 2];
            match tls_stream.read_exact(&mut len_buf).await {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    debug!("DoT connection closed by {}", peer_addr);
                    break;
                }
                Err(e) => {
                    return Err(anyhow!("Failed to read query length: {}", e));
                }
            }

            let query_len = u16::from_be_bytes(len_buf) as usize;
            if query_len == 0 || query_len > 65535 {
                return Err(anyhow!("Invalid query length: {}", query_len));
            }

            // Read query data
            let mut query_buf = vec![0u8; query_len];
            tls_stream
                .read_exact(&mut query_buf)
                .await
                .map_err(|e| anyhow!("Failed to read query data: {}", e))?;

            // Process the query
            let client_ip = peer_addr.ip().to_string();
            let response_bytes = Self::handle_query(&resolver, &query_buf, &client_ip).await?;

            // Write response length
            let response_len = (response_bytes.len() as u16).to_be_bytes();
            tls_stream
                .write_all(&response_len)
                .await
                .map_err(|e| anyhow!("Failed to write response length: {}", e))?;

            // Write response data
            tls_stream
                .write_all(&response_bytes)
                .await
                .map_err(|e| anyhow!("Failed to write response data: {}", e))?;

            tls_stream
                .flush()
                .await
                .map_err(|e| anyhow!("Failed to flush response: {}", e))?;
        }

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
            "Received DoT query: {} {} (ID: {})",
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
    fn test_tls_config_creation() {
        let config = TlsConfig::new("/path/to/cert.pem", "/path/to/key.pem");
        assert_eq!(config.cert_path, "/path/to/cert.pem");
        assert_eq!(config.key_path, "/path/to/key.pem");
    }

    // Note: Full DoT server tests require valid TLS certificates
    // which are not available in unit tests. Integration tests
    // should be used for full server testing.
}
