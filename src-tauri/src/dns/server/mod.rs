//! DNS Server Module
//!
//! Provides DNS server implementations for multiple protocols:
//! - UDP: Standard DNS over UDP (port 53)
//! - DoT: DNS over TLS (port 853)
//! - DoH: DNS over HTTPS (port 443)
//! - DoQ: DNS over QUIC (port 853)
//! - DoH3: DNS over HTTP/3 (port 443)

mod doh;
mod doh3;
mod doq;
mod dot;
mod udp;

#[cfg(test)]
mod protocol_consistency_tests;

pub use doh::*;
#[allow(unused_imports)]
pub use doh3::*;
#[allow(unused_imports)]
pub use doq::*;
#[allow(unused_imports)]
pub use dot::*;
pub use udp::*;
