//! Listener Manager
//!
//! Manages the lifecycle of DNS server listeners (UDP, DoT, DoH, DoQ).
//! Supports dynamic starting, stopping, and restarting of listeners without application restart.

use chrono::Local;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::AbortHandle;

use super::privileged_udp::PrivilegedUdpForwarderLease;
use tracing::{error, info, warn};

use crate::dns::server::{
    Doh3DnsServer, DohDnsServer, DoqDnsServer, DotDnsServer, TlsConfig, UdpDnsServer, ALPN_HTTP,
};
use crate::dns::DnsResolver;
use crate::infrastructure::repository::Database;

/// Listener Manager
///
/// Handles spawning and aborting of listener tasks.
#[derive(Clone)]
pub struct ListenerManager {
    db: Arc<Database>,
    resolver: Arc<DnsResolver>,
    /// Running tasks by protocol name.
    tasks: Arc<RwLock<HashMap<String, AbortHandle>>>,
    /// Privileged port leases keyed by protocol. Dropping a lease asks the
    /// authorized helper to stop.
    privileged_udp_leases: Arc<RwLock<HashMap<String, PrivilegedUdpForwarderLease>>>,
    control_directory: Arc<PathBuf>,
}

impl ListenerManager {
    /// Create a new ListenerManager
    pub fn new(db: Arc<Database>, resolver: Arc<DnsResolver>, control_directory: PathBuf) -> Self {
        Self {
            db,
            resolver,
            tasks: Arc::new(RwLock::new(HashMap::new())),
            privileged_udp_leases: Arc::new(RwLock::new(HashMap::new())),
            control_directory: Arc::new(control_directory),
        }
    }

    /// Write the configured certificate and key to disk so the TLS servers can
    /// load them by path.
    ///
    /// A missing certificate is reported instead of tolerated: an encrypted
    /// transport cannot be served without one, and silently ignoring a failed
    /// write left the listener starting against a stale or absent file.
    fn materialize_tls_config(
        &self,
        protocol: &str,
        listener: &crate::infrastructure::repository::ServerListener,
    ) -> anyhow::Result<TlsConfig> {
        let (Some(cert), Some(key)) = (listener.tls_cert.as_ref(), listener.tls_key.as_ref())
        else {
            return Err(anyhow::anyhow!(
                "{} listener requires both a TLS certificate and a private key",
                protocol.to_uppercase()
            ));
        };

        let dir = std::env::temp_dir();
        let cert_path = dir.join(format!("fluxdns_{}_cert.pem", protocol));
        let key_path = dir.join(format!("fluxdns_{}_key.pem", protocol));

        std::fs::write(&cert_path, cert).map_err(|e| {
            anyhow::anyhow!(
                "Failed to write {} certificate to {:?}: {}",
                protocol,
                cert_path,
                e
            )
        })?;
        std::fs::write(&key_path, key).map_err(|e| {
            anyhow::anyhow!(
                "Failed to write {} private key to {:?}: {}",
                protocol,
                key_path,
                e
            )
        })?;

        Ok(TlsConfig::new(
            cert_path.to_string_lossy().to_string(),
            key_path.to_string_lossy().to_string(),
        ))
    }

    /// Report a successful listener start on both the log and the console.
    fn announce_started(label: &str, addr: SocketAddr) {
        let msg = format!("✅ {} listener started on {}", label, addr);
        info!("{}", msg);
        println!("{} {}", Local::now().format("%Y-%m-%d %H:%M:%S"), msg);
    }

    /// Start all enabled listeners from database
    pub async fn start_all_enabled(&self) {
        info!("Starting all enabled listeners...");
        let listeners = match self.db.server_listeners().list_enabled().await {
            Ok(list) => list,
            Err(e) => {
                error!("Failed to load listeners from database: {}", e);
                return;
            }
        };

        for listener in listeners {
            if let Err(e) = self.start_listener(&listener.protocol).await {
                error!("Failed to start {} listener: {}", listener.protocol, e);
                if let Err(rollback_error) = self.disable_listener(&listener.protocol).await {
                    error!(
                        "Failed to disable {} after startup failure: {}",
                        listener.protocol, rollback_error
                    );
                }
            }
        }
    }

    async fn disable_listener(&self, protocol: &str) -> anyhow::Result<()> {
        let update = crate::infrastructure::repository::UpdateServerListener {
            enabled: Some(false),
            ..Default::default()
        };
        let updated = self.db.server_listeners().update(protocol, update).await?;
        if updated.is_none() {
            anyhow::bail!("Listener {} no longer exists", protocol);
        }
        Ok(())
    }

