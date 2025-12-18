//! RPC Endpoint State Management
//!
//! Tracks health, latency, and status of individual RPC endpoints.

use parking_lot::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::config::{EndpointConfig, EndpointRole};

/// Health status of an endpoint
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointHealth {
    /// Endpoint is healthy and accepting requests
    Healthy,
    /// Endpoint is degraded (high latency or partial failures)
    Degraded,
    /// Endpoint is unhealthy and should not receive traffic
    Unhealthy,
}

/// Statistics for an endpoint
#[derive(Debug, Clone)]
pub struct EndpointStats {
    /// Total requests made to this endpoint
    pub total_requests: u64,
    /// Total successful requests
    pub successful_requests: u64,
    /// Total failed requests
    pub failed_requests: u64,
    /// Average latency in milliseconds
    pub avg_latency_ms: f64,
    /// Current health status
    pub health: EndpointHealth,
    /// Time since last successful request
    pub last_success: Option<Duration>,
    /// Time since last failure
    pub last_failure: Option<Duration>,
}

/// Tracks the state of a single RPC endpoint
pub struct EndpointState {
    /// Configuration for this endpoint
    pub config: EndpointConfig,
    /// Current health status
    health: RwLock<EndpointHealth>,
    /// Consecutive failure count
    consecutive_failures: AtomicU64,
    /// Consecutive success count (used in half-open state)
    consecutive_successes: AtomicU64,
    /// Total request count
    total_requests: AtomicU64,
    /// Successful request count
    successful_requests: AtomicU64,
    /// Failed request count
    failed_requests: AtomicU64,
    /// Rolling average latency in microseconds
    avg_latency_us: AtomicU64,
    /// Last successful request time
    last_success: RwLock<Option<Instant>>,
    /// Last failure time
    last_failure: RwLock<Option<Instant>>,
    /// Latency sample count for averaging
    latency_samples: AtomicU64,
}

impl EndpointState {
    /// Create a new endpoint state from configuration
    pub fn new(config: EndpointConfig) -> Self {
        Self {
            config,
            health: RwLock::new(EndpointHealth::Healthy),
            consecutive_failures: AtomicU64::new(0),
            consecutive_successes: AtomicU64::new(0),
            total_requests: AtomicU64::new(0),
            successful_requests: AtomicU64::new(0),
            failed_requests: AtomicU64::new(0),
            avg_latency_us: AtomicU64::new(0),
            last_success: RwLock::new(None),
            last_failure: RwLock::new(None),
            latency_samples: AtomicU64::new(0),
        }
    }

    /// Get the HTTP URL for this endpoint
    pub fn url(&self) -> &str {
        &self.config.url
    }

    /// Get the WebSocket URL for this endpoint
    pub fn ws_url(&self) -> String {
        self.config.ws_url()
    }

    /// Get the priority of this endpoint
    pub fn priority(&self) -> u8 {
        self.config.priority
    }

    /// Get the role of this endpoint
    pub fn role(&self) -> EndpointRole {
        self.config.role
    }

    /// Check if this endpoint can be used for transaction submission
    pub fn can_submit(&self) -> bool {
        self.config.role.can_submit()
    }

    /// Check if this endpoint can be used for data fetching
    pub fn can_fetch(&self) -> bool {
        self.config.role.can_fetch()
    }

    /// Get current health status
    pub fn health(&self) -> EndpointHealth {
        *self.health.read()
    }

    /// Check if endpoint is healthy enough to receive requests
    pub fn is_available(&self) -> bool {
        matches!(
            self.health(),
            EndpointHealth::Healthy | EndpointHealth::Degraded
        )
    }

    /// Record a successful request
    pub fn record_success(&self, latency: Duration) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.successful_requests.fetch_add(1, Ordering::Relaxed);
        self.consecutive_successes.fetch_add(1, Ordering::Relaxed);
        self.consecutive_failures.store(0, Ordering::Relaxed);

        // Update latency (exponential moving average)
        self.update_latency(latency);

        // Update last success time
        *self.last_success.write() = Some(Instant::now());

