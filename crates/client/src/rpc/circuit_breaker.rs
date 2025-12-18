//! Circuit Breaker Pattern Implementation
//!
//! Prevents cascading failures by temporarily stopping requests to
//! unhealthy endpoints.

use parking_lot::RwLock;
use std::time::{Duration, Instant};

use super::config::CircuitBreakerConfig;

/// Circuit breaker states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Circuit is closed, requests flow normally
    Closed,
    /// Circuit is open, requests are blocked
    Open,
    /// Circuit is half-open, allowing test requests
    HalfOpen,
}

/// Circuit breaker for an endpoint
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: RwLock<CircuitState>,
    failure_count: RwLock<u32>,
    success_count: RwLock<u32>,
    last_failure_time: RwLock<Option<Instant>>,
    last_state_change: RwLock<Instant>,
}

impl CircuitBreaker {
    /// Create a new circuit breaker with configuration
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            state: RwLock::new(CircuitState::Closed),
            failure_count: RwLock::new(0),
            success_count: RwLock::new(0),
            last_failure_time: RwLock::new(None),
            last_state_change: RwLock::new(Instant::now()),
        }
    }

    /// Create with default configuration
    pub fn default_config() -> Self {
        Self::new(CircuitBreakerConfig::default())
    }

    /// Get the current state
    pub fn state(&self) -> CircuitState {
        self.maybe_transition_from_open();
        *self.state.read()
    }

    /// Check if requests should be allowed
    pub fn should_allow_request(&self) -> bool {
        self.maybe_transition_from_open();

        let state = *self.state.read();
        matches!(state, CircuitState::Closed | CircuitState::HalfOpen)
    }

    /// Record a successful request
    pub fn record_success(&self) {
        let mut state = self.state.write();

        match *state {
            CircuitState::HalfOpen => {
                let mut success_count = self.success_count.write();
                *success_count += 1;

                if *success_count >= self.config.success_threshold {
                    // Enough successes in half-open, close the circuit
                    *state = CircuitState::Closed;
                    *self.last_state_change.write() = Instant::now();
                    *self.failure_count.write() = 0;
                    *success_count = 0;
                    log::info!("Circuit breaker closed after {} successes", self.config.success_threshold);
                }
            }
            CircuitState::Closed => {
                // Reset failure count on success
                *self.failure_count.write() = 0;
            }
            CircuitState::Open => {
                // Shouldn't happen, but ignore
            }
        }
    }

    /// Record a failed request
    pub fn record_failure(&self) {
        let mut state = self.state.write();
        *self.last_failure_time.write() = Some(Instant::now());

        match *state {
            CircuitState::Closed => {
                let mut failure_count = self.failure_count.write();
                *failure_count += 1;

                if *failure_count >= self.config.failure_threshold {
                    // Too many failures, open the circuit
                    *state = CircuitState::Open;
                    *self.last_state_change.write() = Instant::now();
                    log::warn!(
                        "Circuit breaker opened after {} failures",
                        self.config.failure_threshold
                    );
                }
            }
            CircuitState::HalfOpen => {
                // Failure in half-open, go back to open
                *state = CircuitState::Open;
                *self.last_state_change.write() = Instant::now();
                *self.success_count.write() = 0;
                log::warn!("Circuit breaker reopened due to failure in half-open state");
            }
            CircuitState::Open => {
                // Already open, ignore
            }
        }
    }

    /// Reset the circuit breaker to closed state
    pub fn reset(&self) {
        *self.state.write() = CircuitState::Closed;
        *self.failure_count.write() = 0;
        *self.success_count.write() = 0;
        *self.last_state_change.write() = Instant::now();
    }

    /// Get time since last state change
    pub fn time_in_current_state(&self) -> Duration {
        self.last_state_change.read().elapsed()
    }

    /// Get failure count
    pub fn failure_count(&self) -> u32 {
        *self.failure_count.read()
    }

    /// Check if we should transition from open to half-open
    fn maybe_transition_from_open(&self) {
        let state = *self.state.read();
        if state != CircuitState::Open {
            return;
        }

        let time_in_open = self.last_state_change.read().elapsed();
        if time_in_open >= self.config.reset_timeout {
            // Transition to half-open
            let mut state = self.state.write();
            if *state == CircuitState::Open {
                *state = CircuitState::HalfOpen;
                *self.last_state_change.write() = Instant::now();
                *self.success_count.write() = 0;
                log::info!(
                    "Circuit breaker transitioning to half-open after {:?}",
                    time_in_open
                );
            }
        }
    }
}

impl std::fmt::Debug for CircuitBreaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CircuitBreaker")
            .field("state", &self.state())
            .field("failure_count", &*self.failure_count.read())
            .field("success_count", &*self.success_count.read())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast_breaker() -> CircuitBreaker {
        CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 3,
            reset_timeout: Duration::from_millis(100),
            success_threshold: 2,
        })
    }

    #[test]
    fn test_starts_closed() {
        let cb = fast_breaker();
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.should_allow_request());
    }

    #[test]
    fn test_opens_after_failures() {
        let cb = fast_breaker();

        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);

        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);

        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);
        assert!(!cb.should_allow_request());
    }

    #[test]
    fn test_success_resets_failure_count() {
        let cb = fast_breaker();

        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.failure_count(), 2);

        cb.record_success();
        assert_eq!(cb.failure_count(), 0);
    }

    #[test]
    fn test_transitions_to_half_open() {
        let cb = fast_breaker();

        // Open the circuit
        for _ in 0..3 {
            cb.record_failure();
        }
        assert_eq!(cb.state(), CircuitState::Open);

        // Wait for reset timeout
        std::thread::sleep(Duration::from_millis(150));

        // Should now be half-open
        assert_eq!(cb.state(), CircuitState::HalfOpen);
        assert!(cb.should_allow_request());
    }

    #[test]
    fn test_half_open_closes_on_success() {
        let cb = fast_breaker();

        // Open and wait
        for _ in 0..3 {
            cb.record_failure();
        }
        std::thread::sleep(Duration::from_millis(150));

        // Now half-open
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        // 2 successes should close it
        cb.record_success();
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        cb.record_success();
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn test_half_open_reopens_on_failure() {
        let cb = fast_breaker();

        // Open and wait
        for _ in 0..3 {
            cb.record_failure();
        }
        std::thread::sleep(Duration::from_millis(150));

        // Now half-open
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        // Failure should reopen
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);
    }

    #[test]
    fn test_reset() {
        let cb = fast_breaker();

        // Open the circuit
        for _ in 0..3 {
            cb.record_failure();
        }
        assert_eq!(cb.state(), CircuitState::Open);

        // Reset
        cb.reset();
        assert_eq!(cb.state(), CircuitState::Closed);
        assert_eq!(cb.failure_count(), 0);
    }
}
