//! Upstream latency monitoring and webhook alerting.

use std::sync::Arc;

use anyhow::Result;
use serde_json::json;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{interval, Duration, Instant};

use crate::dns::UpstreamManager;
use crate::infrastructure::repository::Database;

/// How often upstream latency is sampled.
const CHECK_INTERVAL: Duration = Duration::from_secs(30);

/// Minimum gap between two alerts for the same condition.
const ALERT_COOLDOWN: Duration = Duration::from_secs(300);

/// Latency threshold used when the operator has not configured one.
const DEFAULT_LATENCY_THRESHOLD_MS: f64 = 200.0;

/// Watches upstream latency and pushes webhook alerts when it degrades.
pub struct AlertManager {
    db: Arc<Database>,
    upstream_manager: Arc<UpstreamManager>,
    http: reqwest::Client,
    last_alert_at: Mutex<Option<Instant>>,
}

/// Alert configuration resolved from the system config table.
struct AlertSettings {
    webhook_url: String,
    latency_threshold_ms: f64,
}

impl AlertManager {
    pub fn new(db: Arc<Database>, upstream_manager: Arc<UpstreamManager>) -> Self {
        Self {
            db,
            upstream_manager,
            http: reqwest::Client::new(),
            last_alert_at: Mutex::new(None),
        }
    }

    /// Spawn the sampling loop. The returned handle owns the task lifecycle.
    pub fn spawn(self: Arc<Self>) -> JoinHandle<()> {
        tracing::info!("Alert manager started (interval: {:?})", CHECK_INTERVAL);
        tokio::spawn(async move {
            let mut ticker = interval(CHECK_INTERVAL);
            loop {
                ticker.tick().await;
                if let Err(e) = self.check_once().await {
                    tracing::error!("Alert check failed: {}", e);
                }
            }
        })
    }

    async fn check_once(&self) -> Result<()> {
        let Some(settings) = self.load_settings().await? else {
            return Ok(());
        };

        let mut last_alert = self.last_alert_at.lock().await;
        if let Some(last) = *last_alert {
            if last.elapsed() < ALERT_COOLDOWN {
                return Ok(());
            }
        }

        let Some(avg_latency_ms) = self.weighted_average_latency_ms().await else {
            return Ok(());
        };

        if avg_latency_ms <= settings.latency_threshold_ms {
            return Ok(());
        }

        self.send_alert(&settings, avg_latency_ms).await?;
        *last_alert = Some(Instant::now());
        Ok(())
    }

    /// Returns `None` when alerting is disabled or no webhook is configured.
    async fn load_settings(&self) -> Result<Option<AlertSettings>> {
        let config = self.db.system_config();

        if config.get("alert_enabled").await?.as_deref() != Some("true") {
            return Ok(None);
        }

        let webhook_url = match config.get("alert_webhook_url").await? {
            Some(url) if !url.is_empty() => url,
            _ => return Ok(None),
        };

        let latency_threshold_ms = match config.get("alert_latency_threshold_ms").await? {
            Some(raw) => raw.parse().map_err(|e| {
                anyhow::anyhow!("Invalid alert_latency_threshold_ms {:?}: {}", raw, e)
            })?,
            None => DEFAULT_LATENCY_THRESHOLD_MS,
        };

        Ok(Some(AlertSettings {
            webhook_url,
            latency_threshold_ms,
        }))
    }

    /// Query-count weighted mean of per-upstream EMA latency.
    /// Returns `None` when no upstream has served a query yet.
    async fn weighted_average_latency_ms(&self) -> Option<f64> {
        let stats = self.upstream_manager.get_all_stats().await;

        let mut weighted_sum = 0.0;
        let mut total_queries = 0u64;
        for entry in stats.values().filter(|s| s.queries > 0) {
            weighted_sum += entry.queries as f64 * entry.ema_response_time_ms;
            total_queries += entry.queries;
        }

        if total_queries == 0 {
            return None;
        }
        Some(weighted_sum / total_queries as f64)
    }

    async fn send_alert(&self, settings: &AlertSettings, avg_latency_ms: f64) -> Result<()> {
        let message = format!(
            "[DNS Hearth] 上游平均延迟告警\n当前加权平均延迟: {:.2}ms\n告警阈值: {:.0}ms\n请检查上游服务器状态。",
            avg_latency_ms, settings.latency_threshold_ms
        );

        // `text` targets Slack, `content` targets Discord.
        let payload = json!({ "text": message, "content": message });

        let response = self
            .http
            .post(&settings.webhook_url)
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            anyhow::bail!("Webhook returned status {}", response.status());
        }

        tracing::info!("Latency alert delivered ({:.2}ms)", avg_latency_ms);
        Ok(())
    }
}