        // Potentially upgrade health
        self.maybe_upgrade_health();
    }

    /// Record a failed request
    pub fn record_failure(&self) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.failed_requests.fetch_add(1, Ordering::Relaxed);
        self.consecutive_failures.fetch_add(1, Ordering::Relaxed);
        self.consecutive_successes.store(0, Ordering::Relaxed);

        // Update last failure time
        *self.last_failure.write() = Some(Instant::now());

        // Potentially downgrade health
        self.maybe_downgrade_health();
    }

    /// Get the current average latency
    pub fn avg_latency(&self) -> Duration {
        Duration::from_micros(self.avg_latency_us.load(Ordering::Relaxed))
    }

    /// Get statistics for this endpoint
    pub fn stats(&self) -> EndpointStats {
        let now = Instant::now();
        EndpointStats {
            total_requests: self.total_requests.load(Ordering::Relaxed),
            successful_requests: self.successful_requests.load(Ordering::Relaxed),
            failed_requests: self.failed_requests.load(Ordering::Relaxed),
            avg_latency_ms: self.avg_latency_us.load(Ordering::Relaxed) as f64 / 1000.0,
            health: self.health(),
            last_success: self.last_success.read().map(|t| now.duration_since(t)),
            last_failure: self.last_failure.read().map(|t| now.duration_since(t)),
        }
    }

    /// Manually mark endpoint as unhealthy
    pub fn mark_unhealthy(&self) {
        *self.health.write() = EndpointHealth::Unhealthy;
    }

    /// Manually mark endpoint as degraded
    pub fn mark_degraded(&self) {
        *self.health.write() = EndpointHealth::Degraded;
    }

    /// Manually mark endpoint as healthy
    pub fn mark_healthy(&self) {
        *self.health.write() = EndpointHealth::Healthy;
        self.consecutive_failures.store(0, Ordering::Relaxed);
    }

    /// Reset statistics
    pub fn reset_stats(&self) {
        self.total_requests.store(0, Ordering::Relaxed);
        self.successful_requests.store(0, Ordering::Relaxed);
        self.failed_requests.store(0, Ordering::Relaxed);
        self.avg_latency_us.store(0, Ordering::Relaxed);
        self.latency_samples.store(0, Ordering::Relaxed);
        self.consecutive_failures.store(0, Ordering::Relaxed);
        self.consecutive_successes.store(0, Ordering::Relaxed);
    }

    /// Update latency with exponential moving average
    fn update_latency(&self, latency: Duration) {
        let latency_us = latency.as_micros() as u64;
        let samples = self.latency_samples.fetch_add(1, Ordering::Relaxed);

        if samples == 0 {
            self.avg_latency_us.store(latency_us, Ordering::Relaxed);
        } else {
            // Exponential moving average with alpha = 0.2
            let current = self.avg_latency_us.load(Ordering::Relaxed);
            let new_avg = (current as f64 * 0.8 + latency_us as f64 * 0.2) as u64;
            self.avg_latency_us.store(new_avg, Ordering::Relaxed);
        }
    }

    /// Check if health should be upgraded based on consecutive successes
    fn maybe_upgrade_health(&self) {
        let successes = self.consecutive_successes.load(Ordering::Relaxed);
        let mut health = self.health.write();

        match *health {
            EndpointHealth::Unhealthy if successes >= 3 => {
                *health = EndpointHealth::Degraded;
            }
            EndpointHealth::Degraded if successes >= 5 => {
                *health = EndpointHealth::Healthy;
            }
            _ => {}
        }
    }

    /// Check if health should be downgraded based on consecutive failures
    fn maybe_downgrade_health(&self) {
        let failures = self.consecutive_failures.load(Ordering::Relaxed);
        let mut health = self.health.write();

        match *health {
            EndpointHealth::Healthy if failures >= 3 => {
                *health = EndpointHealth::Degraded;
            }
            EndpointHealth::Degraded if failures >= 5 => {
                *health = EndpointHealth::Unhealthy;
            }
            _ => {}
        }
    }
}

