//! DNS Proxy Module
//!
//! Provides DNS proxy functionality including:
//! - Upstream server management
//! - Multiple protocol support (UDP, DoT, DoH, DoQ)
//! - Query strategies (concurrent, fastest, round-robin, random)
//! - Failover handling

mod client;
mod strategy;
mod upstream;

#[cfg(test)]
mod forwarding_tests;

#[allow(unused_imports)]
pub use client::*;
pub use strategy::*;
pub use upstream::*;