    /// Start a specific listener by protocol
    pub async fn start_listener(&self, protocol: &str) -> anyhow::Result<()> {
        // Double check if already running
        if self.is_running(protocol).await {
            warn!("Listener {} is already running, restarting...", protocol);
            self.stop_listener(protocol).await;
        }

        // Fetch config
        let listener = match self.db.server_listeners().get_by_protocol(protocol).await {
            Ok(Some(l)) => l,
            Ok(None) => {
                let err = format!("Listener {} not found in database", protocol);
                error!("{}", err);
                return Err(anyhow::anyhow!(err));
            }
            Err(e) => {
                let err = format!("Failed to fetch listener config for {}: {}", protocol, e);
                error!("{}", err);
                return Err(anyhow::anyhow!(err));
            }
        };

        // NOTE: Removed enabled check here because the caller (listeners.rs)
        // has already verified the enabled state from the database update response.
        // Re-reading from DB here could get stale data due to transaction timing.

        let bind_addr = format!("{}:{}", listener.bind_address, listener.port);
        let addr: SocketAddr = match bind_addr.parse() {
            Ok(a) => a,
            Err(e) => {
                let err = format!(
                    "Invalid bind address for {}: {} - {}",
                    protocol, bind_addr, e
                );
                error!("{}", err);
                return Err(anyhow::anyhow!(err));
            }
        };

        let resolver = self.resolver.clone();

        info!("Starting {} listener on {}", protocol, addr);

        let handle = match protocol {
            "udp" => {
                let (server, privileged_lease) = self.start_udp_server(addr, resolver).await?;
                Self::announce_started("UDP", addr);
                let server = Arc::new(server);
                let task = tokio::spawn(async move {
                    if let Err(e) = server.run().await {
                        error!("UDP DNS server error: {}", e);
                    }
                    info!("UDP listener stopped");
                });
                if let Some(lease) = privileged_lease {
                    self.privileged_udp_leases
                        .write()
                        .await
                        .insert(protocol.to_string(), lease);
                }
                task.abort_handle()
            }
            "dot" => {
                let tls_config = self.materialize_tls_config(protocol, &listener)?;

                match DotDnsServer::new(addr, tls_config, resolver).await {
                    Ok(server) => {
                        Self::announce_started("DoT", addr);
                        let task = tokio::spawn(async move {
                            if let Err(e) = server.run().await {
                                error!("DoT DNS server error: {}", e);
                            }
                            info!("DoT listener stopped");
                        });
                        task.abort_handle()
                    }
                    Err(e) => {
                        error!("Failed to start DoT server: {}", e);
                        return Err(e);
                    }
                }
            }
            "doh" => {
                // Check if TLS, if not, fail because DoH means HTTPS
                let cert_pem = match &listener.tls_cert {
                    Some(c) => c.clone(),
                    None => {
                        let err = "DoH listener requires TLS certificate";
                        error!("{}", err);
                        return Err(anyhow::anyhow!(err));
                    }
                };
                let key_pem = match &listener.tls_key {
                    Some(k) => k.clone(),
                    None => {
                        let err = "DoH listener requires TLS private key";
                        error!("{}", err);
                        return Err(anyhow::anyhow!(err));
                    }
                };

                // Parse certificates
                let certs: Vec<rustls::pki_types::CertificateDer<'static>> =
                    rustls_pemfile::certs(&mut cert_pem.as_bytes())
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| anyhow::anyhow!("Failed to parse certificate: {}", e))?;

                if certs.is_empty() {
                    let err = "No valid certificates found in PEM";
                    error!("{}", err);
                    return Err(anyhow::anyhow!(err));
                }

                // Parse private key
                let key = rustls_pemfile::private_key(&mut key_pem.as_bytes())
                    .map_err(|e| anyhow::anyhow!("Failed to parse private key: {}", e))?
                    .ok_or_else(|| anyhow::anyhow!("No private key found in PEM"))?;

                // Build rustls config
                let mut tls_config = rustls::ServerConfig::builder()
                    .with_no_client_auth()
                    .with_single_cert(certs, key)
                    .map_err(|e| anyhow::anyhow!("Failed to build TLS config: {}", e))?;

                // Advertise the HTTP versions this listener actually serves;
                // clients that negotiate ALPN abort when the list is empty.
                tls_config.alpn_protocols = ALPN_HTTP.iter().map(|p| p.to_vec()).collect();

                let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls_config));

                // Bind TCP listener first
                let tcp_listener = match tokio::net::TcpListener::bind(addr).await {
                    Ok(l) => l,
                    Err(e) => {
                        error!("Failed to bind DoH address {}: {}", addr, e);
                        return Err(anyhow::anyhow!(e));
                    }
                };

                let server = DohDnsServer::new(resolver.clone());
                let app = server.router();

                let msg = format!("✅ DoH listener (HTTPS) started on {}", addr);
                info!("{}", msg);
                let time = Local::now().format("%Y-%m-%d %H:%M:%S");
                println!("{} {}", time, msg);

