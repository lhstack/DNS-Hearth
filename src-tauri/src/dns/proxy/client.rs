//! DNS Upstream Clients
//!
//! Provides client implementations for querying upstream DNS servers
//! using different protocols (UDP, DoT, DoH, DoQ).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use bytes::Bytes;
use h3::client::SendRequest;
use h3_quinn::OpenStreams;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::net::UdpSocket;
use tokio::sync::OnceCell;
use tokio::time::timeout;

type H3SendRequest = SendRequest<OpenStreams, Bytes>;

use super::upstream::{UpstreamProtocol, UpstreamServer};
use crate::dns::message::{message_is_truncated, message_transaction_id, DnsQuery, DnsResponse};

/// Largest UDP datagram accepted from an upstream.
///
/// EDNS(0) buffers above this are rare and a fixed ceiling keeps a hostile
/// upstream from driving per-query allocations.
const MAX_UDP_DATAGRAM: usize = 4096;

/// Upper bound on a DNS message, used to reject absurd length prefixes.
const MAX_DNS_MESSAGE: usize = 65535;

/// Default port for the HTTPS-based transports (DoH and DoH3).
const HTTPS_PORT: u16 = 443;

/// Parse an address string that may contain IPv6 in bracket notation.
/// Supports formats:
/// - IPv4: "1.1.1.1:53" or "1.1.1.1"
/// - IPv6: "[2001:4860:4860::8888]:53" or "[::1]:853"
/// - Hostname: "dns.google:853" or "dns.google"
/// Returns (host, port) tuple where host has brackets stripped for IPv6
/// A malformed port is reported instead of being replaced by the default:
/// silently rewriting `[::1]:99999` to the protocol default connected the user
/// to a different endpoint than the one they configured, with nothing in the
/// logs to say so.
fn parse_host_port(address: &str, default_port: u16) -> Result<(String, u16)> {
    fn parse_port(raw: &str, address: &str) -> Result<u16> {
        raw.parse::<u16>()
            .map_err(|_| anyhow!("Invalid port '{}' in address '{}'", raw, address))
    }

    // IPv6 in brackets: [::1] or [::1]:port
    if address.starts_with('[') {
        let bracket_end = address
            .find(']')
            .ok_or_else(|| anyhow!("Unterminated IPv6 bracket in address '{}'", address))?;

        let host = address[1..bracket_end].to_string();
        let rest = &address[bracket_end + 1..];

        let port = match rest.strip_prefix(':') {
            Some(raw) => parse_port(raw, address)?,
            None if rest.is_empty() => default_port,
            None => {
                return Err(anyhow!(
                    "Unexpected trailing '{}' in address '{}'",
                    rest,
                    address
                ))
            }
        };

        return Ok((host, port));
    }

    // A bare IPv6 literal has colons but carries no port.
    if address.parse::<std::net::Ipv6Addr>().is_ok() {
        return Ok((address.to_string(), default_port));
    }

    // Otherwise a single colon separates host and port.
    if let Some((host, raw_port)) = address.rsplit_once(':') {
        return Ok((host.to_string(), parse_port(raw_port, address)?));
    }

    Ok((address.to_string(), default_port))
}

/// Query used to probe whether an upstream is answering.
///
/// A root `NS` lookup is answerable by any recursive resolver. Probing a
/// third-party name like `dns.google` tied the health verdict to that zone
/// being reachable, so upstreams that were working fine got marked unhealthy
/// whenever that single name was disrupted.
fn health_probe_query() -> DnsQuery {
    DnsQuery::new(".", crate::dns::message::RecordType::NS)
}

/// Read an HTTP body, aborting once it exceeds `limit` bytes.
///
/// `content_length` is only advisory (and absent for chunked responses), so the
/// limit has to be enforced while streaming rather than trusted up front.
async fn read_body_capped(response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    let mut response = response;

    while let Some(chunk) = response.chunk().await? {
        if body.len() + chunk.len() > limit {
            return Err(anyhow!("DoH response exceeds the {} byte DNS limit", limit));
        }
        body.extend_from_slice(&chunk);
    }

    Ok(body)
}

/// Global QUIC endpoint cache for DoQ clients
/// Reusing endpoints significantly improves performance by avoiding
/// repeated socket binding and configuration overhead.
///
/// We use a pool of endpoints to distribute load and avoid contention.
const ENDPOINT_POOL_SIZE: usize = 20;

static DOQ_ENDPOINTS_V4: OnceCell<Vec<quinn::Endpoint>> = OnceCell::const_new();
static DOQ_INDEX_V4: AtomicUsize = AtomicUsize::new(0);

static DOQ_ENDPOINTS_V6: OnceCell<Vec<quinn::Endpoint>> = OnceCell::const_new();
static DOQ_INDEX_V6: AtomicUsize = AtomicUsize::new(0);

/// Global QUIC endpoint cache for DoH3 clients
static DOH3_ENDPOINTS_V4: OnceCell<Vec<quinn::Endpoint>> = OnceCell::const_new();
static DOH3_INDEX_V4: AtomicUsize = AtomicUsize::new(0);

static DOH3_ENDPOINTS_V6: OnceCell<Vec<quinn::Endpoint>> = OnceCell::const_new();
static DOH3_INDEX_V6: AtomicUsize = AtomicUsize::new(0);

/// Global DoT connection pool.
///
/// Each upstream keeps a small stack of idle TLS connections. A single slot per
/// upstream would serialise nothing and cache nothing: concurrent queries all
/// miss the cache, and the last one to finish evicts everyone else's connection,
/// so every query paid for a fresh TLS handshake.
use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_rustls::client::TlsStream;

type DotConnection = TlsStream<TcpStream>;

/// Maximum idle DoT connections retained per upstream.
const DOT_MAX_IDLE_PER_HOST: usize = 8;

static DOT_POOL: OnceLock<Mutex<HashMap<String, Vec<DotConnection>>>> = OnceLock::new();

fn get_dot_pool() -> &'static Mutex<HashMap<String, Vec<DotConnection>>> {
    DOT_POOL.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Take an idle connection for this upstream, if one is available.
async fn dot_pool_take(pool_key: &str) -> Option<DotConnection> {
    let mut pool = get_dot_pool().lock().await;
    pool.get_mut(pool_key).and_then(|idle| idle.pop())
}

/// Return a healthy connection to the idle set, dropping it when full.
async fn dot_pool_put(pool_key: &str, conn: DotConnection) {
    let mut pool = get_dot_pool().lock().await;
    let idle = pool.entry(pool_key.to_string()).or_default();
    if idle.len() < DOT_MAX_IDLE_PER_HOST {
        idle.push(conn);
    }
}

/// QUIC protocol type for endpoint caching
#[derive(Clone, Copy)]
enum QuicProtocol {
    Doq,
    Doh3,
}

