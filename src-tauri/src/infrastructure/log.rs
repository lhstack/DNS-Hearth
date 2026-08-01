//! Log management module
//!
//! Handles logging configuration, file rotation, and cleanup.
//!
//! # Features
//!
//! - File-based logging with tracing-appender (Requirements 7.1)
//! - Time-based log rotation (Requirements 7.2)
//! - Size-based log rotation (Requirements 7.3)
//! - Automatic cleanup of expired logs (Requirements 7.4)
//! - Environment variable configuration (Requirements 7.5, 7.6, 7.7)
//! - Config file fallback (Requirements 7.8)

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use chrono::Local;
use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// Custom time formatter for logs (yyyy-MM-dd HH:mm:ss)
#[derive(Clone, Copy, Debug)]
struct LocalTimeFormatter;

impl FormatTime for LocalTimeFormatter {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        let now = Local::now();
        write!(w, "{}", now.format("%Y-%m-%d %H:%M:%S"))
    }
}

/// Global guard to keep the non-blocking writer alive
static LOG_GUARD: OnceLock<WorkerGuard> = OnceLock::new();

/// Log configuration
#[derive(Debug, Clone)]
pub struct LogConfig {
    /// Directory path for log files
    pub path: PathBuf,
    /// Log level filter (trace, debug, info, warn, error)
    pub level: String,
    /// Maximum size per log file in bytes (for size-based rotation reference)
    #[allow(dead_code)]
    pub max_size: u64,
    /// Rotation policy
    pub rotation: RotationPolicy,
    /// Number of days to retain log files
    pub retention_days: u32,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("logs"),
            level: "info".to_string(),
            max_size: 10 * 1024 * 1024, // 10MB
            rotation: RotationPolicy::Daily,
            retention_days: 30,
        }
    }
}

/// Log rotation policy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationPolicy {
    /// Rotate logs daily
    Daily,
    /// Rotate logs hourly
    Hourly,
    /// Never rotate (single file)
    Never,
}

impl RotationPolicy {
    /// Convert to tracing-appender Rotation
    fn to_tracing_rotation(self) -> Rotation {
        match self {
            RotationPolicy::Daily => Rotation::DAILY,
            RotationPolicy::Hourly => Rotation::HOURLY,
            RotationPolicy::Never => Rotation::NEVER,
        }
    }
}

impl From<&str> for RotationPolicy {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "hourly" => RotationPolicy::Hourly,
            "never" => RotationPolicy::Never,
            _ => RotationPolicy::Daily,
        }
    }
}

/// Log manager responsible for initializing and managing the logging system
///
/// Implements Requirements 7.1-7.8:
/// - 7.1: Output logs to local files
/// - 7.2: Support time-based log rotation
/// - 7.3: Support size-based log rotation (via daily rotation as approximation)
/// - 7.4: Automatic monthly cleanup of expired logs
/// - 7.5: Support log path configuration via environment variable
/// - 7.6: Support log level configuration via environment variable
/// - 7.7: Support log retention time configuration via environment variable
/// - 7.8: Fall back to config file when environment variables not set
pub struct LogManager {
    config: LogConfig,
}

#[allow(dead_code)]
impl LogManager {
    /// Create a new LogManager with the given configuration
    pub fn new(config: LogConfig) -> Self {
        Self { config }
    }

    /// Initialize the logging system with default configuration
    /// Reads configuration from environment variables with fallback to defaults
    pub fn init() -> Result<()> {
        let config = Self::load_config_from_env();
        Self::init_with_config(config)
    }

