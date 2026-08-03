//! Tauri command adapter. It is the only management boundary exposed to Vue.

use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::State;

use crate::application::runtime::AppState;
use crate::business::setting_business::SettingsUpdate;
use crate::dns::message::RecordType;
use crate::dns::proxy::QueryStrategy;
use crate::infrastructure::repository::{CreateRewriteRule, QueryLogFilter, UpdateServerListener};
use crate::infrastructure::system_dns::{NetworkServiceDns, SystemDnsConfig};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct InvokeRequest {
    pub path: String,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandFailure {
    pub message: String,
}

#[tauri::command]
pub async fn invoke_api(
    state: State<'_, AppState>,
    request: InvokeRequest,
) -> Result<Value, CommandFailure> {
    dispatch(state.inner(), request)
        .await
        .map_err(|error| CommandFailure {
            message: error.to_string(),
        })
}

#[tauri::command]
pub async fn get_system_dns_config(
    state: State<'_, AppState>,
) -> Result<SystemDnsConfig, CommandFailure> {
    Ok(state.system_dns.config().await)
}

#[tauri::command]
pub async fn get_network_services(
    state: State<'_, AppState>,
) -> Result<Vec<NetworkServiceDns>, CommandFailure> {
    state.system_dns.inspect().await.map_err(failure)
}

#[tauri::command]
pub async fn update_system_dns_config(
    state: State<'_, AppState>,
    config: SystemDnsConfig,
) -> Result<SystemDnsConfig, CommandFailure> {
    config.validate().map_err(failure)?;
    let serialized = serde_json::to_string(&config).map_err(failure)?;
    state
        .db
        .system_config()
        .set("network_dns_bindings", &serialized)
        .await
        .map_err(failure)?;
    state
        .system_dns
        .update_config(config.clone())
        .await
        .map_err(failure)?;
    Ok(config)
}

#[tauri::command]
pub async fn enforce_system_dns(state: State<'_, AppState>) -> Result<Value, CommandFailure> {
    serde_json::to_value(state.system_dns.enforce_once().await.map_err(failure)?).map_err(failure)
}

#[tauri::command]
pub async fn clear_network_service_dns(
    state: State<'_, AppState>,
    service: String,
) -> Result<NetworkServiceDns, CommandFailure> {
    state
        .system_dns
        .clear_service(&service)
        .await
        .map_err(failure)
}

#[derive(Debug, Deserialize)]
pub struct HomeConfiguration {
    pub system_dns: SystemDnsConfig,
    pub udp_enabled: bool,
    pub udp_bind_address: String,
    pub udp_port: i32,
}

#[derive(Debug, Serialize)]
pub struct HomeConfigurationResult {
    pub system_dns: SystemDnsConfig,
    pub udp_listener: crate::infrastructure::repository::ServerListener,
    pub services: Vec<NetworkServiceDns>,
}

#[tauri::command]
pub async fn apply_home_configuration(
    state: State<'_, AppState>,
    configuration: HomeConfiguration,
) -> Result<HomeConfigurationResult, CommandFailure> {
    configuration.system_dns.validate().map_err(failure)?;
    let serialized = serde_json::to_string(&configuration.system_dns).map_err(failure)?;

    // Start the listener before publishing DNS bindings that may point to it.
    let udp_listener = state
        .listeners
        .update(
            "udp",
            UpdateServerListener {
                enabled: Some(configuration.udp_enabled),
                bind_address: Some(configuration.udp_bind_address),
                port: Some(configuration.udp_port),
                ..Default::default()
            },
        )
        .await
        .map_err(failure)?;

    if let Err(error) = state
        .db
        .system_config()
        .set("network_dns_bindings", &serialized)
        .await
    {
        // DNS configuration was not published, while the listener update is a
        // complete and independently valid operation. Report persistence error.
        return Err(failure(error));
    }
    state
        .system_dns
        .update_config(configuration.system_dns.clone())
        .await
        .map_err(failure)?;
    let services = state.system_dns.enforce_once().await.map_err(failure)?;
    Ok(HomeConfigurationResult {
        system_dns: configuration.system_dns,
        udp_listener,
        services,
    })
}

fn failure(error: impl std::fmt::Display) -> CommandFailure {
    CommandFailure {
        message: error.to_string(),
    }
}

async fn dispatch(state: &AppState, request: InvokeRequest) -> anyhow::Result<Value> {
    let method = request.method.to_uppercase();
    let path = request.path.trim_end_matches('/');
    match (method.as_str(), path) {
        ("GET", "/api/records") => value(json!({"data": state.records.list().await?})),
        ("POST", "/api/records") => {
            value(state.records.create(from_value(request.payload)?).await?)
        }
        ("GET", p) if p.starts_with("/api/records/") => {
            value(state.records.get(parse_id(p)?).await?)
        }
        ("PUT", p) if p.starts_with("/api/records/") => value(
            state
                .records
                .update(parse_id(p)?, from_value(request.payload)?)
                .await?,
        ),
        ("DELETE", p) if p.starts_with("/api/records/") => {
            state.records.delete(parse_id(p)?).await?;
            Ok(json!({"message":"deleted"}))
        }
        ("GET", "/api/rewrite") => {
            let page = request
                .params
                .get("page")
                .and_then(Value::as_i64)
                .unwrap_or(1);
            let page_size = request
                .params
                .get("page_size")
                .and_then(Value::as_i64)
                .unwrap_or(20);
            if page < 1 {
                anyhow::bail!("rewrite page must be at least 1");
            }
            if !(1..=100).contains(&page_size) {
                anyhow::bail!("rewrite page_size must be between 1 and 100");
            }
            let (data, stats) = state.rewrite.list_paged(page, page_size).await?;
            Ok(json!({
                "data": data,
                "total": stats.total,
                "stats": stats,
                "page": page,
                "page_size": page_size
            }))
        }
        ("POST", "/api/rewrite") => {
            value(json!({"data": state.rewrite.create(from_value(request.payload)?).await?}))
        }
        ("POST", "/api/rewrite/batch") => {
            let rules: Vec<CreateRewriteRule> = from_value(request.payload)?;
            let count = state.rewrite.batch_create(rules).await?;
            Ok(json!({"created":count}))
        }
        ("PUT", p) if p.starts_with("/api/rewrite/") => value(
            json!({"data": state.rewrite.update(parse_id(p)?, from_value(request.payload)?).await?}),
        ),
        ("DELETE", p) if p.starts_with("/api/rewrite/") => {
            state.rewrite.delete(parse_id(p)?).await?;
            Ok(json!({"message":"deleted"}))
        }
        ("GET", "/api/upstreams") => {
            let page = request
                .params
                .get("page")
                .and_then(Value::as_i64)
                .unwrap_or(1);
            let size = request
                .params
                .get("page_size")
                .and_then(Value::as_i64)
                .unwrap_or(20);
            let (items, total) = state.upstreams.list_paged(page, size).await?;
            Ok(json!({"data":items,"total":total,"page":page,"page_size":size}))
        }
        ("POST", "/api/upstreams") => {
            value(state.upstreams.create(from_value(request.payload)?).await?)
        }
        ("PUT", p) if p.starts_with("/api/upstreams/") && !p.ends_with("reset-health") => value(
            state
                .upstreams
                .update(parse_id(p)?, from_value(request.payload)?)
                .await?,
        ),
        ("DELETE", p) if p.starts_with("/api/upstreams/") => {
            state.upstreams.delete(parse_id(p)?).await?;
            Ok(json!({"message":"deleted"}))
        }
        ("GET", "/api/upstreams/status") => {
            value(json!({"data": state.upstreams.list_status().await?}))
        }
        ("POST", p) if p.ends_with("/reset-health") => {
            state
                .upstreams
                .reset_health(parse_id(
                    p.trim_end_matches("/reset-health").trim_end_matches('/'),
                )?)
                .await?;
            Ok(json!({"message":"reset"}))
        }
        ("GET", "/api/cache/stats") => value(state.cache.stats().await),
        ("GET", "/api/cache/config") => value(state.cache.config().await),
        ("PUT", "/api/cache/config") => {
            let p: CachePayload = from_value(request.payload)?;
            value(
                state
                    .cache
                    .update_config(p.default_ttl, p.max_entries)
                    .await?,
            )
        }
        ("POST", "/api/cache/clear") => {
            state.cache.clear_all().await;
            Ok(json!({"message":"cleared"}))
        }
        ("POST", p) if p.starts_with("/api/cache/clear/") => {
            state
                .cache
                .clear_domain(&url_decode(p.trim_start_matches("/api/cache/clear/"))?)
                .await?;
            Ok(json!({"message":"cleared"}))
        }
        ("POST", "/api/cache/cleanup") => {
            Ok(json!({"remaining_entries":state.cache.cleanup_expired().await}))
        }
        ("POST", "/api/dns/query") => {
            let p: DnsQueryPayload = from_value(request.payload)?;
            let result = state
                .dns_query
                .resolve(&p.domain, RecordType::from_str(&p.record_type)?)
                .await?;
            Ok(
                json!({"domain":p.domain,"record_type":p.record_type.to_uppercase(),"records":result.response.answers,"response_time_ms":result.metadata.response_time_ms,"cache_hit":result.metadata.cache_hit,"upstream_used":result.metadata.upstream_used,"rewrite_applied":result.metadata.rewrite_applied,"response_code":result.response.response_code.to_string()}),
            )
        }
        ("GET", "/api/listeners") => value(json!({"data":state.listeners.list().await?})),
        ("GET", p) if p.ends_with("/cert") => value(
            state
                .listeners
                .certificate_details(p.trim_end_matches("/cert").trim_end_matches('/'))
                .await?,
        ),
        ("PUT", p) if p.starts_with("/api/listeners/") => value(
            state
                .listeners
                .update(
                    p.trim_start_matches("/api/listeners/"),
                    from_value(request.payload)?,
                )
                .await?,
        ),
        ("GET", "/api/settings") => value(state.settings.get().await?),
        ("PUT", "/api/settings") => {
            let p: SettingsPayload = from_value(request.payload)?;
            value(
                state
                    .settings
                    .update(SettingsUpdate {
                        disabled_record_types: p.disabled_record_types,
                        alert_enabled: p.alert_enabled,
                        alert_webhook_url: p.alert_webhook_url,
                        alert_latency_threshold_ms: p.alert_latency_threshold_ms,
                    })
                    .await?,
            )
        }
        ("POST", "/api/settings/test-alert") => {
            state.settings.send_test_alert().await?;
            Ok(json!({"message":"sent"}))
        }
        ("GET", "/api/strategy") => value(strategy_view(state.strategy.current().await)),
        ("GET", "/api/strategy/available") => {
            value(json!({"strategies": available_strategy_views()}))
        }
        ("PUT", "/api/strategy") => {
            let p: StrategyPayload = from_value(request.payload)?;
            let strategy = state
                .strategy
                .update(
                    QueryStrategy::from_str(&p.strategy)
                        .ok_or_else(|| anyhow::anyhow!("Invalid strategy"))?,
                )
                .await?;
            value(strategy_view(strategy))
        }
        ("GET", "/api/status") => status_json(state).await,
        ("GET", "/api/status/health") => {
            let h = state.status.health_report().await;
            Ok(
                json!({"status":h.status(),"database":h.database,"cache":h.cache,"upstreams":h.upstreams}),
            )
        }
        ("GET", "/api/logs") => {
            let p: LogsPayload = from_value(request.params)?;
            let result = state.logs.list(p.into_filter()).await?;
            Ok(
                json!({"data":result.items,"total":result.total,"limit":result.limit,"offset":result.offset}),
            )
        }
        ("GET", "/api/logs/export") => {
            let format = request
                .params
                .get("format")
                .and_then(Value::as_str)
                .unwrap_or("json");
            let p: LogsPayload = from_value(request.params.clone())?;
            let logs = state.logs.list_for_export(p.into_filter(), 10_000).await?;
            match format {
                "json" => value(logs),
                "csv" => {
                    let mut csv = String::from("id,client_ip,query_name,query_type,response_code,response_time,cache_hit,upstream_used,created_at\n");
                    for log in logs {
                        csv.push_str(&format!(
                            "{},{},{},{},{},{},{},{},{}\n",
                            log.id,
                            log.client_ip,
                            log.query_name,
                            log.query_type,
                            log.response_code.unwrap_or_default(),
                            log.response_time
                                .map(|value| value.to_string())
                                .unwrap_or_default(),
                            log.cache_hit,
                            log.upstream_used.unwrap_or_default(),
                            log.created_at
                        ));
                    }
                    Ok(Value::String(csv))
                }
                other => Err(anyhow::anyhow!("Unsupported export format: {}", other)),
            }
        }
        ("GET", "/api/logs/stats") => value(state.logs.stats().await?),
        ("GET", "/api/logs/retention") => value(state.logs.retention_settings().await?),
        ("PUT", "/api/logs/retention") => {
            let p: RetentionPayload = from_value(request.payload)?;
            value(
                state
                    .logs
                    .update_retention_settings(p.auto_cleanup_enabled, p.retention_days)
                    .await?,
            )
        }
        ("DELETE", "/api/logs/cleanup/all") => {
            Ok(json!({"deleted_count":state.logs.cleanup_all().await?}))
        }
        ("DELETE", "/api/logs/cleanup/before") => {
            let p: BeforePayload = from_value(request.params)?;
            Ok(json!({"deleted_count":state.logs.cleanup_before_date(&p.before_date).await?}))
        }
        _ => Err(anyhow::anyhow!(
            "Unsupported Tauri bridge operation: {} {}",
            method,
            path
        )),
    }
}

fn strategy_view(strategy: QueryStrategy) -> Value {
    json!({
        "strategy": strategy.as_str(),
        "description": strategy_description(strategy),
    })
}

fn available_strategy_views() -> Vec<Value> {
    [
        QueryStrategy::Concurrent,
        QueryStrategy::Fastest,
        QueryStrategy::RoundRobin,
        QueryStrategy::Random,
    ]
    .into_iter()
    .map(|strategy| {
        json!({
            "name": strategy.as_str(),
            "description": strategy_description(strategy),
        })
    })
    .collect()
}

fn strategy_description(strategy: QueryStrategy) -> &'static str {
    match strategy {
        QueryStrategy::Concurrent => {
            "同时查询所有健康上游；首个 NOERROR 立即返回，若无 NOERROR 则在全部完成后返回最快 NXDOMAIN。"
        }
        QueryStrategy::Fastest => {
            "根据历史成功查询延迟选择最低延迟的健康上游，只向该上游发送查询。"
        }
        QueryStrategy::RoundRobin => "按顺序轮换使用健康上游，均衡分配查询。",
        QueryStrategy::Random => "每次随机选择一个健康上游进行查询。",
    }
}