/// Get or create a cached QUIC endpoint for the given protocol and address family
fn get_quic_endpoint(protocol: QuicProtocol, is_ipv6: bool) -> Result<&'static quinn::Endpoint> {
    let (cell, index) = match (protocol, is_ipv6) {
        (QuicProtocol::Doq, false) => (&DOQ_ENDPOINTS_V4, &DOQ_INDEX_V4),
        (QuicProtocol::Doq, true) => (&DOQ_ENDPOINTS_V6, &DOQ_INDEX_V6),
        (QuicProtocol::Doh3, false) => (&DOH3_ENDPOINTS_V4, &DOH3_INDEX_V4),
        (QuicProtocol::Doh3, true) => (&DOH3_ENDPOINTS_V6, &DOH3_INDEX_V6),
    };

    // Try to get existing endpoint pool
    if let Some(endpoints) = cell.get() {
        let idx = index.fetch_add(1, Ordering::Relaxed) % endpoints.len();
        return Ok(&endpoints[idx]);
    }

    // Create new endpoint pool
    let mut eps = Vec::with_capacity(ENDPOINT_POOL_SIZE);

    for _ in 0..ENDPOINT_POOL_SIZE {
        // Create new endpoint with appropriate config
        let bind_addr: SocketAddr = if is_ipv6 {
            "[::]:0".parse()?
        } else {
            "0.0.0.0:0".parse()?
        };

        // Create TLS config with certificate verification disabled (for IP-based connections)
        let mut crypto = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerifier))
            .with_no_client_auth();

        // Set ALPN protocol based on QUIC protocol type
        crypto.alpn_protocols = match protocol {
            QuicProtocol::Doq => vec![b"doq".to_vec()],
            QuicProtocol::Doh3 => vec![b"h3".to_vec()],
        };

        let quic_crypto = quinn::crypto::rustls::QuicClientConfig::try_from(crypto)
            .map_err(|e| anyhow!("Failed to create QUIC client config: {}", e))?;

        // Configure Transport (Keep-Alive)
        let mut transport = quinn::TransportConfig::default();
        // Send keep-alive every 5 seconds to maintain NAT mappings
        transport.keep_alive_interval(Some(Duration::from_secs(5)));
        // Set max idle timeout to 20 seconds
        transport.max_idle_timeout(Some(quinn::VarInt::from_u32(20_000).into()));

        let mut client_config = quinn::ClientConfig::new(Arc::new(quic_crypto));
        client_config.transport_config(Arc::new(transport));

        let mut endpoint = quinn::Endpoint::client(bind_addr)?;
        endpoint.set_default_client_config(client_config);

        eps.push(endpoint);
    }

    // Try to store it
    // If another thread beat us to it, the result will be Err(value), but we just discard our created value
    // and use the one in the cell.
    let _ = cell.set(eps);

    // Get from cell (guaranteed to be Some now)
    let endpoints = cell.get().unwrap();
    let idx = index.fetch_add(1, Ordering::Relaxed) % endpoints.len();
    Ok(&endpoints[idx])
}

/// Result of a DNS query to an upstream server
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// The DNS response
    pub response: DnsResponse,
    /// Response time in milliseconds
    pub response_time_ms: u64,
    /// Server ID that responded
    pub server_id: i64,
    /// Server name
    pub server_name: String,
}

/// Trait for DNS upstream clients
#[async_trait]
pub trait DnsClient: Send + Sync {
    /// Query the upstream server
    async fn query(&self, query: &DnsQuery) -> Result<QueryResult>;

    /// Get the server this client is connected to
    #[allow(dead_code)]
    fn server(&self) -> &UpstreamServer;

    /// Check if the server is reachable (health check)
    #[allow(dead_code)]
    async fn health_check(&self) -> Result<Duration>;
}

/// UDP DNS Client
///
/// Queries upstream DNS servers using standard UDP protocol.
pub struct UdpDnsClient {
    server: UpstreamServer,
    #[allow(dead_code)]
    socket: Option<UdpSocket>,
}

impl UdpDnsClient {
    /// Create a new UDP DNS client
    pub fn new(server: UpstreamServer) -> Self {
        Self {
            server,
            socket: None,
        }
    }

    /// Parse the server address with IPv6 support
    /// Supports formats: "1.1.1.1:53", "[2001:4860:4860::8888]:53", "dns.google:53"
    fn parse_address(&self) -> Result<SocketAddr> {
        let (host, port) =
            parse_host_port(&self.server.address, UpstreamProtocol::Udp.default_port())?;

        // Try to parse as IP address directly
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            return Ok(SocketAddr::new(ip, port));
        }

        // For hostnames, format with brackets for IPv6 compatibility
        let addr_str = format!("{}:{}", host, port);
        addr_str
            .parse()
            .map_err(|e| anyhow!("Invalid address format '{}': {}", self.server.address, e))
    }

    /// Send a query over UDP and receive a response addressed to this query.
    ///
    /// Datagrams whose source address or transaction ID do not match the
    /// outstanding query are discarded rather than returned: on a connectionless
    /// socket anyone able to guess the ephemeral port can inject an answer, and
    /// accepting the first datagram that arrives would let that answer enter the
    /// cache (RFC 5452).
    async fn send_query(
        &self,
        query_bytes: &[u8],
        server_addr: SocketAddr,
        expected_id: u16,
    ) -> Result<Vec<u8>> {
        use tracing::{debug, warn};

        // Bind to appropriate address family based on target
        let bind_addr = if server_addr.is_ipv6() {
            "[::]:0"
        } else {
            "0.0.0.0:0"
        };
        let socket = UdpSocket::bind(bind_addr)
            .await
            .map_err(|e| anyhow!("Failed to bind UDP socket to {}: {}", bind_addr, e))?;

        debug!(
            "Sending UDP query to {} ({} bytes)",
            server_addr,
            query_bytes.len()
        );

        let sent = socket.send_to(query_bytes, server_addr).await?;
        debug!("Sent {} bytes to {}", sent, server_addr);

        let deadline = Instant::now() + self.server.timeout;
        let mut buf = vec![0u8; MAX_UDP_DATAGRAM];

        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| anyhow!("Query timeout after {:?}", self.server.timeout))?;

            let (len, from) = timeout(remaining, socket.recv_from(&mut buf))
                .await
                .map_err(|_| anyhow!("Query timeout after {:?}", self.server.timeout))??;

            if from != server_addr {
                warn!(
                    "Discarding UDP response for {} from unexpected source {} (expected {})",
                    self.server.name, from, server_addr
                );
                continue;
            }

            match message_transaction_id(&buf[..len]) {
                Some(id) if id == expected_id => {
                    debug!("Received {} bytes from {}", len, from);
                    return Ok(buf[..len].to_vec());
                }
                Some(id) => {
                    warn!(
                        "Discarding UDP response from {} with transaction ID {} (expected {})",
                        from, id, expected_id
                    );
                }
                None => {
                    warn!("Discarding malformed UDP response from {}", from);
                }
            }
        }
    }

    /// Re-issue a query over TCP, used when the UDP answer came back truncated.
    async fn send_query_tcp(&self, query_bytes: &[u8], server_addr: SocketAddr) -> Result<Vec<u8>> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let mut stream = timeout(self.server.timeout, TcpStream::connect(server_addr))
            .await
            .map_err(|_| anyhow!("TCP connect timeout to {}", server_addr))??;

        let len = (query_bytes.len() as u16).to_be_bytes();
        stream.write_all(&len).await?;
        stream.write_all(query_bytes).await?;
        stream.flush().await?;

        let mut len_buf = [0u8; 2];
        timeout(self.server.timeout, stream.read_exact(&mut len_buf))
            .await
            .map_err(|_| anyhow!("TCP read timeout from {}", server_addr))??;

        let response_len = u16::from_be_bytes(len_buf) as usize;
        if response_len == 0 {
            return Err(anyhow!("Empty TCP response from {}", server_addr));
        }

        let mut response = vec![0u8; response_len];
        timeout(self.server.timeout, stream.read_exact(&mut response))
            .await
            .map_err(|_| anyhow!("TCP read timeout from {}", server_addr))??;

        Ok(response)
    }
}

