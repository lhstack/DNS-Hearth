//! macOS network-service DNS inspection and correction.

use std::collections::HashSet;
use std::net::IpAddr;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex as AsyncMutex, Notify, RwLock};
use tokio::task::JoinHandle;
use tracing::{error, info};

const NETWORK_SETUP: &str = "/usr/sbin/networksetup";
const MIN_CHECK_INTERVAL_SECS: u64 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkDnsBinding {
    pub service: String,
    pub enabled: bool,
    pub preset_servers: Vec<IpAddr>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SystemDnsConfig {
    pub bindings: Vec<NetworkDnsBinding>,
    pub check_interval_secs: u64,
}

impl Default for SystemDnsConfig {
    fn default() -> Self {
        Self {
            bindings: Vec::new(),
            check_interval_secs: 30,
        }
    }
}

impl SystemDnsConfig {
    pub fn validate(&self) -> Result<()> {
        if self.check_interval_secs < MIN_CHECK_INTERVAL_SECS {
            bail!("DNS 检测频率不能小于 1 秒");
        }
        let mut services = HashSet::new();
        for binding in &self.bindings {
            let service = binding.service.trim();
            if service.is_empty() {
                bail!("网络服务名称不能为空");
            }
            if !services.insert(service) {
                bail!("网络服务不能重复配置: {}", service);
            }
            if binding.enabled && binding.preset_servers.is_empty() {
                bail!("已启用的网络服务必须配置至少一个 DNS 地址: {}", service);
            }
            if binding.preset_servers.iter().collect::<HashSet<_>>().len()
                != binding.preset_servers.len()
            {
                bail!("网络服务 {} 的 DNS 地址不能重复", service);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkServiceDns {
    pub service: String,
    pub current_servers: Vec<IpAddr>,
    pub enabled: bool,
    pub preset_servers: Vec<IpAddr>,
    pub matches_preset: bool,
}

pub trait SystemDnsPlatform: Send + Sync + 'static {
    fn list_network_services(&self) -> Result<Vec<String>>;
    fn get_servers(&self, service: &str) -> Result<Vec<IpAddr>>;
    fn set_servers(&self, service: &str, servers: &[IpAddr]) -> Result<()>;
    fn clear_servers(&self, service: &str) -> Result<()>;
}

#[derive(Default)]
pub struct MacOsSystemDnsPlatform;

impl MacOsSystemDnsPlatform {
    fn run(args: &[String]) -> Result<String> {
        #[cfg(not(target_os = "macos"))]
        bail!("系统 DNS 管理仅支持 macOS");

        #[cfg(target_os = "macos")]
        {
            let output = Command::new(NETWORK_SETUP)
                .args(args)
                .output()
                .with_context(|| format!("执行 {} 失败", NETWORK_SETUP))?;
            if !output.status.success() {
                bail!(
                    "networksetup 执行失败: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            String::from_utf8(output.stdout).context("networksetup 返回了非 UTF-8 内容")
        }
    }
}

impl SystemDnsPlatform for MacOsSystemDnsPlatform {
    fn list_network_services(&self) -> Result<Vec<String>> {
        let output = Self::run(&["-listallnetworkservices".to_string()])?;
        Ok(output
            .lines()
            .filter(|line| !line.starts_with("An asterisk"))
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('*'))
            .map(str::to_string)
            .collect())
    }

    fn get_servers(&self, service: &str) -> Result<Vec<IpAddr>> {
        let output = Self::run(&["-getdnsservers".to_string(), service.to_string()])?;
        if output
            .trim()
            .starts_with("There aren't any DNS Servers set")
        {
            return Ok(Vec::new());
        }
        output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| {
                line.parse::<IpAddr>()
                    .with_context(|| format!("networksetup 返回了无效 DNS 地址: {}", line))
            })
            .collect()
    }

    fn set_servers(&self, service: &str, servers: &[IpAddr]) -> Result<()> {
        if servers.is_empty() {
            bail!("拒绝为网络服务设置空 DNS 列表");
        }
        let mut args = vec!["-setdnsservers".to_string(), service.to_string()];
        args.extend(servers.iter().map(ToString::to_string));
        Self::run(&args)?;
        Ok(())
    }

    fn clear_servers(&self, service: &str) -> Result<()> {
        Self::run(&[
            "-setdnsservers".to_string(),
            service.to_string(),
            "Empty".to_string(),
        ])?;
        Ok(())
    }
}

pub struct SystemDnsSupervisor<P: SystemDnsPlatform> {
    platform: Arc<P>,
    config: Arc<RwLock<SystemDnsConfig>>,
    config_changed: Arc<Notify>,
    operation_lock: Arc<AsyncMutex<()>>,
    shutting_down: AtomicBool,
}

impl<P: SystemDnsPlatform> SystemDnsSupervisor<P> {
    pub fn new(platform: Arc<P>, config: SystemDnsConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            platform,
            config: Arc::new(RwLock::new(config)),
            config_changed: Arc::new(Notify::new()),
            operation_lock: Arc::new(AsyncMutex::new(())),
            shutting_down: AtomicBool::new(false),
        })
    }

    pub async fn config(&self) -> SystemDnsConfig {
        self.config.read().await.clone()
    }

    pub async fn update_config(&self, config: SystemDnsConfig) -> Result<()> {
        config.validate()?;
        *self.config.write().await = config;
        self.config_changed.notify_waiters();
        Ok(())
    }

    pub async fn inspect(&self) -> Result<Vec<NetworkServiceDns>> {
        let _operation = self.operation_lock.lock().await;
        self.inspect_without_lock().await
    }

    pub async fn enforce_once(&self) -> Result<Vec<NetworkServiceDns>> {
        let _operation = self.operation_lock.lock().await;
        let config = self.config().await;
        let platform = self.platform.clone();
        let should_enforce = !self.shutting_down.load(Ordering::Acquire);
        tokio::task::spawn_blocking(move || {
            if should_enforce {
                for binding in config.bindings.iter().filter(|binding| binding.enabled) {
                    let current = platform.get_servers(&binding.service)?;
                    if current != binding.preset_servers {
                        platform.set_servers(&binding.service, &binding.preset_servers)?;
                        info!(service = %binding.service, "Corrected macOS DNS servers");
                    }
                }
            }
            Self::read_all_services(platform, config)
        })
        .await
        .context("系统 DNS 校正任务失败")?
    }

    pub async fn clear_service(&self, service: &str) -> Result<NetworkServiceDns> {
        let service = service.trim();
        if service.is_empty() {
            bail!("网络服务名称不能为空");
        }
        let _operation = self.operation_lock.lock().await;
        let config = self.config().await;
        let platform = self.platform.clone();
        let service_name = service.to_string();
        tokio::task::spawn_blocking(move || {
            platform.clear_servers(&service_name)?;
            info!(service = %service_name, "Cleared macOS DNS servers");
            Self::read_all_services(platform, config)?
                .into_iter()
                .find(|item| item.service == service_name)
                .with_context(|| format!("清空后未找到网络服务: {}", service_name))
        })
        .await
        .context("清空网络服务 DNS 的任务失败")?
    }

    pub async fn clear_configured_services_for_exit(&self) -> Result<()> {
        self.shutting_down.store(true, Ordering::Release);
        let result = self.clear_configured_services().await;
        if result.is_err() {
            self.shutting_down.store(false, Ordering::Release);
            self.config_changed.notify_waiters();
        }
        result
    }

    async fn clear_configured_services(&self) -> Result<()> {
        let _operation = self.operation_lock.lock().await;
        let services = self
            .config()
            .await
            .bindings
            .into_iter()
            .map(|binding| binding.service)
            .collect::<Vec<_>>();
        let platform = self.platform.clone();
        tokio::task::spawn_blocking(move || {
            let mut failures = Vec::new();
            for service in services {
                match platform.clear_servers(&service) {
                    Ok(()) => info!(service = %service, "Cleared macOS DNS servers before exit"),
                    Err(error) => failures.push(format!("{}: {}", service, error)),
                }
            }
            if failures.is_empty() {
                Ok(())
            } else {
                bail!("退出前清空系统 DNS 失败: {}", failures.join("；"))
            }
        })
        .await
        .context("退出前清空系统 DNS 的任务失败")?
    }

    async fn inspect_without_lock(&self) -> Result<Vec<NetworkServiceDns>> {
        let config = self.config().await;
        let platform = self.platform.clone();
        tokio::task::spawn_blocking(move || Self::read_all_services(platform, config))
            .await
            .context("读取网络服务 DNS 的任务失败")?
    }

    fn read_all_services(
        platform: Arc<P>,
        config: SystemDnsConfig,
    ) -> Result<Vec<NetworkServiceDns>> {
        let services = platform.list_network_services()?;
        let mut result = Vec::with_capacity(services.len());
        for service in services {
            let current_servers = platform.get_servers(&service)?;
            let binding = config
                .bindings
                .iter()
                .find(|binding| binding.service == service);
            let enabled = binding.is_some_and(|binding| binding.enabled);
            let preset_servers = binding
                .map(|binding| binding.preset_servers.clone())
                .unwrap_or_default();
            let matches_preset = enabled && current_servers == preset_servers;
            result.push(NetworkServiceDns {
                service,
                current_servers,
                enabled,
                preset_servers,
                matches_preset,
            });
        }
        Ok(result)
    }

    pub fn spawn(self: Arc<Self>) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                let interval_secs = self.config().await.check_interval_secs;
                if let Err(err) = self.enforce_once().await {
                    error!("Failed to enforce system DNS settings: {}", err);
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(interval_secs)) => {}
                    _ = self.config_changed.notified() => {}
                }
            }
        })
    }
}

