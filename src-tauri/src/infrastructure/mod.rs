//! Infrastructure layer
//!
//! Technical capabilities: persistence, configuration, logging, listener
//! lifecycle and runtime monitoring.

pub mod common;
pub mod config;
pub mod listener_manager;
pub mod log;
pub mod monitor;
pub mod repository;

pub mod system_dns;

pub mod privileged_udp;
