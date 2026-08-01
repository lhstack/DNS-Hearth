//! Hot-path data plane state.
//!
//! The resolver used to hit SQLite twice for every query, before the cache was
//! even consulted: once to read the `disabled_record_types` setting and once to
//! look for local records. Both are near-static compared to query volume, so a
//! cache hit paid two round trips through a five-connection pool for no reason.
//!
//! This module keeps those two facts as atomic bitmasks. Reads are lock-free and
//! allocation-free; writes happen only when the corresponding configuration or
//! record set changes, and are driven by the business layer that owns the
//! mutation.

use std::sync::atomic::{AtomicU16, Ordering};

use anyhow::{anyhow, Result};

use crate::infrastructure::repository::Database;

use super::message::RecordType;

/// Key holding the JSON array of record types the operator has disabled.
const CONFIG_KEY_DISABLED_RECORD_TYPES: &str = "disabled_record_types";

impl RecordType {
    /// Position of this record type in the data plane bitmasks.
    fn mask_bit(self) -> u16 {
        let index = match self {
            RecordType::A => 0,
            RecordType::AAAA => 1,
            RecordType::CNAME => 2,
            RecordType::MX => 3,
            RecordType::TXT => 4,
            RecordType::PTR => 5,
            RecordType::NS => 6,
            RecordType::SOA => 7,
            RecordType::SRV => 8,
        };
        1 << index
    }
}

/// Query-path state that would otherwise require a database round trip.
pub struct DataPlaneState {
    /// Record types the operator has disabled.
    disabled_types: AtomicU16,
    /// Record types for which at least one enabled local record exists.
    ///
    /// A query whose type is absent here cannot match a local record, so the
    /// lookup is skipped entirely.
    local_record_types: AtomicU16,
}

impl DataPlaneState {
    /// Create state with nothing disabled and no local records known.
    pub fn new() -> Self {
        Self {
            disabled_types: AtomicU16::new(0),
            local_record_types: AtomicU16::new(0),
        }
    }

    /// Whether the operator has disabled this record type.
    pub fn is_type_disabled(&self, record_type: RecordType) -> bool {
        self.disabled_types.load(Ordering::Relaxed) & record_type.mask_bit() != 0
    }

    /// Whether a local record of this type could exist.
    pub fn may_have_local_record(&self, record_type: RecordType) -> bool {
        self.local_record_types.load(Ordering::Relaxed) & record_type.mask_bit() != 0
    }

    /// Load both masks from the database.
    ///
    /// A malformed `disabled_record_types` value is reported rather than treated
    /// as "nothing disabled": silently ignoring it would leave record types
    /// answering traffic the operator had turned off.
    pub async fn reload(&self, db: &Database) -> Result<()> {
        self.reload_disabled_types(db).await?;
        self.reload_local_record_types(db).await
    }

    /// Refresh the disabled record type mask from stored configuration.
    pub async fn reload_disabled_types(&self, db: &Database) -> Result<()> {
        let raw = db
            .system_config()
            .get(CONFIG_KEY_DISABLED_RECORD_TYPES)
            .await?;

        let mask = match raw {
            Some(raw) => {
                let names: Vec<String> = serde_json::from_str(&raw)
                    .map_err(|e| anyhow!("Invalid {}: {}", CONFIG_KEY_DISABLED_RECORD_TYPES, e))?;
                Self::mask_from_names(&names)?
            }
            None => 0,
        };

        self.disabled_types.store(mask, Ordering::Relaxed);
        Ok(())
    }

    /// Refresh the mask of record types that have at least one local record.
    pub async fn reload_local_record_types(&self, db: &Database) -> Result<()> {
        let names = db.dns_records().list_enabled_record_types().await?;
        let mask = Self::mask_from_names(&names)?;
        self.local_record_types.store(mask, Ordering::Relaxed);
        Ok(())
    }

    /// Build a bitmask from record type names, rejecting unknown ones.
    fn mask_from_names(names: &[String]) -> Result<u16> {
        let mut mask = 0u16;
        for name in names {
            let record_type: RecordType = name
                .parse()
                .map_err(|_| anyhow!("Unknown record type '{}'", name))?;
            mask |= record_type.mask_bit();
        }
        Ok(mask)
    }
}

impl Default for DataPlaneState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_masks_start_empty() {
        let state = DataPlaneState::new();
        for &record_type in RecordType::all() {
            assert!(!state.is_type_disabled(record_type));
            assert!(!state.may_have_local_record(record_type));
        }
    }

    #[test]
    fn test_each_record_type_has_a_distinct_bit() {
        let mut seen = 0u16;
        for &record_type in RecordType::all() {
            let bit = record_type.mask_bit();
            assert_eq!(seen & bit, 0, "duplicate bit for {:?}", record_type);
            seen |= bit;
        }
    }

    #[test]
    fn test_mask_from_names_is_case_insensitive() {
        let mask =
            DataPlaneState::mask_from_names(&["aaaa".to_string(), "TXT".to_string()]).unwrap();

        assert_eq!(
            mask,
            RecordType::AAAA.mask_bit() | RecordType::TXT.mask_bit()
        );
    }

    #[test]
    fn test_mask_from_names_rejects_unknown_type() {
        // Accepting an unknown name would silently drop a type the operator
        // meant to disable.
        let err = DataPlaneState::mask_from_names(&["NOTATYPE".to_string()]).unwrap_err();
        assert!(err.to_string().contains("Unknown record type"));
    }

    #[test]
    fn test_disabled_lookup_only_matches_stored_types() {
        let state = DataPlaneState::new();
        state.disabled_types.store(
            DataPlaneState::mask_from_names(&["AAAA".to_string()]).unwrap(),
            Ordering::Relaxed,
        );

        assert!(state.is_type_disabled(RecordType::AAAA));
        assert!(!state.is_type_disabled(RecordType::A));
    }
}