                let task = tokio::spawn(async move {
                    use axum::extract::ConnectInfo;
                    use hyper::server::conn::http1;
                    use hyper::service::service_fn;
                    use hyper_util::rt::TokioIo;
                    use tower::ServiceExt;

                    loop {
                        let (stream, peer_addr) = match tcp_listener.accept().await {
                            Ok(s) => s,
                            Err(e) => {
                                error!("TCP accept error: {}", e);
                                continue;
                            }
                        };

                        let acceptor = acceptor.clone();
                        let app = app.clone();

                        tokio::spawn(async move {
                            match acceptor.accept(stream).await {
                                Ok(tls_stream) => {
                                    let io = TokioIo::new(tls_stream);
                                    let svc = app.clone();
                                    let addr = peer_addr;

                                    let service = service_fn(
                                        move |mut req: hyper::Request<hyper::body::Incoming>| {
                                            let svc = svc.clone();
                                            // Inject ConnectInfo extension for client IP extraction
                                            req.extensions_mut().insert(ConnectInfo(addr));
                                            async move { svc.oneshot(req).await }
                                        },
                                    );

                                    if let Err(e) =
                                        http1::Builder::new().serve_connection(io, service).await
                                    {
                                        // Don't log connection reset errors as they're common
                                        if !e.to_string().contains("connection reset") {
                                            tracing::debug!(
                                                "HTTP connection error from {}: {}",
                                                peer_addr,
                                                e
                                            );
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::debug!(
                                        "TLS handshake failed from {}: {}",
                                        peer_addr,
                                        e
                                    );
                                }
                            }
                        });
                    }
                });
                task.abort_handle()
            }
            "doq" => {
                let tls_config = self.materialize_tls_config(protocol, &listener)?;

                match DoqDnsServer::new(addr, tls_config, resolver).await {
                    Ok(server) => {
                        Self::announce_started("DoQ", addr);
                        let task = tokio::spawn(async move {
                            if let Err(e) = server.run().await {
                                error!("DoQ server error: {}", e);
                            }
                            info!("DoQ listener stopped");
                        });
                        task.abort_handle()
                    }
                    Err(e) => {
                        error!("Failed to start DoQ server: {}", e);
                        return Err(e);
                    }
                }
            }
            "doh3" => {
                let tls_config = self.materialize_tls_config(protocol, &listener)?;

                match Doh3DnsServer::new(addr, tls_config, resolver).await {
                    Ok(server) => {
                        Self::announce_started("DoH3", addr);
                        let task = tokio::spawn(async move {
                            if let Err(e) = server.run().await {
                                error!("DoH3 server error: {}", e);
                            }
                            info!("DoH3 listener stopped");
                        });
                        task.abort_handle()
                    }
                    Err(e) => {
                        error!("Failed to start DoH3 server: {}", e);
                        return Err(e);
                    }
                }
            }
            _ => {
                let err = format!("Unknown protocol: {}", protocol);
                warn!("{}", err);
                return Err(anyhow::anyhow!(err));
            }
        };

        self.tasks
            .write()
            .await
            .insert(protocol.to_string(), handle);
        Ok(())
    }

    async fn start_udp_server(
        &self,
        requested_address: SocketAddr,
        resolver: Arc<DnsResolver>,
    ) -> anyhow::Result<(UdpDnsServer, Option<PrivilegedUdpForwarderLease>)> {
        if requested_address.port() >= 1024 {
            return Ok((UdpDnsServer::new(requested_address, resolver).await?, None));
        }
        if requested_address.port() != 53 {
            anyhow::bail!("Privileged DNS listeners only support UDP port 53");
        }

        let internal_ip = match requested_address.ip() {
            IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
        };
        let internal_server = UdpDnsServer::new(SocketAddr::new(internal_ip, 0), resolver).await?;
        let internal_address = internal_server.local_addr()?;
        let lease = PrivilegedUdpForwarderLease::start(
            requested_address,
            internal_address,
            &self.control_directory,
        )
        .await?;
        Ok((internal_server, Some(lease)))
    }

    /// Stop a specific listener
    pub async fn stop_listener(&self, protocol: &str) {
        let privileged_lease = self.privileged_udp_leases.write().await.remove(protocol);
        if let Some(mut lease) = privileged_lease {
            lease.stop_and_wait().await;
        }
        let mut tasks = self.tasks.write().await;
        if let Some(handle) = tasks.remove(protocol) {
            handle.abort();
            let msg = format!("🛑 {} listener stopped", protocol.to_uppercase());
            info!("{}", msg);
            let time = Local::now().format("%Y-%m-%d %H:%M:%S");
            println!("{} {}", time, msg);
        }
    }

    /// Check if a listener is running
    pub async fn is_running(&self, protocol: &str) -> bool {
        self.tasks.read().await.contains_key(protocol)
    }
}
