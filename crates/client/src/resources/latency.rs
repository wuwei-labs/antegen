//! Rolling execution-latency percentiles.
//!
//! The per-execution `antegen::latency` line is the ground truth, but reading a
//! distribution out of it means shipping logs somewhere and parsing them — which
//! is how the before/after numbers for past changes were produced, by hand. A
//! node that reports its own p50/p90/p99 makes every run self-measuring,
//! including the load generator.
//!
//! Deliberately a bounded window rather than a full histogram: the question is
//! "how is this node behaving now", and a distribution that includes the
//! backlog a node drained hours ago answers a different one.

use std::collections::VecDeque;
use std::sync::Mutex;

/// How many recent executions the window holds.
///
/// At the default concurrency this is a few minutes of a busy node and rather
/// longer than the heartbeat interval, so consecutive heartbeats overlap rather
/// than each reporting a disjoint handful of samples.
const WINDOW: usize = 1024;

#[derive(Debug, Default)]
pub struct LatencyStats {
    /// Lag in milliseconds for executions that landed, newest pushed at the back.
    lags: Mutex<VecDeque<i64>>,
}

/// A snapshot of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LatencySnapshot {
    pub count: usize,
    pub p50: i64,
    pub p90: i64,
    pub p99: i64,
    pub max: i64,
}

impl LatencyStats {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the lag of an execution that landed on-chain.
    ///
    /// Only landed executions: a load-balancer skip or a failure carries a lag
    /// that measures nothing anyone acted on, and mixing those in makes the
    /// percentiles describe a distribution that does not exist.
    pub fn record(&self, lag_ms: i64) {
        let mut lags = self.lags.lock().unwrap_or_else(|e| e.into_inner());
        if lags.len() == WINDOW {
            lags.pop_front();
        }
        lags.push_back(lag_ms);
    }

    /// Percentiles over the current window, or `None` when nothing has landed
    /// yet — reporting zeros for an idle node would read as "instant".
    pub fn snapshot(&self) -> Option<LatencySnapshot> {
        let lags = self.lags.lock().unwrap_or_else(|e| e.into_inner());
        if lags.is_empty() {
            return None;
        }

        let mut sorted: Vec<i64> = lags.iter().copied().collect();
        sorted.sort_unstable();

        let at = |q: f64| -> i64 {
            let idx = ((sorted.len() as f64) * q) as usize;
            sorted[idx.min(sorted.len() - 1)]
        };

        Some(LatencySnapshot {
            count: sorted.len(),
            p50: at(0.50),
            p90: at(0.90),
            p99: at(0.99),
            max: *sorted.last().expect("non-empty"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_window_reports_nothing() {
        assert_eq!(LatencyStats::new().snapshot(), None);
    }

    #[test]
    fn percentiles_come_from_the_recorded_lags() {
        let s = LatencyStats::new();
        for lag in 1..=100 {
            s.record(lag);
        }

        let snap = s.snapshot().unwrap();
        assert_eq!(snap.count, 100);
        assert_eq!(snap.p50, 51);
        assert_eq!(snap.p90, 91);
        assert_eq!(snap.p99, 100);
        assert_eq!(snap.max, 100);
    }

    /// The window is what makes this describe the node now rather than the
    /// backlog it drained hours ago.
    #[test]
    fn old_samples_fall_out_of_the_window() {
        let s = LatencyStats::new();

        // A drained backlog: one enormous lag, then a full window of healthy ones.
        s.record(59_195_166);
        for _ in 0..WINDOW {
            s.record(400);
        }

        let snap = s.snapshot().unwrap();
        assert_eq!(snap.count, WINDOW);
        assert_eq!(snap.max, 400, "the outlier should have aged out");
    }

    #[test]
    fn order_of_arrival_does_not_matter() {
        let a = LatencyStats::new();
        let b = LatencyStats::new();
        for lag in [900, 100, 500, 300, 700] {
            a.record(lag);
        }
        for lag in [100, 300, 500, 700, 900] {
            b.record(lag);
        }
        assert_eq!(a.snapshot(), b.snapshot());
    }
}
