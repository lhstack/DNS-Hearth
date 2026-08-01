//! Runtime composition for the desktop application.

use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use tokio::task::JoinHandle;
use tracing::info;

use crate::business::cache_business::CacheBusiness;
use crate::business::dns_query_business::DnsQueryBusiness;
use crate::business::listener_business::ListenerBusiness;
use crate::business::log_business::LogBusiness;
use crate::business::record_business::RecordBusiness;
use crate::business::rewrite_business::RewriteBusiness;
use crate::business::setting_business::SettingBusiness;
use crate::business::status_business::StatusBusiness;
use crate::business::strategy_business::StrategyBusiness;
use crate::business::upstream_business::UpstreamBusiness;
use crate::dns::{
    CacheConfig, CacheManager, DnsResolver, ProxyManager, RewriteEngine, UpstreamManager,
};
use crate::infrastructure::config::ConfigManager;
use crate::infrastructure::listener_manager::ListenerManager;
use crate::infrastructure::log::{LogConfig, LogManager, RotationPolicy};
use crate::infrastructure::monitor::AlertManager;
use crate::infrastructure::repository::{Database, QueryLogWriter};
use crate::infrastructure::system_dns::{
    MacOsSystemDnsPlatform, MacOsSystemDnsSupervisor, SystemDnsConfig,
};

pub struct AppState {
    pub db: Arc<Database>,
    pub cache: Arc<CacheBusiness>,
    pub dns_query: Arc<DnsQueryBusiness>,
    pub listeners: Arc<ListenerBusiness>,
    pub logs: Arc<LogBusiness>,
    pub records: Arc<RecordBusiness>,
    pub rewrite: Arc<RewriteBusiness>,
    pub settings: Arc<SettingBusiness>,
    pub status: Arc<StatusBusiness>,
    pub strategy: Arc<StrategyBusiness>,
    pub upstreams: Arc<UpstreamBusiness>,
    pub system_dns: Arc<MacOsSystemDnsSupervisor>,
    background_tasks: Vec<JoinHandle<()>>,
}

impl AppState {
    pub async fn initialize(data_dir: &std::path::Path) -> Result<Self> {
        let config = ConfigManager::load_for_data_dir(data_dir)?;
        let app_config = config.get();
        let log_config = LogConfig {
            path: app_config.log_path.clone(),
            level: app_config.log_level.clone(),
            max_size: app_config.log_max_size,
            rotation: RotationPolicy::Daily,
            retention_days: app_config.log_retention_days,
        };
        LogManager::init_with_config(log_config.clone())?;

        let db = Arc::new(Database::new(&app_config.database_url).await?);
        let cache_ttl = read_u64_setting(&db, "cache_default_ttl", 60).await?;
        let cache_max_entries = read_usize_setting(&db, "cache_max_entries", 10_000).await?;
        let cache = Arc::new(CacheManager::with_config(CacheConfig {
            default_ttl: cache_ttl,
            max_entries: cache_max_entries,
        }));

        let rewrite_engine = Arc::new(RewriteEngine::with_db(db.clone()));
        rewrite_engine.load_rules().await?;
        let upstream_manager = Arc::new(UpstreamManager::with_db(db.clone()));
        upstream_manager.load_servers().await?;
        let proxy = Arc::new(ProxyManager::new(upstream_manager.clone()));
        if let Some(strategy) = db
            .system_config()
            .get("query_strategy")
            .await?
            .and_then(|value| crate::dns::proxy::QueryStrategy::from_str(&value))
        {
            proxy.set_strategy(strategy).await;
        }

        let query_log_writer = QueryLogWriter::start(db.clone());
        let resolver = Arc::new(DnsResolver::with_db(
            rewrite_engine.clone(),
            cache.clone(),
            proxy.clone(),
            db.clone(),
            query_log_writer.clone(),
        ));
        resolver.plane_state().reload(&db).await?;

        let listener_manager = Arc::new(ListenerManager::new(
            db.clone(),
            resolver.clone(),
            data_dir.join("listener-control"),
        ));
        listener_manager.start_all_enabled().await;

        let system_dns = Arc::new(MacOsSystemDnsSupervisor::new(
            Arc::new(MacOsSystemDnsPlatform),
            load_system_dns_config(&db).await?,
        )?);

        let mut background_tasks = Vec::new();
        background_tasks.push(system_dns.clone().spawn());
        background_tasks
            .push(Arc::new(AlertManager::new(db.clone(), upstream_manager.clone())).spawn());
        background_tasks.push(upstream_manager.clone().spawn_health_checker());

        let records = Arc::new(RecordBusiness::new(
            db.clone(),
            resolver.plane_state().clone(),
        ));
        let rewrite = Arc::new(RewriteBusiness::new(db.clone(), rewrite_engine));
        let upstreams = Arc::new(UpstreamBusiness::new(db.clone(), upstream_manager.clone()));
        let cache_business = Arc::new(CacheBusiness::new(db.clone(), cache));
        let listeners = Arc::new(ListenerBusiness::new(db.clone(), listener_manager));
        let settings = Arc::new(SettingBusiness::new(
            db.clone(),
            resolver.plane_state().clone(),
        ));
        let status = Arc::new(StatusBusiness::new(
            db.clone(),
            cache_business.cache(),
            proxy.clone(),
            upstream_manager,
            upstreams.clone(),
            query_log_writer,
            Instant::now(),
        ));

        info!("DNS Hearth runtime initialized without an HTTP management server");
        Ok(Self {
            db: db.clone(),
            cache: cache_business,
            dns_query: Arc::new(DnsQueryBusiness::new(resolver)),
            listeners,
            logs: Arc::new(LogBusiness::new(db.clone())),
            records,
            rewrite,
            settings,
            status,
            strategy: Arc::new(StrategyBusiness::new(db.clone(), proxy)),
            upstreams,
            system_dns,
            background_tasks,
        })
    }
}

impl Drop for AppState {
    fn drop(&mut self) {
        for task in &self.background_tasks {
            task.abort();
        }
    }
}

async fn read_u64_setting(db: &Database, key: &str, default: u64) -> Result<u64> {
    match db.system_config().get(key).await? {
        Some(value) => Ok(value
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid {}: {}", key, value))?),
        None => Ok(default),
    }
}

async fn read_usize_setting(db: &Database, key: &str, default: usize) -> Result<usize> {
    match db.system_config().get(key).await? {
        Some(value) => Ok(value
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid {}: {}", key, value))?),
        None => Ok(default),
    }
}

async fn load_system_dns_config(db: &Database) -> Result<SystemDnsConfig> {
    match db.system_config().get("network_dns_bindings").await? {
        Some(value) => Ok(serde_json::from_str(&value)?),
        None => Ok(SystemDnsConfig::default()),
    }
}
