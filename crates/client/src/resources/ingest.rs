//! Per-endpoint ingest attribution.
//!
//! Datasources race: several WebSocket subscriptions deliver the same account
//! updates and clock ticks, and whichever arrives first wins. The cache
//! (`put_if_newer`) and the clock high-water mark are the arbitration points —
//! later duplicates are simply dropped.
//!
//! Racing is only worth its cost if some endpoint is actually winning. These
//! counters answer that: an endpoint winning 0% of races is pure spend, while
//! one winning a meaningful share is genuinely cutting the tail.

use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Minimum spacing between backfills across *all* datasources.
///
/// Each endpoint reconnecting independently would otherwise fire its own
/// unpaginated `getProgramAccounts`. Once backfilled accounts carry a real slot,
/// every snapshot after the first is rejected as a duplicate anyway — so the
/// extra calls buy nothing and cost a full program scan each.
const BACKFILL_DEBOUNCE: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
struct EndpointIngest {
    accounts_seen: AtomicU64,
    accounts_won: AtomicU64,
    clocks_seen: AtomicU64,
    clocks_won: AtomicU64,
    /// Total time this endpoint trailed the winner, over lost clock races.
    clock_lag_total_ms: AtomicU64,
    clock_lag_max_ms: AtomicU64,
    /// Successful connects. The first is startup; the rest are reconnects.
    connects: AtomicU64,
}

/// A point-in-time view of one endpoint's ingest performance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestSnapshot {
    pub endpoint: String,
    pub accounts_seen: u64,
    pub accounts_won: u64,
    pub clocks_seen: u64,
    pub clocks_won: u64,
    /// Mean milliseconds behind the winner, across lost clock races.
    pub clock_lag_avg_ms: u64,
    pub clock_lag_max_ms: u64,
}

impl IngestSnapshot {
    /// Share of clock ticks this endpoint delivered first, as a percentage.
    pub fn clock_win_pct(&self) -> u64 {
        if self.clocks_seen == 0 {
            return 0;
        }
        self.clocks_won * 100 / self.clocks_seen
    }
}

/// Shared across all datasource actors.
#[derive(Debug)]
pub struct IngestStats {
    per_endpoint: DashMap<String, EndpointIngest>,
    /// Highest clock slot any datasource has delivered, and when it landed.
    /// This is the arbitration point for the clock race.
    clock_high: Mutex<Option<(u64, Instant)>>,
    /// When any datasource last started a backfill.
    last_backfill: Mutex<Option<Instant>>,
}

impl Default for IngestStats {
    fn default() -> Self {
        Self::new()
    }
}

impl IngestStats {
    pub fn new() -> Self {
        Self {
            per_endpoint: DashMap::new(),
            clock_high: Mutex::new(None),
            last_backfill: Mutex::new(None),
        }
    }

    /// Record a successful connect. Returns the total connect count for this
    /// endpoint — 1 is startup, anything higher is a reconnect.
    pub fn record_connect(&self, endpoint: &str) -> u64 {
        self.entry(endpoint)
            .connects
            .fetch_add(1, Ordering::Relaxed)
            + 1
    }

    /// Claim the right to run a backfill, debounced across all datasources.
    ///
    /// Returns `false` if another endpoint backfilled recently, in which case
    /// the caller should skip.
    pub fn try_claim_backfill(&self) -> bool {
        let mut last = self.last_backfill.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        match *last {
            Some(prev) if now.duration_since(prev) < BACKFILL_DEBOUNCE => false,
            _ => {
                *last = Some(now);
                true
            }
        }
    }

