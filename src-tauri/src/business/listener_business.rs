//! Listener use cases: configuration persistence plus runtime lifecycle.

use std::sync::Arc;

use crate::infrastructure::common::cert::{inspect_pem_chain, CertificateDetails};
use crate::infrastructure::common::error::{AppError, AppResult};
use crate::infrastructure::listener_manager::ListenerManager;
use crate::infrastructure::repository::{Database, ServerListener, UpdateServerListener};

/// Protocols that cannot serve traffic without a TLS keypair.
const TLS_PROTOCOLS: [&str; 4] = ["dot", "doh", "doq", "doh3"];

pub struct ListenerBusiness {
    db: Arc<Database>,
    listener_manager: Arc<ListenerManager>,
}

impl ListenerBusiness {
    pub fn new(db: Arc<Database>, listener_manager: Arc<ListenerManager>) -> Self {
        Self {
            db,
            listener_manager,
        }
    }

    pub async fn list(&self) -> AppResult<Vec<ServerListener>> {
        Ok(self.db.server_listeners().list().await?)
    }

    /// Persist listener config, then align the running listener with it.
    ///
    /// If the listener is enabled but fails to bind, the enabled flag is rolled
    /// back so the stored config never claims a listener that is not running.
    pub async fn update(
        &self,
        protocol: &str,
        update: UpdateServerListener,
    ) -> AppResult<ServerListener> {
        let listener = self
            .db
            .server_listeners()
            .update(protocol, update)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("监听器 '{}' 不存在", protocol)))?;

        if !listener.enabled {
            self.listener_manager.stop_listener(protocol).await;
            tracing::info!("Listener {} disabled", protocol);
            return Ok(listener);
        }

        self.warn_on_missing_tls(&listener);

        if let Err(e) = self.listener_manager.start_listener(protocol).await {
            self.rollback_enabled(protocol).await;
            return Err(AppError::Internal(format!("启动失败: {}", e)));
        }

        tracing::info!(
            "Listener {} updated: enabled={}, port={}",
            protocol,
            listener.enabled,
            listener.port
        );
        Ok(listener)
    }

    pub async fn certificate_details(&self, protocol: &str) -> AppResult<CertificateDetails> {
        let listener = self.require_listener(protocol).await?;
        let cert_pem = listener
            .tls_cert
            .ok_or_else(|| AppError::NotFound("该监听器未配置证书".to_string()))?;
        inspect_pem_chain(&cert_pem)
    }

    async fn require_listener(&self, protocol: &str) -> AppResult<ServerListener> {
        self.db
            .server_listeners()
            .get_by_protocol(protocol)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("监听器 '{}' 不存在", protocol)))
    }

    /// Undo the enabled flag after a failed start so config matches reality.
    async fn rollback_enabled(&self, protocol: &str) {
        let revert = UpdateServerListener {
            enabled: Some(false),
            ..Default::default()
        };
        if let Err(e) = self.db.server_listeners().update(protocol, revert).await {
            tracing::error!(
                "Listener {} failed to start and the enabled flag could not be reverted: {}",
                protocol,
                e
            );
        }
    }

    fn warn_on_missing_tls(&self, listener: &ServerListener) {
        let requires_tls = TLS_PROTOCOLS.contains(&listener.protocol.as_str());
        if requires_tls && (listener.tls_cert.is_none() || listener.tls_key.is_none()) {
            tracing::warn!(
                "Listener {} enabled but TLS certificates are not configured",
                listener.protocol
            );
        }
    }
}
