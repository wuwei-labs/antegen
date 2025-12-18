//! RPC Pool Configuration
//!
//! Configuration types for the custom RPC client pool.

use std::time::Duration;

/// Main configuration for the RPC pool
#[derive(Debug, Clone)]
pub struct RpcPoolConfig {
    /// HTTP client settings
    pub http: HttpConfig,
    /// Circuit breaker settings
    pub circuit_breaker: CircuitBreakerConfig,
    /// Rate limiting settings
    pub rate_limit: RateLimitConfig,
    /// Health check settings
    pub health_check: HealthCheckConfig,
    /// Retry settings
    pub retry: RetryConfig,
    /// Load balancing strategy
    pub load_balance_strategy: LoadBalanceStrategy,
}

impl Default for RpcPoolConfig {
    fn default() -> Self {
        Self {
            http: HttpConfig::default(),
            circuit_breaker: CircuitBreakerConfig::default(),
            rate_limit: RateLimitConfig::default(),
            health_check: HealthCheckConfig::default(),
            retry: RetryConfig::default(),
            load_balance_strategy: LoadBalanceStrategy::RoundRobin,
        }
    }
}

/// HTTP client configuration
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// Connection timeout
    pub connect_timeout: Duration,
    /// Request timeout
    pub request_timeout: Duration,
    /// Pool idle timeout
    pub pool_idle_timeout: Duration,
    /// Max idle connections per host
    pub pool_max_idle_per_host: usize,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            pool_idle_timeout: Duration::from_secs(90),
            pool_max_idle_per_host: 10,
        }
    }
}

/// Circuit breaker configuration
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of failures before opening the circuit
    pub failure_threshold: u32,
    /// Duration the circuit stays open before testing
    pub reset_timeout: Duration,
    /// Number of successes in half-open state before closing
    pub success_threshold: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            reset_timeout: Duration::from_secs(30),
            success_threshold: 3,
        }
    }
}

/// Rate limiting configuration
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Requests per second per endpoint
    pub requests_per_second: f64,
    /// Burst capacity
    pub burst_capacity: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_second: 50.0,
            burst_capacity: 100,
        }
    }
}

/// Health check configuration
#[derive(Debug, Clone)]
pub struct HealthCheckConfig {
    /// Interval between health checks
    pub interval: Duration,
    /// Timeout for health check requests
    pub timeout: Duration,
    /// Failures before marking unhealthy
    pub unhealthy_threshold: u32,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(30),
            timeout: Duration::from_secs(5),
            unhealthy_threshold: 3,
        }
    }
}

/// Retry configuration
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum retry attempts
    pub max_attempts: u32,
    /// Initial backoff duration
    pub initial_backoff: Duration,
    /// Maximum backoff duration
    pub max_backoff: Duration,
    /// Backoff multiplier
    pub multiplier: f64,
    /// Jitter factor (0.0 - 1.0)
    pub jitter: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(10),
            multiplier: 2.0,
            jitter: 0.1,
        }
    }
}

/// Load balancing strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadBalanceStrategy {
    /// Simple round-robin
    RoundRobin,
    /// Weighted round-robin based on priority
    WeightedRoundRobin,
    /// Select endpoint with lowest latency
    LeastLatency,
    /// Always use highest priority healthy endpoint
    Priority,
}

/// Configuration for a single RPC endpoint
#[derive(Debug, Clone)]
pub struct EndpointConfig {
    /// HTTP URL for RPC calls
    pub url: String,
    /// WebSocket URL (derived from HTTP URL if not provided)
    pub ws_url: Option<String>,
    /// Priority (lower = higher priority, used for failover ordering)
    pub priority: u8,
    /// Role of this endpoint
    pub role: EndpointRole,
    /// Custom rate limit for this endpoint (overrides global)
    pub rate_limit: Option<RateLimitConfig>,
}

impl EndpointConfig {
    /// Create a new endpoint config with default settings
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ws_url: None,
            priority: 100,
            role: EndpointRole::Both,
            rate_limit: None,
        }
    }

    /// Set the priority
    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    /// Set the role
    pub fn with_role(mut self, role: EndpointRole) -> Self {
        self.role = role;
        self
    }

    /// Set custom WebSocket URL
    pub fn with_ws_url(mut self, ws_url: impl Into<String>) -> Self {
        self.ws_url = Some(ws_url.into());
        self
    }

    /// Derive WebSocket URL from HTTP URL
    pub fn ws_url(&self) -> String {
        self.ws_url.clone().unwrap_or_else(|| {
            self.url
                .replace("https://", "wss://")
                .replace("http://", "ws://")
        })
    }
}

/// Role of an endpoint in the pool
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointRole {
    /// Used for transaction submission only
    Submission,
    /// Used for data fetching/subscriptions only
    Datasource,
    /// Used for both submission and data fetching
    Both,
}

impl EndpointRole {
    /// Check if this role can be used for submission
    pub fn can_submit(&self) -> bool {
        matches!(self, EndpointRole::Submission | EndpointRole::Both)
    }

    /// Check if this role can be used as a datasource
    pub fn can_fetch(&self) -> bool {
        matches!(self, EndpointRole::Datasource | EndpointRole::Both)
    }
}

// Conversion from existing config types
impl From<&crate::config::RpcEndpoint> for EndpointConfig {
    fn from(endpoint: &crate::config::RpcEndpoint) -> Self {
        let role = match endpoint.role {
            crate::config::EndpointRole::Submission => EndpointRole::Submission,
            crate::config::EndpointRole::Datasource => EndpointRole::Datasource,
            crate::config::EndpointRole::Both => EndpointRole::Both,
        };

        Self {
            url: endpoint.url.clone(),
            ws_url: endpoint.ws_url.clone(),
            priority: endpoint.priority,
            role,
            rate_limit: None,
        }
    }
}

impl EndpointConfig {
    /// Create endpoint configs from the existing RpcConfig
    pub fn from_rpc_config(config: &crate::config::RpcConfig) -> Vec<Self> {
        config.endpoints.iter().map(|e| e.into()).collect()
    }
}