pub type MacOsSystemDnsSupervisor = SystemDnsSupervisor<MacOsSystemDnsPlatform>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakePlatform {
        servers: Mutex<HashMap<String, Vec<IpAddr>>>,
    }

    impl SystemDnsPlatform for FakePlatform {
        fn list_network_services(&self) -> Result<Vec<String>> {
            let mut services = self
                .servers
                .lock()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            services.sort();
            Ok(services)
        }
        fn get_servers(&self, service: &str) -> Result<Vec<IpAddr>> {
            self.servers
                .lock()
                .unwrap()
                .get(service)
                .cloned()
                .with_context(|| format!("Unknown service: {}", service))
        }
        fn set_servers(&self, service: &str, servers: &[IpAddr]) -> Result<()> {
            let previous = self
                .servers
                .lock()
                .unwrap()
                .insert(service.to_string(), servers.to_vec());
            if previous.is_none() {
                bail!("Unknown service: {}", service);
            }
            Ok(())
        }

        fn clear_servers(&self, service: &str) -> Result<()> {
            self.set_servers(service, &[])
        }
    }

    fn ip(value: &str) -> IpAddr {
        value.parse().unwrap()
    }

    #[tokio::test]
    async fn each_service_uses_its_own_dns_list() {
        let platform = Arc::new(FakePlatform::default());
        platform
            .servers
            .lock()
            .unwrap()
            .insert("VPN".into(), vec![ip("8.8.8.8")]);
        platform
            .servers
            .lock()
            .unwrap()
            .insert("Wi-Fi".into(), vec![ip("9.9.9.9")]);
        let supervisor = SystemDnsSupervisor::new(
            platform,
            SystemDnsConfig {
                bindings: vec![
                    NetworkDnsBinding {
                        service: "VPN".into(),
                        enabled: true,
                        preset_servers: vec![ip("1.1.1.1")],
                    },
                    NetworkDnsBinding {
                        service: "Wi-Fi".into(),
                        enabled: true,
                        preset_servers: vec![ip("127.0.0.1"), ip("223.5.5.5")],
                    },
                ],
                check_interval_secs: 5,
            },
        )
        .unwrap();
        let status = supervisor.enforce_once().await.unwrap();
        assert_eq!(
            status
                .iter()
                .find(|item| item.service == "VPN")
                .unwrap()
                .current_servers,
            vec![ip("1.1.1.1")]
        );
        assert_eq!(
            status
                .iter()
                .find(|item| item.service == "Wi-Fi")
                .unwrap()
                .current_servers,
            vec![ip("127.0.0.1"), ip("223.5.5.5")]
        );
    }

    #[test]
    fn enabled_binding_requires_dns_servers() {
        let config = SystemDnsConfig {
            bindings: vec![NetworkDnsBinding {
                service: "Wi-Fi".into(),
                enabled: true,
                preset_servers: Vec::new(),
            }],
            check_interval_secs: 5,
        };
        assert!(config.validate().is_err());
    }
}
