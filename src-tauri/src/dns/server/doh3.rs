//! DNS over HTTP/3 (DoH3) Server
//!
//! Serves DNS queries over HTTP/3, i.e. RFC 8484 semantics carried on QUIC
//! instead of TCP+TLS.
//!
//! This is a distinct listener from DoH: HTTP/3 is not reached by upgrading an
//! HTTP/1.1 connection, it needs its own QUIC endpoint advertising the `h3`
//! ALPN. That is why the DoH listener cannot serve DoH3 clients.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use bytes::{Buf, Bytes};
use http::{Method, Response, StatusCode};
use quinn::{Endpoint, ServerConfig};
use tracing::{debug, info, warn};

use super::dot::{TlsConfig, ALPN_H3};
use crate::dns::message::{DnsQuery, DnsResponse};
use crate::dns::resolver::DnsResolver;

/// Largest DNS message accepted in a request body.
const MAX_DNS_MESSAGE: usize = 65535;

/// DNS over HTTP/3 Server
pub struct Doh3DnsServer {
    endpoint: Endpoint,
    resolver: Arc<DnsResolver>,
    bind_addr: SocketAddr,
}

impl Doh3DnsServer {
    /// Create a new DoH3 server bound to `bind_addr`.
    pub async fn new(
        bind_addr: SocketAddr,
        tls_config: TlsConfig,
        resolver: Arc<DnsResolver>,
    ) -> Result<Self> {
        let server_config = Self::create_server_config(&tls_config)?;

        let endpoint = Endpoint::server(server_config, bind_addr)
            .map_err(|e| anyhow!("Failed to create QUIC endpoint for DoH3: {}", e))?;

        info!("DoH3 DNS server bound to {}", bind_addr);

        Ok(Self {
            endpoint,
            resolver,
            bind_addr,
        })
    }