#[async_trait]
impl DnsClient for UdpDnsClient {
    async fn query(&self, query: &DnsQuery) -> Result<QueryResult> {
        use tracing::{debug, warn};

        debug!(
            "UdpDnsClient querying {} {} via server {} ({})",
            query.name, query.record_type, self.server.name, self.server.address
        );

        let server_addr = self.parse_address()?;
        debug!("Parsed server address: {}", server_addr);

        let query_bytes = query
            .to_bytes()
            .map_err(|e| anyhow!("Failed to encode query: {}", e))?;
        debug!("Encoded query: {} bytes", query_bytes.len());

        let start = Instant::now();
        let mut response_bytes = match self.send_query(&query_bytes, server_addr, query.id).await {
            Ok(bytes) => bytes,
            Err(e) => {
                warn!("UDP query to {} failed: {}", server_addr, e);
                return Err(e);
            }
        };

        // A truncated UDP answer carries only part of the record set; RFC 1035
        // section 4.2.1 requires retrying over TCP instead of using it as-is.
        if message_is_truncated(&response_bytes) {
            debug!(
                "UDP response from {} is truncated, retrying over TCP",
                server_addr
            );
            response_bytes = self
                .send_query_tcp(&query_bytes, server_addr)
                .await
                .map_err(|e| anyhow!("TCP retry after truncated UDP answer failed: {}", e))?;
        }

        let response_time = start.elapsed();

        debug!(
            "Received response: {} bytes in {:?}",
            response_bytes.len(),
            response_time
        );

        let response = DnsResponse::from_bytes(&response_bytes)
            .map_err(|e| anyhow!("Failed to parse response: {}", e))?;

        debug!(
            "Parsed response: {} answers, code={}",
            response.answers.len(),
            response.response_code
        );

        Ok(QueryResult {
            response,
            response_time_ms: response_time.as_millis() as u64,
            server_id: self.server.id,
            server_name: self.server.name.clone(),
        })
    }

    fn server(&self) -> &UpstreamServer {
        &self.server
    }

    async fn health_check(&self) -> Result<Duration> {
        let start = Instant::now();
        self.query(&health_probe_query()).await?;
        Ok(start.elapsed())
    }
}

/// DoT (DNS over TLS) Client
///
/// Queries upstream DNS servers using DNS over TLS protocol.
/// Supports connection reuse for better performance.
pub struct DotDnsClient {
    server: UpstreamServer,
}

impl DotDnsClient {
    /// Create a new DoT DNS client
    pub fn new(server: UpstreamServer) -> Self {
        Self { server }
    }

    /// Parse the server address with IPv6 support
    /// Supports formats: "dns.google:853", "[2001:4860:4860::8888]:853", "1.1.1.1:853"
    fn parse_address(&self) -> Result<(String, u16)> {
        parse_host_port(&self.server.address, UpstreamProtocol::Dot.default_port())
    }

    /// Create a new TLS connection with IPv6 support
    async fn create_connection(&self, host: &str, port: u16) -> Result<DotConnection> {
        use rustls::pki_types::ServerName;
        use rustls::{ClientConfig, RootCertStore};
        use tokio_rustls::TlsConnector;

        // Format address for connection - use brackets for IPv6
        let addr = if host.contains(':') {
            format!("[{}]:{}", host, port)
        } else {
            format!("{}:{}", host, port)
        };

        // Create TLS config with system root certificates
        let mut root_store = RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        let mut config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        // Advertise the "dot" ALPN token registered by RFC 7858; servers that
        // enforce ALPN reject handshakes without it.
        config.alpn_protocols = vec![b"dot".to_vec()];

        let connector = TlsConnector::from(Arc::new(config));

        // For SNI, use the host directly (without brackets). A bare IP is allowed:
        // rustls builds an IpAddress ServerName, which validates against providers
        // that ship IP SANs (Cloudflare's 1.1.1.1 does). Providers without an IP
        // SAN fail during the handshake, which is reported below.
        let server_name = ServerName::try_from(host.to_string())
            .map_err(|_| anyhow!("Invalid server name: {}", host))?;

        // Connect with timeout
        let stream = timeout(self.server.timeout, TcpStream::connect(&addr))
            .await
            .map_err(|_| anyhow!("Connection timeout to {}", addr))??;

        let tls_stream = timeout(self.server.timeout, connector.connect(server_name, stream))
            .await
            .map_err(|_| anyhow!("TLS handshake timeout to {}", addr))?
            .map_err(|e| {
                anyhow!(
                    "TLS handshake with {} failed: {}. When the address is a bare IP, the \
                 upstream certificate must include a matching IP SAN; otherwise use a hostname",
                    addr,
                    e
                )
            })?;

        Ok(tls_stream)
    }

    /// Run a query on a freshly established connection, keeping it for reuse.
    async fn query_on_new_connection(
        &self,
        host: &str,
        port: u16,
        pool_key: &str,
        query: &DnsQuery,
    ) -> Result<Vec<u8>> {
        let mut conn = self.create_connection(host, port).await?;
        let bytes = self.send_query_on_conn(&mut conn, query).await?;
        dot_pool_put(pool_key, conn).await;
        Ok(bytes)
    }

    /// Send query over an existing connection, returns None if connection is broken
    async fn send_query_on_conn(
        &self,
        conn: &mut DotConnection,
        query: &DnsQuery,
    ) -> Result<Vec<u8>> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // Encode query with length prefix (TCP DNS format)
        let query_bytes = query
            .to_bytes()
            .map_err(|e| anyhow!("Failed to encode query: {}", e))?;
        let len = (query_bytes.len() as u16).to_be_bytes();