    /// Record an account update. `won` is whether it was new to the cache.
    pub fn record_account(&self, endpoint: &str, won: bool) {
        let entry = self.entry(endpoint);
        entry.accounts_seen.fetch_add(1, Ordering::Relaxed);
        if won {
            entry.accounts_won.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a clock tick and decide whether this endpoint won the race.
    ///
    /// Returns `None` if this endpoint delivered the slot first, or
    /// `Some(lag)` describing how far behind the winner it arrived.
    pub fn record_clock(&self, endpoint: &str, slot: u64, seen_at: Instant) -> Option<Duration> {
        let lag = {
            let mut high = self.clock_high.lock().unwrap_or_else(|e| e.into_inner());
            match *high {
                Some((high_slot, _)) if slot <= high_slot => {
                    // Lost: someone already delivered this slot (or a later one).
                    let (_, won_at) = high.expect("checked above");
                    Some(seen_at.saturating_duration_since(won_at))
                }
                _ => {
                    *high = Some((slot, seen_at));
                    None
                }
            }
        };

        let entry = self.entry(endpoint);
        entry.clocks_seen.fetch_add(1, Ordering::Relaxed);
        match lag {
            None => {
                entry.clocks_won.fetch_add(1, Ordering::Relaxed);
            }
            Some(d) => {
                let ms = d.as_millis() as u64;
                entry.clock_lag_total_ms.fetch_add(ms, Ordering::Relaxed);
                entry.clock_lag_max_ms.fetch_max(ms, Ordering::Relaxed);
            }
        }
        lag
    }

    /// Snapshot every endpoint, ordered by clock wins descending.
    pub fn snapshot(&self) -> Vec<IngestSnapshot> {
        let mut out: Vec<IngestSnapshot> = self
            .per_endpoint
            .iter()
            .map(|e| {
                let v = e.value();
                let clocks_seen = v.clocks_seen.load(Ordering::Relaxed);
                let clocks_won = v.clocks_won.load(Ordering::Relaxed);
                let lost = clocks_seen.saturating_sub(clocks_won);
                let lag_total = v.clock_lag_total_ms.load(Ordering::Relaxed);
                IngestSnapshot {
                    endpoint: e.key().clone(),
                    accounts_seen: v.accounts_seen.load(Ordering::Relaxed),
                    accounts_won: v.accounts_won.load(Ordering::Relaxed),
                    clocks_seen,
                    clocks_won,
                    clock_lag_avg_ms: if lost == 0 { 0 } else { lag_total / lost },
                    clock_lag_max_ms: v.clock_lag_max_ms.load(Ordering::Relaxed),
                }
            })
            .collect();
        out.sort_by(|a, b| b.clocks_won.cmp(&a.clocks_won));
        out
    }

    /// True when more than one endpoint has delivered anything, i.e. there is
    /// actually a race to report on.
    pub fn is_racing(&self) -> bool {
        self.per_endpoint.len() > 1
    }

    fn entry(&self, endpoint: &str) -> dashmap::mapref::one::Ref<'_, String, EndpointIngest> {
        if !self.per_endpoint.contains_key(endpoint) {
            self.per_endpoint
                .entry(endpoint.to_string())
                .or_insert_with(EndpointIngest::default);
        }
        self.per_endpoint
            .get(endpoint)
            .expect("inserted immediately above")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_delivery_of_a_slot_wins() {
        let stats = IngestStats::new();
        let t0 = Instant::now();

        assert_eq!(stats.record_clock("fast", 100, t0), None);

        // A second endpoint delivering the same slot lost, by 40ms.
        let lag = stats
            .record_clock("slow", 100, t0 + Duration::from_millis(40))
            .expect("should have lost");
        assert_eq!(lag.as_millis(), 40);
    }

    #[test]
    fn older_slot_from_a_lagging_endpoint_also_loses() {
        let stats = IngestStats::new();
        let t0 = Instant::now();

        stats.record_clock("fast", 200, t0);
        assert!(stats
            .record_clock("slow", 199, t0 + Duration::from_millis(10))
            .is_some());
    }

    #[test]
    fn snapshot_reports_win_share_and_lag() {
        let stats = IngestStats::new();
        let t0 = Instant::now();

        // "fast" wins two of three; "slow" wins one, trailing 30ms then 50ms.
        stats.record_clock("fast", 1, t0);
        stats.record_clock("slow", 1, t0 + Duration::from_millis(30));
        stats.record_clock("fast", 2, t0 + Duration::from_millis(400));
        stats.record_clock("slow", 2, t0 + Duration::from_millis(450));
        stats.record_clock("slow", 3, t0 + Duration::from_millis(800));

        let snap = stats.snapshot();
        assert_eq!(snap.len(), 2);

        // Ordered by wins descending.
        assert_eq!(snap[0].endpoint, "fast");
        assert_eq!(snap[0].clocks_won, 2);
        assert_eq!(snap[0].clock_win_pct(), 100);

        assert_eq!(snap[1].endpoint, "slow");
        assert_eq!(snap[1].clocks_won, 1);
        assert_eq!(snap[1].clocks_seen, 3);
        assert_eq!(snap[1].clock_win_pct(), 33);
        assert_eq!(snap[1].clock_lag_avg_ms, 40); // (30 + 50) / 2
        assert_eq!(snap[1].clock_lag_max_ms, 50);
    }

    #[test]
    fn account_wins_are_counted() {
        let stats = IngestStats::new();
        stats.record_account("a", true);
        stats.record_account("a", false);
        stats.record_account("b", false);

        let snap = stats.snapshot();
        let a = snap.iter().find(|s| s.endpoint == "a").unwrap();
        assert_eq!(a.accounts_seen, 2);
        assert_eq!(a.accounts_won, 1);

        let b = snap.iter().find(|s| s.endpoint == "b").unwrap();
        assert_eq!(b.accounts_won, 0);
    }

    #[test]
    fn backfill_is_debounced_across_endpoints() {
        let stats = IngestStats::new();

        // First claim succeeds; a second endpoint reconnecting immediately
        // afterwards must not trigger another full program scan.
        assert!(stats.try_claim_backfill());
        assert!(!stats.try_claim_backfill());
        assert!(!stats.try_claim_backfill());
    }

    #[test]
    fn connect_count_distinguishes_startup_from_reconnect() {
        let stats = IngestStats::new();
        assert_eq!(stats.record_connect("a"), 1); // startup
        assert_eq!(stats.record_connect("a"), 2); // reconnect
        assert_eq!(stats.record_connect("b"), 1); // separate endpoint
    }

    #[test]
    fn racing_requires_more_than_one_endpoint() {
        let stats = IngestStats::new();
        assert!(!stats.is_racing());
        stats.record_account("only", true);
        assert!(!stats.is_racing());
        stats.record_account("second", true);
        assert!(stats.is_racing());
    }
}