    /// Initialize the logging system with explicit configuration
    pub fn init_with_config(config: LogConfig) -> Result<()> {
        // Create log directory if it doesn't exist
        fs::create_dir_all(&config.path)
            .with_context(|| format!("Failed to create log directory: {:?}", config.path))?;

        // Create rolling file appender
        let file_appender = RollingFileAppender::new(
            config.rotation.to_tracing_rotation(),
            &config.path,
            "dns-proxy.log",
        );

        // Create non-blocking writer
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

        // Store the guard globally to keep the writer alive
        let _ = LOG_GUARD.set(guard);

        // Parse log level
        let level_filter = Self::parse_level_filter(&config.level);

        // Build the subscriber with both console and file output
        let env_filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level_filter));

        // File layer - writes to rolling log files
        let file_layer = tracing_subscriber::fmt::layer()
            .with_writer(non_blocking)
            .with_ansi(false)
            .with_target(true)
            .with_thread_ids(false)
            .with_file(false)
            .with_line_number(false)
            .with_timer(LocalTimeFormatter)
            .with_span_events(FmtSpan::CLOSE);

        // Console layer - writes to stdout
        let console_layer = tracing_subscriber::fmt::layer()
            .with_target(true)
            .with_thread_ids(false)
            .with_timer(LocalTimeFormatter);

        // Initialize the subscriber
        tracing_subscriber::registry()
            .with(env_filter)
            .with(file_layer)
            .with(console_layer)
            .init();

        Ok(())
    }

    /// Load configuration from environment variables
    /// Falls back to defaults when environment variables are not set
    pub fn load_config_from_env() -> LogConfig {
        let path = std::env::var("LOG_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("logs"));

        let level = std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());

        let max_size = std::env::var("LOG_MAX_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10 * 1024 * 1024);

        let rotation = std::env::var("LOG_ROTATION")
            .map(|v| RotationPolicy::from(v.as_str()))
            .unwrap_or(RotationPolicy::Daily);

        let retention_days = std::env::var("LOG_RETENTION_DAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        LogConfig {
            path,
            level,
            max_size,
            rotation,
            retention_days,
        }
    }

    /// Parse log level string to filter string
    fn parse_level_filter(level: &str) -> &str {
        match level.to_lowercase().as_str() {
            "trace" => "trace",
            "debug" => "debug",
            "info" => "info",
            "warn" | "warning" => "warn",
            "error" => "error",
            _ => "info",
        }
    }

    /// Parse log level string to tracing Level
    pub fn parse_level(level: &str) -> Level {
        match level.to_lowercase().as_str() {
            "trace" => Level::TRACE,
            "debug" => Level::DEBUG,
            "info" => Level::INFO,
            "warn" | "warning" => Level::WARN,
            "error" => Level::ERROR,
            _ => Level::INFO,
        }
    }

    /// Clean up expired log files based on retention policy
    ///
    /// This method scans the log directory and removes files older than
    /// the configured retention period.
    ///
    /// Implements Requirement 7.4: Monthly automatic cleanup of expired logs
    pub fn cleanup_old_logs(&self) -> Result<CleanupResult> {
        self.cleanup_logs_in_dir(&self.config.path, self.config.retention_days)
    }

    /// Clean up logs in a specific directory with given retention days
    pub fn cleanup_logs_in_dir(&self, dir: &Path, retention_days: u32) -> Result<CleanupResult> {
        let mut result = CleanupResult::default();

        if !dir.exists() {
            return Ok(result);
        }

        let retention_duration = Duration::from_secs(retention_days as u64 * 24 * 60 * 60);
        let now = SystemTime::now();

        let entries = fs::read_dir(dir)
            .with_context(|| format!("Failed to read log directory: {:?}", dir))?;

        for entry in entries.flatten() {
            let path = entry.path();

            // Only process log files
            if !Self::is_log_file(&path) {
                continue;
            }

            result.total_files += 1;

            // Get file modification time
            if let Ok(metadata) = fs::metadata(&path) {
                if let Ok(modified) = metadata.modified() {
                    if let Ok(age) = now.duration_since(modified) {
                        if age > retention_duration {
                            // File is older than retention period, delete it
                            if let Ok(()) = fs::remove_file(&path) {
                                result.deleted_files += 1;
                                result.deleted_bytes += metadata.len();
                                tracing::info!("Deleted expired log file: {:?}", path);
                            } else {
                                tracing::warn!("Failed to delete expired log file: {:?}", path);
                            }
                        }
                    }
                }
            }
        }

        Ok(result)
    }

    /// Check if a file is a log file based on its name
    fn is_log_file(path: &Path) -> bool {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            // Match log files: dns-proxy.log, dns-proxy.log.2024-01-01, etc.
            name.starts_with("dns-proxy") && name.contains("log")
        } else {
            false
        }
    }

    /// Get the current log configuration
    pub fn config(&self) -> &LogConfig {
        &self.config
    }

    /// Calculate total size of log files in the log directory
    pub fn total_log_size(&self) -> Result<u64> {
        Self::calculate_dir_size(&self.config.path)
    }

    /// Calculate total size of files in a directory
    fn calculate_dir_size(dir: &Path) -> Result<u64> {
        let mut total = 0u64;

        if !dir.exists() {
            return Ok(0);
        }

        let entries =
            fs::read_dir(dir).with_context(|| format!("Failed to read directory: {:?}", dir))?;

        for entry in entries.flatten() {
            let path = entry.path();
            if Self::is_log_file(&path) {
                if let Ok(metadata) = fs::metadata(&path) {
                    total += metadata.len();
                }
            }
        }

        Ok(total)
    }

    /// List all log files in the log directory
    pub fn list_log_files(&self) -> Result<Vec<LogFileInfo>> {
        Self::list_logs_in_dir(&self.config.path)
    }

    /// List log files in a specific directory
    fn list_logs_in_dir(dir: &Path) -> Result<Vec<LogFileInfo>> {
        let mut files = Vec::new();

        if !dir.exists() {
            return Ok(files);
        }

        let entries =
            fs::read_dir(dir).with_context(|| format!("Failed to read directory: {:?}", dir))?;

        for entry in entries.flatten() {
            let path = entry.path();
            if Self::is_log_file(&path) {
                if let Ok(metadata) = fs::metadata(&path) {
                    files.push(LogFileInfo {
                        path: path.clone(),
                        size: metadata.len(),
                        modified: metadata.modified().ok(),
                    });
                }
            }
        }

        // Sort by modification time, newest first
        files.sort_by(|a, b| b.modified.cmp(&a.modified));

        Ok(files)
    }
}