        conn.write_all(&len).await?;
        conn.write_all(&query_bytes).await?;
        conn.flush().await?;

        // Read response length
        let mut len_buf = [0u8; 2];
        timeout(self.server.timeout, conn.read_exact(&mut len_buf))
            .await
            .map_err(|_| anyhow!("Read timeout"))??;
        let response_len = u16::from_be_bytes(len_buf) as usize;

        // Read response
        let mut response_bytes = vec![0u8; response_len];
        timeout(self.server.timeout, conn.read_exact(&mut response_bytes))
            .await
            .map_err(|_| anyhow!("Read timeout"))??;

        Ok(response_bytes)
    }
}

#[async_trait]
impl DnsClient for DotDnsClient {
    async fn query(&self, query: &DnsQuery) -> Result<QueryResult> {
        use tracing::debug;

        let (host, port) = self.parse_address()?;
        let pool_key = format!("{}:{}", host, port);

        let start = Instant::now();

        // Try an idle connection first; a failure there is expected (the peer may
        // have closed it while idle) and falls through to a fresh handshake.
        let response_bytes = match dot_pool_take(&pool_key).await {
            Some(mut conn) => match self.send_query_on_conn(&mut conn, query).await {
                Ok(bytes) => {
                    debug!("DoT query succeeded on reused connection to {}", pool_key);
                    dot_pool_put(&pool_key, conn).await;
                    bytes
                }
                Err(e) => {
                    debug!(
                        "DoT reused connection failed: {}, creating new connection",
                        e
                    );
                    self.query_on_new_connection(&host, port, &pool_key, query)
                        .await?
                }
            },
            None => {
                debug!("DoT creating new connection to {}", pool_key);
                self.query_on_new_connection(&host, port, &pool_key, query)
                    .await?
            }
        };

        let response_time = start.elapsed();

        let response = DnsResponse::from_bytes(&response_bytes)
            .map_err(|e| anyhow!("Failed to parse response: {}", e))?;

        Ok(QueryResult {
            response,
            response_time_ms: response_time.as_millis() as u64,
            server_id: self.server.id,
            server_name: self.server.name.clone(),
        })
    }

    fn server(&self) -> &UpstreamServer {
        &self.server
    }

    async fn health_check(&self) -> Result<Duration> {
        let start = Instant::now();
        self.query(&health_probe_query()).await?;
        Ok(start.elapsed())
    }
}

/// DoH (DNS over HTTPS) Client
///
/// Queries upstream DNS servers using DNS over HTTPS protocol.
pub struct DohDnsClient {
    server: UpstreamServer,
    client: reqwest::Client,
}

impl DohDnsClient {
    /// Create a new DoH DNS client
    ///
    /// Returns an error when the HTTP client cannot be built: falling back to a
    /// default client would silently discard the configured timeout, leaving
    /// queries to hang on reqwest's own (much longer) defaults.
    pub fn new(server: UpstreamServer) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(server.timeout)
            .build()
            .map_err(|e| anyhow!("Failed to build DoH HTTP client for {}: {}", server.name, e))?;

        Ok(Self { server, client })
    }

    /// Get the DoH URL
    fn get_url(&self) -> String {
        if self.server.address.starts_with("http://") || self.server.address.starts_with("https://")
        {
            self.server.address.clone()
        } else {
            format!("https://{}/dns-query", self.server.address)
        }
    }
}

#[async_trait]
impl DnsClient for DohDnsClient {
    async fn query(&self, query: &DnsQuery) -> Result<QueryResult> {
        let url = self.get_url();
        let query_bytes = query
            .to_bytes()
            .map_err(|e| anyhow!("Failed to encode query: {}", e))?;

        let start = Instant::now();

        // Use POST method with application/dns-message content type
        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/dns-message")
            .header("Accept", "application/dns-message")
            .body(query_bytes)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "DoH query failed with status: {}",
                response.status()
            ));
        }

        // Reject oversized bodies before buffering them: a DNS message cannot
        // exceed 65535 bytes, and an unbounded read lets a faulty or hostile
        // upstream drive our memory use.
        if let Some(declared) = response.content_length() {
            if declared > MAX_DNS_MESSAGE as u64 {
                return Err(anyhow!(
                    "DoH response from {} declares {} bytes, over the {} byte DNS limit",
                    self.server.name,
                    declared,
                    MAX_DNS_MESSAGE
                ));
            }
        }

        let response_bytes = read_body_capped(response, MAX_DNS_MESSAGE).await?;
        let response_time = start.elapsed();

        let dns_response = DnsResponse::from_bytes(&response_bytes)
            .map_err(|e| anyhow!("Failed to parse response: {}", e))?;

        Ok(QueryResult {
            response: dns_response,
            response_time_ms: response_time.as_millis() as u64,
            server_id: self.server.id,
            server_name: self.server.name.clone(),
        })
    }

    fn server(&self) -> &UpstreamServer {
        &self.server
    }

    async fn health_check(&self) -> Result<Duration> {
        let start = Instant::now();
        self.query(&health_probe_query()).await?;
        Ok(start.elapsed())
    }
}

/// DoQ (DNS over QUIC) Client
///
/// Queries upstream DNS servers using DNS over QUIC protocol.
pub struct DoqDnsClient {
    server: UpstreamServer,
    connections: Vec<Arc<tokio::sync::RwLock<Option<quinn::Connection>>>>,
    connect_locks: Vec<Arc<tokio::sync::Mutex<()>>>,
    index: AtomicUsize,
}

impl DoqDnsClient {
    /// Create a new DoQ DNS client
    pub fn new(server: UpstreamServer) -> Self {
        let mut connections = Vec::with_capacity(ENDPOINT_POOL_SIZE);
        let mut connect_locks = Vec::with_capacity(ENDPOINT_POOL_SIZE);

        for _ in 0..ENDPOINT_POOL_SIZE {
            connections.push(Arc::new(tokio::sync::RwLock::new(None)));
            connect_locks.push(Arc::new(tokio::sync::Mutex::new(())));
        }

        Self {
            server,
            connections,
            connect_locks,
            index: AtomicUsize::new(0),
        }
    }

    /// Parse the server address and resolve hostname if needed
    /// Prefers IPv4 addresses over IPv6 for better compatibility
    ///
    /// Returns (SocketAddr, SNI) where SNI is the original host (IP or hostname)
    async fn resolve_address(&self) -> Result<(SocketAddr, String)> {
        let (host, port) =
            parse_host_port(&self.server.address, UpstreamProtocol::Doq.default_port())?;

        // An IP literal needs no lookup.
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            return Ok((SocketAddr::new(ip, port), host));
        }

