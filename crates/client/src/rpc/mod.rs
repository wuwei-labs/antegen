//! Custom RPC client with safe deserialization
//!
//! Handles the u64::MAX -> float serialization issue in Solana RPC responses.
//!
//! ## Architecture
//!
//! - `response` - Safe response types with custom deserialization
//! - `config` - Configuration types for pool and endpoints
//! - `endpoint` - Individual endpoint state and health tracking
//! - `pool` - Core RPC pool implementation
//! - `circuit_breaker` - Circuit breaker pattern for fault tolerance
//! - `rate_limiter` - Token bucket rate limiting
//! - `health` - Background health checking
//! - `websocket` - Persistent WebSocket subscriptions using pws

pub mod circuit_breaker;
pub mod config;
pub mod endpoint;
pub mod health;
pub mod pool;
pub mod rate_limiter;
pub mod response;
pub mod websocket;

pub use circuit_breaker::*;
pub use config::*;
pub use endpoint::*;
pub use health::*;
pub use pool::*;
pub use rate_limiter::*;
pub use response::*;
pub use websocket::*;