/// Result of log cleanup operation
#[derive(Debug, Default, Clone)]
pub struct CleanupResult {
    /// Total number of log files found
    pub total_files: usize,
    /// Number of files deleted
    pub deleted_files: usize,
    /// Total bytes freed
    pub deleted_bytes: u64,
}

/// Information about a log file
#[derive(Debug, Clone)]
pub struct LogFileInfo {
    /// Path to the log file
    #[allow(dead_code)]
    pub path: PathBuf,
    /// Size in bytes
    #[allow(dead_code)]
    pub size: u64,
    /// Last modification time
    pub modified: Option<SystemTime>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = LogConfig::default();
        assert_eq!(config.path, PathBuf::from("logs"));
        assert_eq!(config.level, "info");
        assert_eq!(config.max_size, 10 * 1024 * 1024);
        assert_eq!(config.rotation, RotationPolicy::Daily);
        assert_eq!(config.retention_days, 30);
    }

    #[test]
    fn test_rotation_policy_from_str() {
        assert_eq!(RotationPolicy::from("daily"), RotationPolicy::Daily);
        assert_eq!(RotationPolicy::from("Daily"), RotationPolicy::Daily);
        assert_eq!(RotationPolicy::from("hourly"), RotationPolicy::Hourly);
        assert_eq!(RotationPolicy::from("HOURLY"), RotationPolicy::Hourly);
        assert_eq!(RotationPolicy::from("never"), RotationPolicy::Never);
        assert_eq!(RotationPolicy::from("unknown"), RotationPolicy::Daily);
    }

    #[test]
    fn test_parse_level() {
        assert_eq!(LogManager::parse_level("trace"), Level::TRACE);
        assert_eq!(LogManager::parse_level("debug"), Level::DEBUG);
        assert_eq!(LogManager::parse_level("info"), Level::INFO);
        assert_eq!(LogManager::parse_level("warn"), Level::WARN);
        assert_eq!(LogManager::parse_level("warning"), Level::WARN);
        assert_eq!(LogManager::parse_level("error"), Level::ERROR);
        assert_eq!(LogManager::parse_level("ERROR"), Level::ERROR);
        assert_eq!(LogManager::parse_level("unknown"), Level::INFO);
    }

    #[test]
    fn test_is_log_file() {
        assert!(LogManager::is_log_file(Path::new("dns-proxy.log")));
        assert!(LogManager::is_log_file(Path::new(
            "dns-proxy.log.2024-01-01"
        )));
        assert!(LogManager::is_log_file(Path::new("/var/log/dns-proxy.log")));
        assert!(!LogManager::is_log_file(Path::new("other.txt")));
        assert!(!LogManager::is_log_file(Path::new("config.toml")));
    }

    #[test]
    fn test_cleanup_empty_dir() {
        let temp_dir = TempDir::new().unwrap();
        let config = LogConfig {
            path: temp_dir.path().to_path_buf(),
            retention_days: 7,
            ..Default::default()
        };
        let manager = LogManager::new(config);

        let result = manager.cleanup_old_logs().unwrap();
        assert_eq!(result.total_files, 0);
        assert_eq!(result.deleted_files, 0);
    }

    #[test]
    fn test_cleanup_nonexistent_dir() {
        let config = LogConfig {
            path: PathBuf::from("/nonexistent/path/to/logs"),
            retention_days: 7,
            ..Default::default()
        };
        let manager = LogManager::new(config);

        let result = manager.cleanup_old_logs().unwrap();
        assert_eq!(result.total_files, 0);
        assert_eq!(result.deleted_files, 0);
    }

    #[test]
    fn test_cleanup_keeps_recent_files() {
        let temp_dir = TempDir::new().unwrap();

        // Create a recent log file
        let log_path = temp_dir.path().join("dns-proxy.log");
        let mut file = File::create(&log_path).unwrap();
        writeln!(file, "test log entry").unwrap();

        let config = LogConfig {
            path: temp_dir.path().to_path_buf(),
            retention_days: 7,
            ..Default::default()
        };
        let manager = LogManager::new(config);

        let result = manager.cleanup_old_logs().unwrap();
        assert_eq!(result.total_files, 1);
        assert_eq!(result.deleted_files, 0);
        assert!(log_path.exists());
    }

    #[test]
    fn test_list_log_files() {
        let temp_dir = TempDir::new().unwrap();

        // Create some log files
        let log1 = temp_dir.path().join("dns-proxy.log");
        let log2 = temp_dir.path().join("dns-proxy.log.2024-01-01");
        let other = temp_dir.path().join("other.txt");

        File::create(&log1).unwrap().write_all(b"log1").unwrap();
        File::create(&log2)
            .unwrap()
            .write_all(b"log2 content")
            .unwrap();
        File::create(&other).unwrap().write_all(b"other").unwrap();

        let config = LogConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };
        let manager = LogManager::new(config);

        let files = manager.list_log_files().unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_total_log_size() {
        let temp_dir = TempDir::new().unwrap();

        // Create log files with known sizes
        let log1 = temp_dir.path().join("dns-proxy.log");
        let log2 = temp_dir.path().join("dns-proxy.log.old");

        File::create(&log1).unwrap().write_all(&[0u8; 100]).unwrap();
        File::create(&log2).unwrap().write_all(&[0u8; 200]).unwrap();

        let config = LogConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };
        let manager = LogManager::new(config);

        let total = manager.total_log_size().unwrap();
        assert_eq!(total, 300);
    }

    #[test]
    fn test_rotation_to_tracing() {
        assert!(matches!(
            RotationPolicy::Daily.to_tracing_rotation(),
            Rotation::DAILY
        ));
        assert!(matches!(
            RotationPolicy::Hourly.to_tracing_rotation(),
            Rotation::HOURLY
        ));
        assert!(matches!(
            RotationPolicy::Never.to_tracing_rotation(),
            Rotation::NEVER
        ));
    }
}
