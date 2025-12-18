//! Token Bucket Rate Limiter
//!
//! Limits request rate per endpoint to prevent overwhelming RPCs.

use parking_lot::Mutex;
use std::time::{Duration, Instant};

use super::config::RateLimitConfig;

/// Token bucket rate limiter
pub struct RateLimiter {
    /// Configuration
    config: RateLimitConfig,
    /// Current tokens available
    tokens: Mutex<f64>,
    /// Last refill time
    last_refill: Mutex<Instant>,
}

impl RateLimiter {
    /// Create a new rate limiter with configuration
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            tokens: Mutex::new(config.burst_capacity as f64),
            last_refill: Mutex::new(Instant::now()),
            config,
        }
    }

    /// Create with default configuration
    pub fn default_config() -> Self {
        Self::new(RateLimitConfig::default())
    }

    /// Try to acquire a permit (non-blocking)
    ///
    /// Returns true if a token was acquired, false if rate limited.
    pub fn try_acquire(&self) -> bool {
        self.refill();

        let mut tokens = self.tokens.lock();
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Acquire a permit, waiting if necessary
    ///
    /// Returns the duration waited.
    pub async fn acquire(&self) -> Duration {
        let start = Instant::now();

        loop {
            self.refill();

            {
                let mut tokens = self.tokens.lock();
                if *tokens >= 1.0 {
                    *tokens -= 1.0;
                    return start.elapsed();
                }
            }

            // Calculate wait time for next token
            let wait = Duration::from_secs_f64(1.0 / self.config.requests_per_second);
            tokio::time::sleep(wait.min(Duration::from_millis(10))).await;
        }
    }

    /// Get current available tokens
    pub fn available_tokens(&self) -> f64 {
        self.refill();
        *self.tokens.lock()
    }

    /// Get current requests per second setting
    pub fn requests_per_second(&self) -> f64 {
        self.config.requests_per_second
    }

    /// Get burst capacity
    pub fn burst_capacity(&self) -> u64 {
        self.config.burst_capacity
    }

    /// Refill tokens based on elapsed time
    fn refill(&self) {
        let mut last_refill = self.last_refill.lock();
        let mut tokens = self.tokens.lock();

        let now = Instant::now();
        let elapsed = now.duration_since(*last_refill);
        let elapsed_secs = elapsed.as_secs_f64();

        // Add tokens based on time elapsed
        let new_tokens = elapsed_secs * self.config.requests_per_second;
        *tokens = (*tokens + new_tokens).min(self.config.burst_capacity as f64);
        *last_refill = now;
    }
}

impl std::fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimiter")
            .field("tokens", &self.available_tokens())
            .field("rps", &self.config.requests_per_second)
            .field("burst", &self.config.burst_capacity)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast_limiter() -> RateLimiter {
        RateLimiter::new(RateLimitConfig {
            requests_per_second: 10.0,
            burst_capacity: 5,
        })
    }

    #[test]
    fn test_initial_burst() {
        let limiter = fast_limiter();

        // Should be able to make burst_capacity requests immediately
        for _ in 0..5 {
            assert!(limiter.try_acquire());
        }

        // Next request should fail
        assert!(!limiter.try_acquire());
    }

    #[test]
    fn test_refill() {
        let limiter = fast_limiter();

        // Use all tokens
        for _ in 0..5 {
            assert!(limiter.try_acquire());
        }

        // Wait for refill (at 10 rps, we get 1 token per 100ms)
        std::thread::sleep(Duration::from_millis(150));

        // Should have at least 1 token now
        assert!(limiter.try_acquire());
    }

    #[test]
    fn test_available_tokens() {
        let limiter = fast_limiter();
        assert!((limiter.available_tokens() - 5.0).abs() < 0.1);

        limiter.try_acquire();
        assert!((limiter.available_tokens() - 4.0).abs() < 0.1);
    }

    #[test]
    fn test_max_capacity() {
        let limiter = fast_limiter();

        // Wait to potentially overfill
        std::thread::sleep(Duration::from_millis(200));

        // Should still be capped at burst_capacity
        assert!(limiter.available_tokens() <= 5.0);
    }

    #[tokio::test]
    async fn test_async_acquire() {
        let limiter = RateLimiter::new(RateLimitConfig {
            requests_per_second: 100.0,  // Fast for testing
            burst_capacity: 2,
        });

        // Use burst
        limiter.try_acquire();
        limiter.try_acquire();

        // Next acquire should wait
        let start = Instant::now();
        limiter.acquire().await;
        let elapsed = start.elapsed();

        // Should have waited some time (but not too long due to high rps)
        assert!(elapsed >= Duration::from_millis(5));
    }
}