async fn status_json(state: &AppState) -> anyhow::Result<Value> {
    let s = state.status.system_status().await?;
    Ok(
        json!({"status":"running","uptime_seconds":s.uptime_seconds,"cache":{"entries":s.cache_stats.entries,"hits":s.cache_stats.hits,"misses":s.cache_stats.misses,"hit_rate":s.cache_stats.hit_rate(),"default_ttl":s.cache_config.default_ttl,"max_entries":s.cache_config.max_entries},"query":s.query_stats,"upstreams":{"total":s.upstreams.len(),"healthy":s.healthy_upstreams,"servers":s.upstreams},"strategy":s.strategy,"dropped_log_entries":s.dropped_query_logs}),
    )
}
fn value<T: Serialize>(v: T) -> anyhow::Result<Value> {
    Ok(serde_json::to_value(v)?)
}
fn from_value<T: for<'de> Deserialize<'de>>(v: Value) -> anyhow::Result<T> {
    Ok(serde_json::from_value(v)?)
}
fn parse_id(path: &str) -> anyhow::Result<i64> {
    path.rsplit('/')
        .next()
        .ok_or_else(|| anyhow::anyhow!("Missing id"))?
        .parse()
        .map_err(Into::into)
}
fn url_decode(v: &str) -> anyhow::Result<String> {
    urlencoding::decode(v)
        .map(|v| v.into_owned())
        .map_err(Into::into)
}