    /// Build the QUIC server config, advertising the HTTP/3 ALPN.
    ///
    /// Without the `h3` ALPN token clients abort the handshake, so this differs
    /// from the DoQ config even though both run on QUIC.
    fn create_server_config(tls_config: &TlsConfig) -> Result<ServerConfig> {
        let crypto = tls_config.load(&[ALPN_H3])?;

        let server_config = ServerConfig::with_crypto(Arc::new(
            quinn::crypto::rustls::QuicServerConfig::try_from(crypto)
                .map_err(|e| anyhow!("Failed to create QUIC server config: {}", e))?,
        ));

        Ok(server_config)
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

    /// Run the DoH3 server, accepting QUIC connections until aborted.
    pub async fn run(&self) -> Result<()> {
        info!("DoH3 DNS server starting on {}", self.bind_addr);

        while let Some(incoming) = self.endpoint.accept().await {
            let resolver = self.resolver.clone();

            tokio::spawn(async move {
                match incoming.await {
                    Ok(connection) => {
                        let peer_addr = connection.remote_address();
                        debug!("New DoH3 connection from {}", peer_addr);

                        if let Err(e) =
                            Self::handle_connection(resolver, connection, peer_addr).await
                        {
                            debug!("DoH3 connection from {} ended: {}", peer_addr, e);
                        }
                    }
                    Err(e) => {
                        warn!("Failed to accept QUIC connection for DoH3: {}", e);
                    }
                }
            });
        }

        Ok(())
    }

    /// Serve every HTTP/3 request arriving on one QUIC connection.
    async fn handle_connection(
        resolver: Arc<DnsResolver>,
        connection: quinn::Connection,
        peer_addr: SocketAddr,
    ) -> Result<()> {
        let mut h3_conn: h3::server::Connection<h3_quinn::Connection, Bytes> =
            h3::server::Connection::new(h3_quinn::Connection::new(connection))
                .await
                .map_err(|e| anyhow!("HTTP/3 handshake failed: {}", e))?;

        loop {
            match h3_conn.accept().await {
                Ok(Some(request_resolver)) => {
                    let resolver = resolver.clone();

                    tokio::spawn(async move {
                        if let Err(e) =
                            Self::handle_request(resolver, request_resolver, peer_addr).await
                        {
                            debug!("DoH3 request from {} failed: {}", peer_addr, e);
                        }
                    });
                }
                Ok(None) => {
                    debug!("DoH3 connection closed by {}", peer_addr);
                    return Ok(());
                }
                Err(e) => {
                    return Err(anyhow!("HTTP/3 accept failed: {}", e));
                }
            }
        }
    }

    /// Handle one HTTP/3 request, supporting the RFC 8484 GET and POST forms.
    async fn handle_request(
        resolver: Arc<DnsResolver>,
        request_resolver: h3::server::RequestResolver<h3_quinn::Connection, Bytes>,
        peer_addr: SocketAddr,
    ) -> Result<()> {
        let (request, mut stream) = request_resolver
            .resolve_request()
            .await
            .map_err(|e| anyhow!("Failed to resolve HTTP/3 request: {}", e))?;

        let client_ip = peer_addr.ip().to_string();

        let query_bytes = match Self::extract_query(&request, &mut stream).await {
            Ok(bytes) => bytes,
            Err(status) => return Self::respond_status(&mut stream, status).await,
        };

        let response_bytes = Self::resolve_to_wire(&resolver, &query_bytes, &client_ip).await?;

        let response = Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/dns-message")
            .header("content-length", response_bytes.len().to_string())
            .body(())
            .map_err(|e| anyhow!("Failed to build response: {}", e))?;

        stream
            .send_response(response)
            .await
            .map_err(|e| anyhow!("Failed to send response headers: {}", e))?;
        stream
            .send_data(Bytes::from(response_bytes))
            .await
            .map_err(|e| anyhow!("Failed to send response body: {}", e))?;
        stream
            .finish()
            .await
            .map_err(|e| anyhow!("Failed to finish response: {}", e))?;

        Ok(())
    }

    /// Pull the raw DNS message out of a GET query string or a POST body.
    async fn extract_query(
        request: &http::Request<()>,
        stream: &mut h3::server::RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>,
    ) -> std::result::Result<Vec<u8>, StatusCode> {
        match *request.method() {
            Method::GET => {
                use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

                let query = request.uri().query().ok_or(StatusCode::BAD_REQUEST)?;
                let encoded = query
                    .split('&')
                    .find_map(|pair| pair.strip_prefix("dns="))
                    .ok_or(StatusCode::BAD_REQUEST)?;

                URL_SAFE_NO_PAD
                    .decode(encoded)
                    .map_err(|_| StatusCode::BAD_REQUEST)
            }
            Method::POST => {
                let mut body = Vec::new();

                loop {
                    match stream.recv_data().await {
                        Ok(Some(mut chunk)) => {
                            while chunk.has_remaining() {
                                let consumed = {
                                    let bytes = chunk.chunk();
                                    if body.len() + bytes.len() > MAX_DNS_MESSAGE {
                                        return Err(StatusCode::PAYLOAD_TOO_LARGE);
                                    }
                                    body.extend_from_slice(bytes);
                                    bytes.len()
                                };
                                chunk.advance(consumed);
                            }
                        }
                        Ok(None) => break,
                        Err(_) => return Err(StatusCode::BAD_REQUEST),
                    }
                }

                if body.is_empty() {
                    return Err(StatusCode::BAD_REQUEST);
                }

                Ok(body)
            }
            _ => Err(StatusCode::METHOD_NOT_ALLOWED),
        }
    }

    /// Send a bodyless error response.
    async fn respond_status(
        stream: &mut h3::server::RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>,
        status: StatusCode,
    ) -> Result<()> {
        let response = Response::builder()
            .status(status)
            .body(())
            .map_err(|e| anyhow!("Failed to build error response: {}", e))?;

        stream
            .send_response(response)
            .await
            .map_err(|e| anyhow!("Failed to send error response: {}", e))?;
        stream
            .finish()
            .await
            .map_err(|e| anyhow!("Failed to finish error response: {}", e))?;

        Ok(())
    }

    /// Resolve a wire-format query into a wire-format response.
    ///
    /// Parse and resolve failures are reported as DNS-level SERVFAIL rather than
    /// HTTP errors, matching the other listeners.
    async fn resolve_to_wire(
        resolver: &DnsResolver,
        data: &[u8],
        client_ip: &str,
    ) -> Result<Vec<u8>> {
        let query = match DnsQuery::from_bytes(data) {
            Ok(q) => q,
            Err(e) => {
                debug!("Failed to parse DoH3 query: {}", e);
                let fallback = DnsQuery::new(".", crate::dns::message::RecordType::A);
                return DnsResponse::servfail(0)
                    .to_bytes(&fallback)
                    .map_err(|e| anyhow!("Failed to encode error response: {}", e));
            }
        };

        debug!(
            "Received DoH3 query: {} {} (ID: {})",
            query.name, query.record_type, query.id
        );

        let result = match resolver.resolve_with_client(&query, client_ip).await {
            Ok(r) => r,
            Err(e) => {
                warn!("Failed to resolve DoH3 query for {}: {}", query.name, e);
                return DnsResponse::servfail(query.id)
                    .to_bytes(&query)
                    .map_err(|e| anyhow!("Failed to encode error response: {}", e));
            }
        };

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
    fn test_doh3_tls_config_paths() {
        let tls_config = TlsConfig::new("/path/to/cert.pem", "/path/to/key.pem");
        assert_eq!(tls_config.cert_path, "/path/to/cert.pem");
        assert_eq!(tls_config.key_path, "/path/to/key.pem");
    }

    #[test]
    fn test_max_dns_message_matches_wire_limit() {
        assert_eq!(MAX_DNS_MESSAGE, 65535);
    }
}
