//! Per-execution latency trace.
//!
//! One `ExecTrace` is created when the staging actor observes that a thread's
//! trigger deadline has been crossed, and travels with that execution attempt
//! through the processor and worker until it terminates. On completion it is
//! rendered as a single structured line.
//!
//! The trace is anchored on the **trigger deadline**, not on the account update
//! that scheduled the thread. Anchoring on ingest would report an hour of
//! "latency" for an hourly cron thread, when that hour is correct waiting.
//!
//! All timings are `Instant`s: monotonic, immune to wall-clock adjustment, and
//! ~20ns to record.

use solana_sdk::pubkey::Pubkey;
use std::fmt::Write as _;
use std::time::Instant;

/// Which submission paths accepted the transaction.
///
/// Deliberately *not* "which one landed it": the same signed transaction is
/// broadcast over every available path, so at most one can land but the client
/// cannot tell which. `Tpu` here means the TPU client accepted the transaction
/// for delivery — a fire-and-forget enqueue that succeeds even when the
/// underlying QUIC connection is failing — so reporting it as the landing path
/// would be wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendPath {
    Tpu,
    Rpc,
    /// Accepted by both; which one the leader actually took is unknowable here.
    Both,
}

impl SendPath {
    fn as_str(self) -> &'static str {
        match self {
            SendPath::Tpu => "tpu",
            SendPath::Rpc => "rpc",
            SendPath::Both => "both",
        }
    }

    /// Combine with a path already recorded for this attempt.
    fn merge(self, other: SendPath) -> SendPath {
        if self == other {
            self
        } else {
            SendPath::Both
        }
    }
}

/// How an execution attempt finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Landed on-chain.
    Ok,
    /// Nothing to submit (empty fiber).
    Skip,
    /// Declined by the load balancer.
    LbSkip,
    /// Attempted and failed.
    Fail,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Skip => "skip",
            Outcome::LbSkip => "lbskip",
            Outcome::Fail => "fail",
        }
    }
}

/// Signed millisecond delta. Signed because with an early-fire bias `sent` can
/// legitimately precede `due_at`.
fn delta_ms(from: Instant, to: Instant) -> i64 {
    if to >= from {
        to.duration_since(from).as_millis() as i64
    } else {
        -(from.duration_since(to).as_millis() as i64)
    }
}

fn opt_delta(from: Instant, to: Option<Instant>) -> String {
    match to {
        Some(t) => delta_ms(from, t).to_string(),
        None => "-".to_string(),
    }
}

/// Timeline of a single execution attempt.
#[derive(Debug, Clone)]
pub struct ExecTrace {
    pub thread: Pubkey,
    /// The thread's exec_count when this attempt was dispatched.
    pub exec_count: u64,
    /// `Schedule::Timed { next }` — the on-chain deadline, in unix seconds.
    pub due_ts: i64,
    /// Projected local instant of that deadline, via `ClockRef`. `None` when no
    /// clock tick has been observed yet, or for slot/epoch triggers which have
    /// no wall-clock deadline.
    pub due_at: Option<Instant>,
    /// When staging decided the thread was ready.
    pub released: Instant,
    pub spawned: Option<Instant>,
    pub built: Option<Instant>,
    pub simulated: Option<Instant>,
    pub signed: Option<Instant>,
    pub sent: Option<Instant>,
    pub settled: Option<Instant>,
    pub path: Option<SendPath>,
    /// RPC round trips consumed by this attempt.
    pub rpc_calls: u16,
    /// Submission attempts made.
    pub attempts: u32,
}

impl ExecTrace {
    pub fn new(thread: Pubkey, exec_count: u64, due_ts: i64, due_at: Option<Instant>) -> Self {
        Self {
            thread,
            exec_count,
            due_ts,
            due_at,
            released: Instant::now(),
            spawned: None,
            built: None,
            simulated: None,
            signed: None,
            sent: None,
            settled: None,
            path: None,
            rpc_calls: 0,
            attempts: 0,
        }
    }