#[derive(Debug, Deserialize)]
struct CachePayload {
    default_ttl: Option<u64>,
    max_entries: Option<usize>,
}
#[derive(Debug, Deserialize)]
struct DnsQueryPayload {
    domain: String,
    record_type: String,
}
#[derive(Debug, Deserialize)]
struct SettingsPayload {
    disabled_record_types: Option<Vec<String>>,
    alert_enabled: Option<bool>,
    alert_webhook_url: Option<String>,
    alert_latency_threshold_ms: Option<i64>,
}
#[derive(Debug, Deserialize)]
struct StrategyPayload {
    strategy: String,
}
#[derive(Debug, Deserialize)]
struct RetentionPayload {
    auto_cleanup_enabled: Option<bool>,
    retention_days: Option<i64>,
}
#[derive(Debug, Deserialize)]
struct BeforePayload {
    before_date: String,
}
#[derive(Debug, Deserialize)]
struct LogsPayload {
    query_name: Option<String>,
    query_type: Option<String>,
    client_ip: Option<String>,
    cache_hit: Option<bool>,
    start_time: Option<String>,
    end_time: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}
impl LogsPayload {
    fn into_filter(self) -> QueryLogFilter {
        QueryLogFilter {
            query_name: self.query_name,
            query_type: self.query_type,
            client_ip: self.client_ip,
            cache_hit: self.cache_hit,
            start_time: self.start_time.and_then(|v| {
                chrono::DateTime::parse_from_rfc3339(&v)
                    .ok()
                    .map(|v| v.with_timezone(&chrono::Utc))
            }),
            end_time: self.end_time.and_then(|v| {
                chrono::DateTime::parse_from_rfc3339(&v)
                    .ok()
                    .map(|v| v.with_timezone(&chrono::Utc))
            }),
            limit: self.limit,
            offset: self.offset,
        }
    }
}