        // It's a hostname, resolve it - prefer IPv4
        use tokio::net::lookup_host;
        let addr_str = format!("{}:{}", host, port);
        let addrs: Vec<SocketAddr> = lookup_host(&addr_str)
            .await
            .map_err(|e| anyhow!("Failed to resolve hostname {}: {}", host, e))?
            .collect();

        if addrs.is_empty() {
            return Err(anyhow!("No addresses found for {}", host));
        }

        // Prefer IPv4 addresses
        let addr = addrs
            .iter()
            .find(|a| a.is_ipv4())
            .or_else(|| addrs.first())
            .cloned()
            .ok_or_else(|| anyhow!("No addresses found for {}", host))?;

        Ok((addr, host))
    }
}

#[async_trait]
impl DnsClient for DoqDnsClient {
    async fn query(&self, query: &DnsQuery) -> Result<QueryResult> {
        use tracing::debug;

        let (addr, sni_host) = self.resolve_address().await?;

        // Loop to allow one retry if cached connection fails
        let mut attempts = 0;

        // Select a connection slot using round-robin
        let idx = self.index.fetch_add(1, Ordering::Relaxed) % ENDPOINT_POOL_SIZE;
        let connection_slot = &self.connections[idx];
        let connect_lock = &self.connect_locks[idx];

        loop {
            attempts += 1;
            let is_retry = attempts > 1;

            // 1. Get connection (cached or new)
            // On retry, force new connection
            let is_healthy = |c: &quinn::Connection| c.close_reason().is_none();

            // 1. Get connection (cached) - Fast path
            let connection = if is_retry {
                debug!("DoQ retry: forcing new connection to {}", addr);
                None
            } else {
                let guard = connection_slot.read().await;
                guard.clone().filter(|c| is_healthy(c))
            };

            let connection = if let Some(conn) = connection {
                debug!("DoQ reusing existing connection to {} (slot {})", addr, idx);
                conn
            } else {
                // 2. Slow path: Acquire lock to serialize connection attempts
                let _lock = connect_lock.lock().await;

                // 3. Double check
                let guard = connection_slot.read().await;
                if let Some(conn) = guard.clone().filter(|c| is_healthy(c)) {
                    drop(guard);
                    debug!(
                        "DoQ reused connection created by another thread to {} (slot {})",
                        addr, idx
                    );
                    conn
                } else {
                    drop(guard);

                    debug!(
                        "DoQ creating new connection to {} (SNI: {}, slot {})",
                        addr, sni_host, idx
                    );

                    // Get or create cached endpoint
                    let endpoint = get_quic_endpoint(QuicProtocol::Doq, addr.is_ipv6())?;
                    let connect_sni = sni_host.as_str();

                    match timeout(self.server.timeout, endpoint.connect(addr, connect_sni)?).await {
                        Ok(Ok(conn)) => {
                            debug!("DoQ connection established to {} (slot {})", addr, idx);
                            // Update cache
                            let mut guard = connection_slot.write().await;
                            *guard = Some(conn.clone());
                            conn
                        }
                        Ok(Err(e)) => return Err(anyhow!("Connection failed: {}", e)),
                        Err(_) => return Err(anyhow!("Connection timeout")),
                    }
                }
            };

            // 2. Perform Query
            let query_result = async {
                // Open stream
                let (mut send, mut recv) = timeout(self.server.timeout, connection.open_bi())
                    .await
                    .map_err(|_| anyhow!("Stream open timeout"))??;

                // Encode query
                let doq_query = DnsQuery::with_id(0, &query.name, query.record_type.clone());
                let query_bytes = doq_query
                    .to_bytes()
                    .map_err(|e| anyhow!("Failed to encode query: {}", e))?;
                let len = (query_bytes.len() as u16).to_be_bytes();

                let start = Instant::now();
                debug!("DoQ sending {} bytes query", query_bytes.len());

                send.write_all(&len).await?;
                send.write_all(&query_bytes).await?;
                send.finish()
                    .map_err(|e| anyhow!("Failed to finish stream: {}", e))?;

                // Read response
                let mut len_buf = [0u8; 2];
                timeout(self.server.timeout, recv.read_exact(&mut len_buf))
                    .await
                    .map_err(|_| anyhow!("read timeout"))??;

                let response_len = u16::from_be_bytes(len_buf) as usize;

                if response_len == 0 || response_len > 65535 {
                    return Err(anyhow!("Invalid response length: {}", response_len));
                }

                let mut response_bytes = vec![0u8; response_len];
                recv.read_exact(&mut response_bytes)
                    .await
                    .map_err(|e| anyhow!("Failed to read response body: {}", e))?;

                let response_time = start.elapsed();
                let response = DnsResponse::from_bytes(&response_bytes)
                    .map_err(|e| anyhow!("Failed to parse response: {}", e))?;

                Ok(QueryResult {
                    response,
                    response_time_ms: response_time.as_millis() as u64,
                    server_id: self.server.id,
                    server_name: self.server.name.clone(),
                })
            }
            .await;

            match query_result {
                Ok(res) => return Ok(res),
                Err(e) => {
                    // If we failed on a possibly reused connection, retry once with a fresh one
                    if !is_retry {
                        // Check if it was a connection error that warrants retry
                        // (Assume yes for most errors if we were using a cached conn)
                        debug!("DoQ query failed on cached connection: {}, retrying...", e);

                        // Clear cache to ensure next loop gets a fresh one
                        let mut guard = connection_slot.write().await;
                        *guard = None;
                        continue;
                    }
                    return Err(e);
                }
            }
        }
    }

    fn server(&self) -> &UpstreamServer {
        &self.server
    }

    async fn health_check(&self) -> Result<Duration> {
        let start = Instant::now();
        self.query(&health_probe_query()).await?;
        Ok(start.elapsed())
    }
}

/// Certificate verifier that accepts any certificate (for IP-based connections)
#[derive(Debug)]
struct NoVerifier;

impl rustls::client::danger::ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
        ]
    }
}

/// DoH3 (DNS over HTTP/3) Client
///
/// Queries upstream DNS servers using DNS over HTTP/3 protocol.
/// Note: Due to h3 library limitations, each query creates a new H3 session.
/// The QUIC endpoint is reused for better performance.
pub struct Doh3DnsClient {
    server: UpstreamServer,
    connections: Vec<Arc<tokio::sync::RwLock<Option<Doh3Session>>>>,
    connect_locks: Vec<Arc<tokio::sync::Mutex<()>>>,
    index: AtomicUsize,
}

/// A cached HTTP/3 session together with the QUIC connection carrying it.
///
/// The QUIC handle is kept so liveness can be checked before reuse. Holding
/// only the `SendRequest` gave no way to tell a live session from one whose
/// connection had already timed out, so every idle period cost a failed request
/// plus a retry.
#[derive(Clone)]
struct Doh3Session {
    connection: quinn::Connection,
    sender: H3SendRequest,
}