    pub fn mark_spawned(&mut self) {
        self.spawned = Some(Instant::now());
    }

    pub fn mark_built(&mut self) {
        self.built = Some(Instant::now());
    }

    pub fn mark_simulated(&mut self) {
        self.simulated = Some(Instant::now());
    }

    pub fn mark_signed(&mut self) {
        self.signed = Some(Instant::now());
    }

    /// Record that a submission path accepted the transaction.
    ///
    /// The first accepting path sets `sent`; later ones only widen `path`, so
    /// the timestamp reflects when the transaction first reached the network.
    pub fn mark_sent(&mut self, path: SendPath) {
        if self.sent.is_none() {
            self.sent = Some(Instant::now());
        }
        self.path = Some(match self.path {
            Some(existing) => existing.merge(path),
            None => path,
        });
    }

    pub fn mark_settled(&mut self) {
        self.settled = Some(Instant::now());
    }

    pub fn count_rpc(&mut self) {
        self.rpc_calls = self.rpc_calls.saturating_add(1);
    }

    /// Milliseconds from the trigger deadline to submission — the headline
    /// number. `None` when the deadline had no projected instant, or when
    /// nothing was ever sent.
    pub fn lag_ms(&self) -> Option<i64> {
        Some(delta_ms(self.due_at?, self.sent?))
    }

    /// Milliseconds from the trigger deadline to staging noticing — the cost of
    /// the tick source.
    pub fn tick_ms(&self) -> Option<i64> {
        Some(delta_ms(self.due_at?, self.released))
    }

