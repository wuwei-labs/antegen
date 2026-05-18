use std::time::Duration;

/// Reconnect backoff policy.
///
/// Defaults to `Backoff::exponential(100ms, 30s, 2.0)`, which matches
/// the cadence antegen-client wants for Solana RPC subscriptions
/// (snappy initial retry, capped well below the validator's connection
/// keepalive horizon).
#[derive(Debug, Clone)]
pub struct Backoff {
    pub initial: Duration,
    pub max: Duration,
    pub factor: f64,
}

impl Backoff {
    pub fn exponential(initial: Duration, max: Duration, factor: f64) -> Self {
        Self {
            initial,
            max,
            factor,
        }
    }

    pub fn constant(d: Duration) -> Self {
        Self {
            initial: d,
            max: d,
            factor: 1.0,
        }
    }

    /// Returns the delay for the given retry `attempt` (1-indexed).
    pub fn delay(&self, attempt: u64) -> Duration {
        if attempt == 0 {
            return Duration::ZERO;
        }
        let exp = (attempt - 1) as i32;
        let scaled = self.initial.as_secs_f64() * self.factor.powi(exp);
        let max = self.max.as_secs_f64();
        Duration::from_secs_f64(scaled.min(max).max(0.0))
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self::exponential(Duration::from_millis(100), Duration::from_secs(30), 2.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponential_grows_then_caps() {
        let b = Backoff::exponential(Duration::from_millis(100), Duration::from_secs(1), 2.0);
        assert_eq!(b.delay(0), Duration::ZERO);
        assert_eq!(b.delay(1), Duration::from_millis(100));
        assert_eq!(b.delay(2), Duration::from_millis(200));
        assert_eq!(b.delay(3), Duration::from_millis(400));
        assert_eq!(b.delay(4), Duration::from_millis(800));
        assert_eq!(b.delay(5), Duration::from_secs(1)); // capped
        assert_eq!(b.delay(20), Duration::from_secs(1)); // still capped
    }

    #[test]
    fn constant_does_not_grow() {
        let b = Backoff::constant(Duration::from_millis(250));
        assert_eq!(b.delay(1), Duration::from_millis(250));
        assert_eq!(b.delay(100), Duration::from_millis(250));
    }
}
