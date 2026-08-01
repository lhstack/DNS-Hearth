//! Manual DNS query use case, used by the web query tool.

use std::sync::Arc;

use crate::dns::message::RecordType;
use crate::dns::resolver::{DnsResolver, ResolveResult};
use crate::infrastructure::common::error::{AppError, AppResult};

/// Diagnostic DNS query use cases.
pub struct DnsQueryBusiness {
    resolver: Arc<DnsResolver>,
}

impl DnsQueryBusiness {
    pub fn new(resolver: Arc<DnsResolver>) -> Self {
        Self { resolver }
    }

    /// Resolve a domain through the full resolution pipeline.
    pub async fn resolve(&self, domain: &str, record_type: RecordType) -> AppResult<ResolveResult> {
        self.resolver
            .resolve_with_type(domain, record_type)
            .await
            .map_err(|e| AppError::Dns(e.to_string()))
    }
}