    /// Render the single structured line. Deltas, not absolutes: absolutes are
    /// not comparable across runs, and the whole point is to attribute the
    /// latency to a segment.
    pub fn render(&self, outcome: Outcome) -> String {
        let mut s = String::with_capacity(256);
        let _ = write!(
            s,
            "thread={} due={} outcome={}",
            self.thread,
            self.due_ts,
            outcome.as_str()
        );

        match self.lag_ms() {
            Some(v) => {
                let _ = write!(s, " lag_ms={}", v);
            }
            None => s.push_str(" lag_ms=-"),
        }
        match self.tick_ms() {
            Some(v) => {
                let _ = write!(s, " tick_ms={}", v);
            }
            None => s.push_str(" tick_ms=-"),
        }

        let _ = write!(
            s,
            " queue_ms={} build_ms={} sim_ms={} sign_ms={} send_ms={} settle_ms={}",
            opt_delta(self.released, self.spawned),
            // Each segment is measured from the previous mark that was actually
            // recorded, so a skipped stage shows "-" rather than silently
            // folding its cost into the next segment.
            match self.spawned {
                Some(from) => opt_delta(from, self.built),
                None => "-".to_string(),
            },
            match self.built {
                Some(from) => opt_delta(from, self.simulated),
                None => "-".to_string(),
            },
            match self.simulated.or(self.built) {
                Some(from) => opt_delta(from, self.signed),
                None => "-".to_string(),
            },
            match self.signed {
                Some(from) => opt_delta(from, self.sent),
                None => "-".to_string(),
            },
            match self.sent {
                Some(from) => opt_delta(from, self.settled),
                None => "-".to_string(),
            },
        );

        let _ = write!(
            s,
            " rpc={} attempts={} path={}",
            self.rpc_calls,
            self.attempts,
            self.path.map(SendPath::as_str).unwrap_or("-")
        );

        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn trace_at(due_at: Instant) -> ExecTrace {
        ExecTrace {
            thread: Pubkey::new_unique(),
            exec_count: 0,
            due_ts: 1_700_000_000,
            due_at: Some(due_at),
            released: due_at + Duration::from_millis(600),
            spawned: None,
            built: None,
            simulated: None,
            signed: None,
            sent: None,
            settled: None,
            path: None,
            rpc_calls: 0,
            attempts: 0,
        }
    }

    #[test]
    fn lag_and_tick_measured_from_deadline() {
        let due = Instant::now();
        let mut t = trace_at(due);
        t.sent = Some(due + Duration::from_millis(1_400));

        assert_eq!(t.tick_ms(), Some(600));
        assert_eq!(t.lag_ms(), Some(1_400));
    }

    #[test]
    fn early_send_reports_negative_lag() {
        let due = Instant::now() + Duration::from_secs(10);
        let mut t = trace_at(due);
        t.sent = Some(due - Duration::from_millis(120));

        assert_eq!(t.lag_ms(), Some(-120));
    }

    #[test]
    fn both_paths_accepting_is_reported_as_both() {
        // The transaction is broadcast over every available path, so a single
        // path name would imply an attribution the client cannot make.
        let due = Instant::now();
        let mut t = trace_at(due);
        t.mark_sent(SendPath::Tpu);
        let first = t.sent;
        t.mark_sent(SendPath::Rpc);

        assert_eq!(t.path, Some(SendPath::Both));
        assert_eq!(t.sent, first, "sent marks first acceptance, not the last");
        assert!(t.render(Outcome::Ok).contains("path=both"));
    }

    #[test]
    fn a_single_path_is_reported_alone() {
        let due = Instant::now();
        let mut t = trace_at(due);
        t.mark_sent(SendPath::Rpc);
        t.mark_sent(SendPath::Rpc);
        assert_eq!(t.path, Some(SendPath::Rpc));
    }

    #[test]
    fn missing_marks_render_as_dashes() {
        let due = Instant::now();
        let t = trace_at(due);
        let line = t.render(Outcome::Fail);

        assert!(line.contains("lag_ms=-"), "{line}");
        assert!(line.contains("tick_ms=600"), "{line}");
        assert!(line.contains("queue_ms=-"), "{line}");
        assert!(line.contains("path=-"), "{line}");
        assert!(line.contains("outcome=fail"), "{line}");
    }

    #[test]
    fn full_timeline_renders_every_segment() {
        let due = Instant::now();
        let mut t = trace_at(due);
        t.spawned = Some(t.released + Duration::from_millis(2));
        t.built = Some(t.released + Duration::from_millis(52));
        t.simulated = Some(t.released + Duration::from_millis(102));
        t.signed = Some(t.released + Duration::from_millis(104));
        t.sent = Some(t.released + Duration::from_millis(110));
        t.settled = Some(t.released + Duration::from_millis(900));
        t.path = Some(SendPath::Tpu);
        t.rpc_calls = 2;
        t.attempts = 1;

        let line = t.render(Outcome::Ok);
        assert!(line.contains("queue_ms=2"), "{line}");
        assert!(line.contains("build_ms=50"), "{line}");
        assert!(line.contains("sim_ms=50"), "{line}");
        assert!(line.contains("sign_ms=2"), "{line}");
        assert!(line.contains("send_ms=6"), "{line}");
        assert!(line.contains("settle_ms=790"), "{line}");
        assert!(line.contains("rpc=2"), "{line}");
        assert!(line.contains("path=tpu"), "{line}");
    }

    #[test]
    fn sign_segment_falls_back_to_build_when_simulate_skipped() {
        // Once compute units are reused from the batching simulate, the separate
        // simulate mark disappears; sign_ms must then measure from `built`
        // rather than reporting "-".
        let due = Instant::now();
        let mut t = trace_at(due);
        t.spawned = Some(t.released);
        t.built = Some(t.released + Duration::from_millis(40));
        t.signed = Some(t.released + Duration::from_millis(45));

        let line = t.render(Outcome::Ok);
        assert!(line.contains("sim_ms=-"), "{line}");
        assert!(line.contains("sign_ms=5"), "{line}");
    }
}
