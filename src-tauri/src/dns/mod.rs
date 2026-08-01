//! DNS module
//!
//! Contains DNS server implementations and related functionality.

pub mod cache;
pub mod message;
pub mod plane_state;
pub mod proxy;
pub mod resolver;
pub mod rewrite;
pub mod server;

pub use cache::*;
pub use plane_state::*;
pub use proxy::*;
pub use resolver::*;
pub use rewrite::*;
