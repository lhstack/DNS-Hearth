//! System settings use cases.

use std::sync::Arc;

use crate::dns::DataPlaneState;
use crate::infrastructure::common::error::{AppError, AppResult};
use crate::infrastructure::repository::Database;

const CONFIG_KEY_DISABLED_RECORD_TYPES: &str = "disabled_record_types";
const CONFIG_KEY_ALERT_ENABLED: &str = "alert_enabled";
const CONFIG_KEY_ALERT_WEBHOOK_URL: &str = "alert_webhook_url";
const CONFIG_KEY_ALERT_LATENCY_THRESHOLD: &str = "alert_latency_threshold_ms";

const VALID_RECORD_TYPES: [&str; 9] =
    ["A", "AAAA", "CNAME", "MX", "TXT", "PTR", "NS", "SOA", "SRV"];
const DEFAULT_LATENCY_THRESHOLD_MS: i64 = 200;

/// System-wide settings.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SystemSettings {
    pub disabled_record_types: Vec<String>,
    pub alert_enabled: bool,
    pub alert_webhook_url: Option<String>,
    pub alert_latency_threshold_ms: i64,
}

/// Requested settings changes; `None` means leave unchanged.
#[derive(Default)]
pub struct SettingsUpdate {
    pub disabled_record_types: Option<Vec<String>>,
    pub alert_enabled: Option<bool>,
    pub alert_webhook_url: Option<String>,
    pub alert_latency_threshold_ms: Option<i64>,
}

/// Orchestrates reading and writing system settings.
pub struct SettingBusiness {
    db: Arc<Database>,
    /// Query-path copy of the disabled record types, refreshed after writes.
    plane_state: Arc<DataPlaneState>,
}

impl SettingBusiness {
    pub fn new(db: Arc<Database>, plane_state: Arc<DataPlaneState>) -> Self {
        Self { db, plane_state }
    }

    /// Reads settings. A malformed stored value is reported rather than
    /// silently replaced by a default, so bad data stays visible.
    pub async fn get(&self) -> AppResult<SystemSettings> {
        let repo = self.db.system_config();

        let disabled_record_types = match repo.get(CONFIG_KEY_DISABLED_RECORD_TYPES).await? {
            Some(raw) => serde_json::from_str::<Vec<String>>(&raw).map_err(|e| {
                AppError::Config(format!(
                    "Invalid {}: {}",
                    CONFIG_KEY_DISABLED_RECORD_TYPES, e
                ))
            })?,
            None => Vec::new(),
        };

        let alert_latency_threshold_ms = match repo.get(CONFIG_KEY_ALERT_LATENCY_THRESHOLD).await? {
            Some(raw) => raw.parse::<i64>().map_err(|_| {
                AppError::Config(format!(
                    "Invalid {}: {}",
                    CONFIG_KEY_ALERT_LATENCY_THRESHOLD, raw
                ))
            })?,
            None => DEFAULT_LATENCY_THRESHOLD_MS,
        };

        Ok(SystemSettings {
            disabled_record_types,
            alert_enabled: repo.get(CONFIG_KEY_ALERT_ENABLED).await?.as_deref() == Some("true"),
            alert_webhook_url: repo.get(CONFIG_KEY_ALERT_WEBHOOK_URL).await?,
            alert_latency_threshold_ms,
        })
    }

    pub async fn update(&self, update: SettingsUpdate) -> AppResult<SystemSettings> {
        let repo = self.db.system_config();

        if let Some(types) = update.disabled_record_types {
            let normalized = Self::normalize_record_types(types)?;
            let value = serde_json::to_string(&normalized).map_err(|e| {
                AppError::Internal(format!("Failed to serialize record types: {}", e))
            })?;
            repo.set(CONFIG_KEY_DISABLED_RECORD_TYPES, &value).await?;

            // The query path reads this from memory, so a stale mask would keep
            // answering for a type the operator just turned off.
            self.plane_state
                .reload_disabled_types(&self.db)
                .await
                .map_err(|e| {
                    AppError::Internal(format!("Failed to refresh disabled record types: {}", e))
                })?;
        }

        if let Some(enabled) = update.alert_enabled {
            repo.set(
                CONFIG_KEY_ALERT_ENABLED,
                if enabled { "true" } else { "false" },
            )
            .await?;
        }

        if let Some(url) = update.alert_webhook_url {
            repo.set(CONFIG_KEY_ALERT_WEBHOOK_URL, &url).await?;
        }

        if let Some(threshold) = update.alert_latency_threshold_ms {
            if threshold <= 0 {
                return Err(AppError::Validation(
                    "Latency threshold must be greater than 0".to_string(),
                ));
            }
            repo.set(CONFIG_KEY_ALERT_LATENCY_THRESHOLD, &threshold.to_string())
                .await?;
        }

        self.get().await
    }

    /// Uppercases and validates record types against the supported set.
    fn normalize_record_types(types: Vec<String>) -> AppResult<Vec<String>> {
        types
            .into_iter()
            .map(|raw| {
                let upper = raw.to_uppercase();
                if VALID_RECORD_TYPES.contains(&upper.as_str()) {
                    Ok(upper)
                } else {
                    Err(AppError::Validation(format!(
                        "Invalid record type: {}",
                        raw
                    )))
                }
            })
            .collect()
    }

    /// Send a test notification to the configured webhook.
    pub async fn send_test_alert(&self) -> AppResult<()> {
        let webhook = self
            .db
            .system_config()
            .get(CONFIG_KEY_ALERT_WEBHOOK_URL)
            .await?
            .filter(|url| !url.is_empty())
            .ok_or_else(|| AppError::Validation("Webhook URL is not configured".to_string()))?;

        let payload = serde_json::json!({
            "text": "\u{1f514} **Test Alert**\n\nThis is a test notification from DNS Hearth.",
            "content": "Test notification from DNS Hearth"
        });

        reqwest::Client::new()
            .post(&webhook)
            .json(&payload)
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("Failed to send test alert: {}", e)))?;

        Ok(())
    }
}
