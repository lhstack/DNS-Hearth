//! DNS record use cases.
//!
//! Every mutation refreshes the resolver's local-record mask. The resolver uses
//! that mask to skip the database entirely for record types that have no local
//! records, so a stale mask would either hide a new record or keep querying for
//! a type that no longer has any.

use std::sync::Arc;

use crate::dns::DataPlaneState;
use crate::infrastructure::common::error::{AppError, AppResult};
use crate::infrastructure::repository::{CreateDnsRecord, Database, DnsRecord, UpdateDnsRecord};

/// Orchestrates DNS record management.
pub struct RecordBusiness {
    db: Arc<Database>,
    plane_state: Arc<DataPlaneState>,
}

impl RecordBusiness {
    pub fn new(db: Arc<Database>, plane_state: Arc<DataPlaneState>) -> Self {
        Self { db, plane_state }
    }

    /// Refresh the resolver's local-record mask after a mutation.
    async fn refresh_plane_state(&self) -> AppResult<()> {
        self.plane_state
            .reload_local_record_types(&self.db)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to refresh local record types: {}", e)))
    }

    pub async fn list(&self) -> AppResult<Vec<DnsRecord>> {
        Ok(self.db.dns_records().list().await?)
    }

    pub async fn get(&self, id: i64) -> AppResult<DnsRecord> {
        self.db
            .dns_records()
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("DNS record with id {} not found", id)))
    }

    pub async fn create(&self, record: CreateDnsRecord) -> AppResult<DnsRecord> {
        let created = self.db.dns_records().create(record).await?;
        self.refresh_plane_state().await?;
        Ok(created)
    }

    /// Loads the record first so callers can validate the merged result before writing.
    pub async fn update(&self, id: i64, update: UpdateDnsRecord) -> AppResult<DnsRecord> {
        let updated = self
            .db
            .dns_records()
            .update(id, update)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("DNS record with id {} not found", id)))?;

        self.refresh_plane_state().await?;
        Ok(updated)
    }

    pub async fn delete(&self, id: i64) -> AppResult<()> {
        if self.db.dns_records().delete(id).await? {
            self.refresh_plane_state().await?;
            return Ok(());
        }
        Err(AppError::NotFound(format!(
            "DNS record with id {} not found",
            id
        )))
    }
}