impl Doh3Session {
    fn is_alive(&self) -> bool {
        self.connection.close_reason().is_none()
    }
}

impl Doh3DnsClient {
    /// Create a new DoH3 DNS client
    pub fn new(server: UpstreamServer) -> Self {
        let mut connections = Vec::with_capacity(ENDPOINT_POOL_SIZE);
        let mut connect_locks = Vec::with_capacity(ENDPOINT_POOL_SIZE);

        for _ in 0..ENDPOINT_POOL_SIZE {
            connections.push(Arc::new(tokio::sync::RwLock::new(None)));
            connect_locks.push(Arc::new(tokio::sync::Mutex::new(())));
        }

        Self {
            server,
            connections,
            connect_locks,
            index: AtomicUsize::new(0),
        }
    }

    /// Get the DoH3 URL and parse host/port with IPv6 support
    /// Returns (sni_host, host, port, path)
    /// Supports formats:
    /// - https://dns.google/dns-query
    /// - https://[2001:4860:4860::8888]/dns-query
    /// - [2001:4860:4860::8888]:443
    /// - dns.google:443
    fn parse_url(&self) -> Result<(String, String, u16, String)> {
        let addr = &self.server.address;

        // Parse URL format: https://host:port/path or host:port or host
        if addr.starts_with("https://") {
            let without_scheme = &addr[8..];

            // Find path (starts with /)
            let (host_port, path) = if let Some(slash_pos) = without_scheme.find('/') {
                (&without_scheme[..slash_pos], &without_scheme[slash_pos..])
            } else {
                (without_scheme, "/dns-query")
            };

            let (host, port) = parse_host_port(host_port, HTTPS_PORT)?;

            Ok((host.clone(), host, port, path.to_string()))
        } else {
            // Non-URL format: use parse_host_port helper
            let (host, port) = parse_host_port(addr, HTTPS_PORT)?;
            Ok((host.clone(), host, port, "/dns-query".to_string()))
        }
    }

    /// Resolve hostname to socket address with proper IPv6 formatting
    async fn resolve_address(&self, host: &str, port: u16) -> Result<std::net::SocketAddr> {
        // Try to parse as IP address first
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            return Ok(std::net::SocketAddr::new(ip, port));
        }

        // Resolve hostname - format with brackets for IPv6 compatibility in lookup
        use tokio::net::lookup_host;
        let addr_str = if host.contains(':') {
            format!("[{}]:{}", host, port)
        } else {
            format!("{}:{}", host, port)
        };
        let addrs: Vec<std::net::SocketAddr> = lookup_host(&addr_str)
            .await
            .map_err(|e| anyhow!("Failed to resolve hostname {}: {}", host, e))?
            .collect();

        // Prefer IPv4 for better compatibility, but accept IPv6
        addrs
            .iter()
            .find(|a| a.is_ipv4())
            .or_else(|| addrs.first())
            .cloned()
            .ok_or_else(|| anyhow!("No addresses found for {}", host))
    }
}

#[async_trait]
impl DnsClient for Doh3DnsClient {
    async fn query(&self, query: &DnsQuery) -> Result<QueryResult> {
        use bytes::{Buf, Bytes};
        use http::{Method, Request};
        use tracing::debug;

        let (sni_host, host, port, path) = self.parse_url()?;
        let addr = self.resolve_address(&host, port).await?;

        debug!(
            "DoH3 connecting to {} (SNI: {}, path: {})",
            addr, sni_host, path
        );

        let idx = self.index.fetch_add(1, Ordering::Relaxed) % ENDPOINT_POOL_SIZE;
        let connection_slot = &self.connections[idx];
        let connect_lock = &self.connect_locks[idx];

        // START TIMER
        let start = std::time::Instant::now();

        // ENCODE QUERY
        let query_bytes = query
            .to_bytes()
            .map_err(|e| anyhow!("Failed to encode query: {}", e))?;

        let mut attempts = 0;

        // Loop to get a request stream
        let mut request_stream = loop {
            attempts += 1;
            let is_retry = attempts > 1;

            // 1. Get connection (cached), skipping sessions whose QUIC
            //    connection has already been closed.
            let mut session = if is_retry {
                None
            } else {
                let guard = connection_slot.read().await;
                guard.clone().filter(|s| s.is_alive())
            };

            // 2. If no usable cached connection, create new one
            if session.is_none() {
                let _lock = connect_lock.lock().await;

                // Double check
                let guard = connection_slot.read().await;
                if let Some(s) = guard.clone().filter(|s| s.is_alive()) {
                    session = Some(s);
                } else {
                    drop(guard);

                    // Get or create cached endpoint
                    let endpoint = get_quic_endpoint(QuicProtocol::Doh3, addr.is_ipv6())?;
                    let connect_sni = sni_host.as_str();

                    debug!("DoH3 creating new connection to {} (slot {})", addr, idx);

                    // Create new QUIC connection
                    let connection =
                        timeout(self.server.timeout, endpoint.connect(addr, connect_sni)?)
                            .await
                            .map_err(|_| anyhow!("Connection timeout"))??;

                    debug!("DoH3 QUIC connection established (slot {})", idx);

                    // Create HTTP/3 session
                    let quinn_conn = h3_quinn::Connection::new(connection.clone());
                    let (mut driver, new_sender) = h3::client::new(quinn_conn)
                        .await
                        .map_err(|e| anyhow!("Failed to create H3 connection: {}", e))?;

                    // Drive the connection until it closes. The reason is logged
                    // rather than discarded so unexpected teardowns are visible.
                    let driver_target = format!("{} ({})", self.server.name, addr);
                    tokio::spawn(async move {
                        let reason = futures::future::poll_fn(|cx| driver.poll_close(cx)).await;
                        debug!("DoH3 connection to {} closed: {}", driver_target, reason);
                    });

                    let new_session = Doh3Session {
                        connection,
                        sender: new_sender,
                    };

                    // Update cache
                    let mut guard = connection_slot.write().await;
                    *guard = Some(new_session.clone());
                    session = Some(new_session);
                }
            }

            // At this point we have a live session
            let mut sender = session
                .expect("session is populated by the branch above")
                .sender;

            // Build HTTP/3 request
            let uri = format!("https://{}:{}{}", sni_host, port, path);
            let request = Request::builder()
                .method(Method::POST)
                .uri(&uri)
                .header("content-type", "application/dns-message")
                .header("accept", "application/dns-message")
                .header("user-agent", "fluxdns/1.1.7")
                .header("content-length", query_bytes.len().to_string())
                .body(())
                .map_err(|e| anyhow!("Failed to build request: {}", e))?;

            debug!("DoH3 sending request to {} (slot {})", uri, idx);

            // Send request
            match sender.send_request(request).await {
                Ok(stream) => break stream,
                Err(e) => {
                    // If cached connection failed, clear it and retry
                    if !is_retry {
                        debug!("DoH3 cached connection failed: {}, retrying...", e);
                        let mut guard = connection_slot.write().await;
                        *guard = None;
                        continue;
                    }
                    return Err(anyhow!("Failed to send request: {}", e));
                }
            }
        };

        // Send body
        request_stream
            .send_data(Bytes::from(query_bytes))
            .await
            .map_err(|e| anyhow!("Failed to send body: {}", e))?;

        request_stream
            .finish()
            .await
            .map_err(|e| anyhow!("Failed to finish request: {}", e))?;

        // Receive response
        let response = request_stream
            .recv_response()
            .await
            .map_err(|e| anyhow!("Failed to receive response: {}", e))?;

        debug!("DoH3 response status: {}", response.status());

        if !response.status().is_success() {
            return Err(anyhow!(
                "DoH3 query failed with status: {}",
                response.status()
            ));
        }

        // Read response body, bounded by the DNS message limit so a runaway
        // peer cannot grow this buffer without limit.
        let mut response_bytes = Vec::new();
        while let Some(mut chunk) = request_stream
            .recv_data()
            .await
            .map_err(|e| anyhow!("Failed to read response body: {}", e))?
        {
            while chunk.has_remaining() {
                use bytes::Buf; // Import Buf trait
                let bytes = chunk.chunk();
                if response_bytes.len() + bytes.len() > MAX_DNS_MESSAGE {
                    return Err(anyhow!(
                        "DoH3 response exceeds the {} byte DNS limit",
                        MAX_DNS_MESSAGE
                    ));
                }
                response_bytes.extend_from_slice(bytes);
                chunk.advance(bytes.len());
            }
        }

        let response_time = start.elapsed();
        debug!(
            "DoH3 received {} bytes in {:?}",
            response_bytes.len(),
            response_time
        );

        let dns_response = DnsResponse::from_bytes(&response_bytes)
            .map_err(|e| anyhow!("Failed to parse response: {}", e))?;

        Ok(QueryResult {
            response: dns_response,
            response_time_ms: response_time.as_millis() as u64,
            server_id: self.server.id,
            server_name: self.server.name.clone(),
        })
    }