impl std::fmt::Debug for EndpointState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EndpointState")
            .field("url", &self.config.url)
            .field("health", &self.health())
            .field("priority", &self.config.priority)
            .field("role", &self.config.role)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_endpoint() -> EndpointState {
        EndpointState::new(EndpointConfig::new("https://api.devnet.solana.com"))
    }

    #[test]
    fn test_new_endpoint_is_healthy() {
        let endpoint = test_endpoint();
        assert_eq!(endpoint.health(), EndpointHealth::Healthy);
        assert!(endpoint.is_available());
    }

    #[test]
    fn test_record_success_updates_stats() {
        let endpoint = test_endpoint();
        endpoint.record_success(Duration::from_millis(100));

        let stats = endpoint.stats();
        assert_eq!(stats.total_requests, 1);
        assert_eq!(stats.successful_requests, 1);
        assert_eq!(stats.failed_requests, 0);
        assert!(stats.avg_latency_ms > 0.0);
    }

    #[test]
    fn test_record_failure_updates_stats() {
        let endpoint = test_endpoint();
        endpoint.record_failure();

        let stats = endpoint.stats();
        assert_eq!(stats.total_requests, 1);
        assert_eq!(stats.successful_requests, 0);
        assert_eq!(stats.failed_requests, 1);
    }

    #[test]
    fn test_consecutive_failures_degrade_health() {
        let endpoint = test_endpoint();

        // 3 failures -> Degraded
        for _ in 0..3 {
            endpoint.record_failure();
        }
        assert_eq!(endpoint.health(), EndpointHealth::Degraded);

        // 5 more failures -> Unhealthy
        for _ in 0..5 {
            endpoint.record_failure();
        }
        assert_eq!(endpoint.health(), EndpointHealth::Unhealthy);
    }

    #[test]
    fn test_consecutive_successes_upgrade_health() {
        let endpoint = test_endpoint();
        endpoint.mark_unhealthy();

        // 3 successes -> Degraded
        for _ in 0..3 {
            endpoint.record_success(Duration::from_millis(50));
        }
        assert_eq!(endpoint.health(), EndpointHealth::Degraded);

        // 5 more successes -> Healthy
        for _ in 0..5 {
            endpoint.record_success(Duration::from_millis(50));
        }
        assert_eq!(endpoint.health(), EndpointHealth::Healthy);
    }

    #[test]
    fn test_success_resets_failure_count() {
        let endpoint = test_endpoint();

        // 2 failures
        endpoint.record_failure();
        endpoint.record_failure();
        assert_eq!(endpoint.health(), EndpointHealth::Healthy);

        // 1 success resets
        endpoint.record_success(Duration::from_millis(50));

        // 2 more failures shouldn't degrade (only 2 consecutive now)
        endpoint.record_failure();
        endpoint.record_failure();
        assert_eq!(endpoint.health(), EndpointHealth::Healthy);
    }

    #[test]
    fn test_latency_averaging() {
        let endpoint = test_endpoint();

        endpoint.record_success(Duration::from_millis(100));
        let initial = endpoint.avg_latency();

        endpoint.record_success(Duration::from_millis(200));
        let updated = endpoint.avg_latency();

        // EMA should be between 100 and 200
        assert!(updated > initial);
        assert!(updated < Duration::from_millis(200));
    }

    #[test]
    fn test_ws_url_derivation() {
        let endpoint = EndpointState::new(EndpointConfig::new("https://api.devnet.solana.com"));
        assert_eq!(endpoint.ws_url(), "wss://api.devnet.solana.com");

        let endpoint = EndpointState::new(EndpointConfig::new("http://localhost:8899"));
        assert_eq!(endpoint.ws_url(), "ws://localhost:8899");
    }

    #[test]
    fn test_role_filtering() {
        let submit_only =
            EndpointState::new(EndpointConfig::new("https://submit.example.com").with_role(EndpointRole::Submission));
        assert!(submit_only.can_submit());
        assert!(!submit_only.can_fetch());

        let fetch_only =
            EndpointState::new(EndpointConfig::new("https://fetch.example.com").with_role(EndpointRole::Datasource));
        assert!(!fetch_only.can_submit());
        assert!(fetch_only.can_fetch());

        let both = EndpointState::new(EndpointConfig::new("https://both.example.com").with_role(EndpointRole::Both));
        assert!(both.can_submit());
        assert!(both.can_fetch());
    }
}
