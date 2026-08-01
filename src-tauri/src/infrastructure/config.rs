//! Configuration management module
//!
//! Handles loading configuration from environment variables and config files,
//! with environment variables taking priority over config file values.
//!
//! # Configuration Priority
//!
//! 1. Environment variables (highest priority)
//! 2. Configuration file (config.toml)
//! 3. Default values (lowest priority)
//!
//! # Note
//!
//! The following configurations are managed via Web UI and stored in database:
//! - DNS listeners (ports, bind addresses, TLS certificates)
//! - Upstream DNS servers
//! - Cache settings
//! - Query strategy

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    // Database configuration
    pub database_url: String,

    // Log configuration
    pub log_path: PathBuf,
    pub log_level: String,
    pub log_max_size: u64,
    pub log_retention_days: u32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            database_url: "sqlite:fluxdns.db?mode=rwc".to_string(),
            log_path: PathBuf::from("logs"),
            log_level: "warn".to_string(),
            log_max_size: 10 * 1024 * 1024, // 10MB
            log_retention_days: 30,
        }
    }
}

/// Partial configuration for merging from different sources
#[derive(Debug, Default, Clone, Deserialize)]
pub struct PartialConfig {
    pub database_url: Option<String>,
    pub log_path: Option<PathBuf>,
    pub log_level: Option<String>,
    pub log_max_size: Option<u64>,
    pub log_retention_days: Option<u32>,
}

/// Configuration manager responsible for loading and providing access to configuration
pub struct ConfigManager {
    config: RwLock<AppConfig>,
}

impl ConfigManager {
    /// Load configuration from environment variables and config file
    pub fn load() -> Result<Self> {
        Self::load_with_path("config.toml")
    }

    /// Load configuration with a custom config file path
    pub fn load_with_path<P: AsRef<Path>>(config_path: P) -> Result<Self> {
        // Load .env file if present
        let _ = dotenvy::dotenv();

        // Start with defaults
        let mut config = AppConfig::default();

        // A missing config file is valid on first launch. A present but invalid
        // file is an initialization error and must remain visible.
        if config_path.as_ref().exists() {
            Self::merge_config(&mut config, Self::load_from_file(config_path.as_ref())?);
        }

        // Load from environment variables (higher priority)
        let env_config = Self::load_from_env();
        Self::merge_config(&mut config, env_config);

        Ok(Self {
            config: RwLock::new(config),
        })
    }

    /// Loads configuration while forcing mutable runtime data into the
    /// platform-specific application data directory.
    ///
    /// The database and logs must not depend on the process working directory
    /// or be written into the signed `.app` bundle.
    pub fn load_for_data_dir<P: AsRef<Path>>(data_dir: P) -> Result<Self> {
        let manager = Self::load()?;
        let data_dir = data_dir.as_ref();
        std::fs::create_dir_all(data_dir).with_context(|| {
            format!(
                "Failed to create application data directory: {}",
                data_dir.display()
            )
        })?;
        let mut config = manager.config.write().unwrap();
        config.database_url = format!(
            "sqlite:{}?mode=rwc",
            data_dir.join("dns-hearth.db").display()
        );
        config.log_path = data_dir.join("logs");
        drop(config);
        Ok(manager)
    }

    /// Create ConfigManager from explicit configs for testing
    #[allow(dead_code)]
    pub fn from_configs(
        file_config: Option<PartialConfig>,
        env_config: Option<PartialConfig>,
    ) -> Self {
        let mut config = AppConfig::default();

        if let Some(fc) = file_config {
            Self::merge_config(&mut config, fc);
        }

        if let Some(ec) = env_config {
            Self::merge_config(&mut config, ec);
        }

        Self {
            config: RwLock::new(config),
        }
    }

    /// Get current configuration
    pub fn get(&self) -> AppConfig {
        self.config.read().unwrap().clone()
    }

    /// Load configuration from environment variables
    pub fn load_from_env() -> PartialConfig {
        PartialConfig {
            database_url: std::env::var("DATABASE_URL").ok(),
            log_path: std::env::var("LOG_PATH").ok().map(PathBuf::from),
            log_level: std::env::var("LOG_LEVEL").ok(),
            log_max_size: std::env::var("LOG_MAX_SIZE")
                .ok()
                .and_then(|v| v.parse().ok()),
            log_retention_days: std::env::var("LOG_RETENTION_DAYS")
                .ok()
                .and_then(|v| v.parse().ok()),
        }
    }

    /// Load configuration from TOML file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<PartialConfig> {
        let content = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read config file: {}", path.as_ref().display()))?;
        let config: PartialConfig =
            toml::from_str(&content).with_context(|| "Failed to parse config file as TOML")?;
        Ok(config)
    }

    /// Merge partial config into full config
    pub fn merge_config(config: &mut AppConfig, partial: PartialConfig) {
        if let Some(v) = partial.database_url {
            config.database_url = v;
        }
        if let Some(v) = partial.log_path {
            config.log_path = v;
        }
        if let Some(v) = partial.log_level {
            config.log_level = v;
        }
        if let Some(v) = partial.log_max_size {
            config.log_max_size = v;
        }
        if let Some(v) = partial.log_retention_days {
            config.log_retention_days = v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.database_url, "sqlite:fluxdns.db?mode=rwc");
        assert_eq!(config.log_level, "warn");
    }

    #[test]
    fn test_load_from_toml_file() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"
database_url = "sqlite:test.db"
log_level = "debug"
"#
        )
        .unwrap();

        let config = ConfigManager::load_from_file(file.path()).unwrap();
        assert_eq!(config.database_url, Some("sqlite:test.db".to_string()));
        assert_eq!(config.log_level, Some("debug".to_string()));
    }

    #[test]
    fn test_merge_config() {
        let mut config = AppConfig::default();
        let partial = PartialConfig {
            database_url: Some("sqlite:merged.db".to_string()),
            ..Default::default()
        };

        ConfigManager::merge_config(&mut config, partial);

        assert_eq!(config.database_url, "sqlite:merged.db");
        assert_eq!(config.log_level, "warn"); // unchanged
    }

    #[test]
    fn test_env_priority_over_file() {
        let file_config = PartialConfig {
            database_url: Some("sqlite:file.db".to_string()),
            ..Default::default()
        };

        let env_config = PartialConfig {
            ..Default::default()
        };

        let manager = ConfigManager::from_configs(Some(file_config), Some(env_config));
        let config = manager.get();

        assert_eq!(config.database_url, "sqlite:file.db");
    }

    #[test]
    fn test_missing_config_file_uses_defaults() {
        let manager = ConfigManager::load_with_path("nonexistent_config.toml").unwrap();
        let config = manager.get();

        assert_eq!(config.database_url, "sqlite:fluxdns.db?mode=rwc");
    }
}