    fn server(&self) -> &UpstreamServer {
        &self.server
    }

    async fn health_check(&self) -> Result<Duration> {
        let start = Instant::now();
        self.query(&health_probe_query()).await?;
        Ok(start.elapsed())
    }
}

/// Create a DNS client for the given upstream server
pub fn create_client(server: UpstreamServer) -> Result<Box<dyn DnsClient>> {
    let client: Box<dyn DnsClient> = match server.protocol {
        UpstreamProtocol::Udp => Box::new(UdpDnsClient::new(server)),
        UpstreamProtocol::Dot => Box::new(DotDnsClient::new(server)),
        UpstreamProtocol::Doh => Box::new(DohDnsClient::new(server)?),
        UpstreamProtocol::Doq => Box::new(DoqDnsClient::new(server)),
        UpstreamProtocol::Doh3 => Box::new(Doh3DnsClient::new(server)),
    };
    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_udp_client() {
        let server = UpstreamServer::new(1, "Test", "8.8.8.8:53", UpstreamProtocol::Udp, 5000);
        let client = create_client(server.clone()).unwrap();
        assert_eq!(client.server().protocol, UpstreamProtocol::Udp);
    }

    #[test]
    fn test_create_dot_client() {
        let server = UpstreamServer::new(1, "Test", "dns.google:853", UpstreamProtocol::Dot, 5000);
        let client = create_client(server.clone()).unwrap();
        assert_eq!(client.server().protocol, UpstreamProtocol::Dot);
    }

    #[test]
    fn test_create_doh_client() {
        let server = UpstreamServer::new(
            1,
            "Test",
            "https://dns.google/dns-query",
            UpstreamProtocol::Doh,
            5000,
        );
        let client = create_client(server.clone()).unwrap();
        assert_eq!(client.server().protocol, UpstreamProtocol::Doh);
    }

    #[test]
    fn test_create_doq_client() {
        let server = UpstreamServer::new(
            1,
            "Test",
            "dns.adguard.com:853",
            UpstreamProtocol::Doq,
            5000,
        );
        let client = create_client(server.clone()).unwrap();
        assert_eq!(client.server().protocol, UpstreamProtocol::Doq);
    }

    #[test]
    fn test_create_doh3_client() {
        let server = UpstreamServer::new(
            1,
            "Test",
            "https://dns.adguard-dns.com/dns-query",
            UpstreamProtocol::Doh3,
            5000,
        );
        let client = create_client(server.clone()).unwrap();
        assert_eq!(client.server().protocol, UpstreamProtocol::Doh3);
    }

    /// Build a response carrying `address` as its single A record.
    fn build_response(query: &DnsQuery, id: u16, address: std::net::Ipv4Addr) -> Vec<u8> {
        use crate::dns::message::{DnsRecordData, DnsResponse};

        let mut response = DnsResponse::new(id);
        response.add_answer(DnsRecordData::a(&query.name, address, 300));
        response.to_bytes(query).unwrap()
    }

    /// Build a truncated response: the TC bit is set and the record set is
    /// dropped, which is what a real upstream sends when the answer does not
    /// fit. Keeping the records here would let the test pass even if the client
    /// ignored the TC bit entirely.
    fn build_truncated_response(query: &DnsQuery, id: u16) -> Vec<u8> {
        use crate::dns::message::DnsResponse;

        let bytes = DnsResponse::new(id).to_bytes(query).unwrap();

        // Set the TC bit (header byte 2, bit 1) so the client must retry.
        let mut bytes = bytes;
        bytes[2] |= 0b0000_0010;
        bytes
    }

    fn udp_client_for(addr: SocketAddr) -> UdpDnsClient {
        UdpDnsClient::new(UpstreamServer::new(
            1,
            "fake",
            addr.to_string(),
            UpstreamProtocol::Udp,
            2000,
        ))
    }

    /// A response whose transaction ID does not match the query must be
    /// discarded: accepting it is the basic cache-poisoning vector.
    #[tokio::test]
    async fn test_udp_client_ignores_mismatched_transaction_id() {
        let upstream = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream.local_addr().unwrap();

        tokio::spawn(async move {
            let mut buf = vec![0u8; MAX_UDP_DATAGRAM];
            let (len, from) = upstream.recv_from(&mut buf).await.unwrap();
            let query = DnsQuery::from_bytes(&buf[..len]).unwrap();

            // First answer carries the wrong ID and must be ignored. It also
            // carries a distinct address so accepting it would be visible.
            let forged = build_response(
                &query,
                query.id.wrapping_add(1),
                std::net::Ipv4Addr::new(198, 51, 100, 66),
            );
            upstream.send_to(&forged, from).await.unwrap();

            // The genuine answer follows.
            let genuine = build_response(&query, query.id, std::net::Ipv4Addr::new(203, 0, 113, 7));
            upstream.send_to(&genuine, from).await.unwrap();
        });

        let client = udp_client_for(upstream_addr);
        let query = DnsQuery::with_id(4242, "example.com", crate::dns::message::RecordType::A);

        let result = client.query(&query).await.unwrap();
        assert_eq!(result.response.id, 4242);
        assert_eq!(result.response.answers.len(), 1);
        // The forged datagram's address must not surface.
        assert_eq!(result.response.answers[0].value, "203.0.113.7");
    }

    /// A truncated UDP answer only holds part of the record set, so RFC 1035
    /// requires the query to be reissued over TCP rather than used as-is.
    #[tokio::test]
    async fn test_udp_client_retries_over_tcp_when_truncated() {
        let upstream_udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream_udp.local_addr().unwrap();
        // DNS uses the same port number for UDP and TCP.
        let upstream_tcp = tokio::net::TcpListener::bind(upstream_addr).await.unwrap();

        tokio::spawn(async move {
            let mut buf = vec![0u8; MAX_UDP_DATAGRAM];
            let (len, from) = upstream_udp.recv_from(&mut buf).await.unwrap();
            let query = DnsQuery::from_bytes(&buf[..len]).unwrap();
            let truncated = build_truncated_response(&query, query.id);
            upstream_udp.send_to(&truncated, from).await.unwrap();
        });

        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};

            let (mut stream, _) = upstream_tcp.accept().await.unwrap();

            let mut len_buf = [0u8; 2];
            stream.read_exact(&mut len_buf).await.unwrap();
            let mut query_buf = vec![0u8; u16::from_be_bytes(len_buf) as usize];
            stream.read_exact(&mut query_buf).await.unwrap();

            let query = DnsQuery::from_bytes(&query_buf).unwrap();
            let full = build_response(&query, query.id, std::net::Ipv4Addr::new(203, 0, 113, 7));

            stream
                .write_all(&(full.len() as u16).to_be_bytes())
                .await
                .unwrap();
            stream.write_all(&full).await.unwrap();
            stream.flush().await.unwrap();
        });

        let client = udp_client_for(upstream_addr);
        let query = DnsQuery::with_id(777, "example.com", crate::dns::message::RecordType::A);

        let result = client.query(&query).await.unwrap();

        // The TCP answer is the complete one, so the record survives.
        assert_eq!(result.response.answers.len(), 1);
        assert_eq!(result.response.answers[0].value, "203.0.113.7");
    }

    #[test]
    fn test_doh_url_generation() {
        let server = UpstreamServer::new(1, "Test", "dns.google", UpstreamProtocol::Doh, 5000);
        let client = DohDnsClient::new(server).unwrap();
        assert_eq!(client.get_url(), "https://dns.google/dns-query");

        let server2 = UpstreamServer::new(
            2,
            "Test2",
            "https://cloudflare-dns.com/dns-query",
            UpstreamProtocol::Doh,
            5000,
        );
        let client2 = DohDnsClient::new(server2).unwrap();
        assert_eq!(client2.get_url(), "https://cloudflare-dns.com/dns-query");
    }

    #[test]
    fn test_parse_host_port_accepts_supported_forms() {
        assert_eq!(
            parse_host_port("1.1.1.1:53", 53).unwrap(),
            ("1.1.1.1".to_string(), 53)
        );
        assert_eq!(
            parse_host_port("1.1.1.1", 53).unwrap(),
            ("1.1.1.1".to_string(), 53)
        );
        assert_eq!(
            parse_host_port("dns.google:853", 53).unwrap(),
            ("dns.google".to_string(), 853)
        );
        // Brackets are stripped so the host can be used directly as an SNI name.
        assert_eq!(
            parse_host_port("[2001:4860:4860::8888]:853", 53).unwrap(),
            ("2001:4860:4860::8888".to_string(), 853)
        );
        assert_eq!(
            parse_host_port("[::1]", 853).unwrap(),
            ("::1".to_string(), 853)
        );
        // A bare IPv6 literal has colons but no port.
        assert_eq!(
            parse_host_port("2001:4860:4860::8888", 853).unwrap(),
            ("2001:4860:4860::8888".to_string(), 853)
        );
    }

    #[test]
    fn test_parse_host_port_rejects_bad_port_instead_of_defaulting() {
        // Silently falling back to the default would connect the user to a
        // different endpoint than the one they configured.
        for address in ["[::1]:99999", "dns.google:99999", "1.1.1.1:http"] {
            let err =
                parse_host_port(address, 53).expect_err(&format!("{} should be rejected", address));
            assert!(
                err.to_string().contains("Invalid port"),
                "unexpected error for {}: {}",
                address,
                err
            );
        }
    }

    #[test]
    fn test_parse_host_port_rejects_unterminated_bracket() {
        let err = parse_host_port("[::1:853", 53).unwrap_err();
        assert!(err.to_string().contains("Unterminated IPv6 bracket"));
    }

    #[test]
    fn test_doh3_parse_url_forms() {
        let parse = |address: &str| {
            Doh3DnsClient::new(UpstreamServer::new(
                1,
                "Test",
                address,
                UpstreamProtocol::Doh3,
                5000,
            ))
            .parse_url()
            .unwrap()
        };

        let (sni, host, port, path) = parse("https://dns.google/dns-query");
        assert_eq!(
            (sni.as_str(), host.as_str(), port, path.as_str()),
            ("dns.google", "dns.google", 443, "/dns-query")
        );

        // No path in the URL falls back to the RFC 8484 well-known path.
        let (_, host, port, path) = parse("https://dns.google");
        assert_eq!(
            (host.as_str(), port, path.as_str()),
            ("dns.google", 443, "/dns-query")
        );

        let (_, host, port, path) = parse("https://[2001:4860:4860::8888]:8443/query");
        assert_eq!(
            (host.as_str(), port, path.as_str()),
            ("2001:4860:4860::8888", 8443, "/query")
        );

        // Bare host:port is accepted for convenience.
        let (_, host, port, path) = parse("dns.google:8443");
        assert_eq!(
            (host.as_str(), port, path.as_str()),
            ("dns.google", 8443, "/dns-query")
        );
    }

    #[test]
    fn test_doq_default_port_is_applied() {
        let client = DoqDnsClient::new(UpstreamServer::new(
            1,
            "Test",
            "dns.adguard.com",
            UpstreamProtocol::Doq,
            5000,
        ));
        let (_, port) =
            parse_host_port(&client.server.address, UpstreamProtocol::Doq.default_port()).unwrap();
        assert_eq!(port, 853);
    }
}
