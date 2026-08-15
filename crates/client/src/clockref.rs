//! Monotonic anchor mapping on-chain time to local time.
//!
//! The Clock sysvar's `unix_timestamp` is a coarse integer that steps roughly
//! once per second, while slots arrive ~2.5 times per second — so consecutive
//! ticks frequently carry the *same* timestamp. Interpolating between those
//! integer steps is meaningless. What is useful is recording **when the chain's
//! clock first reached a given value**, then projecting forward from there:
//!
//! ```text
//! instant_for_ts(T) = anchor_at + (T - anchor_ts) seconds
//! ```
//!
//! That turns "fire when `Schedule::Timed { next }` is reached" from a question
//! we can only answer when a WebSocket message happens to arrive into one we can
//! answer with a local timer.
//!
//! Used read-only for latency measurement first; it becomes the firing primitive
//! once scheduling moves onto a timer.

use solana_sdk::clock::Clock;
use std::time::{Duration, Instant};

/// How far a tick's slot may sit below the highest slot seen and still be
/// accepted. At `processed` commitment slots are not monotone — they can be
/// skipped or rolled back, and with several RPC providers racing, different
/// endpoints may briefly sit on different forks. This window is wide enough to
/// absorb that while still rejecting an endpoint that has fallen genuinely
/// behind.
const FORK_TOLERANCE_SLOTS: u64 = 64;

/// Maps on-chain `unix_timestamp` onto the local monotonic clock.
#[derive(Debug, Default)]
pub struct ClockRef {
    /// The highest `unix_timestamp` observed.
    anchor_ts: i64,
    /// Local instant at which `anchor_ts` was *first* seen.
    anchor_at: Option<Instant>,
    /// Slot that carried `anchor_ts`.
    anchor_slot: u64,
    /// Highest slot observed, used for fork rejection.
    high_slot: u64,
}

impl ClockRef {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a tick. Returns `false` if the tick was rejected as a stale fork.
    ///
    /// The anchor only ever moves forward: a tick carrying a lower
    /// `unix_timestamp` than we've already seen updates the slot high-water mark
    /// but leaves the time projection untouched, so the estimate can never jump
    /// backwards.
    pub fn observe(&mut self, clock: &Clock, seen_at: Instant) -> bool {
        if self.anchor_at.is_none() {
            self.anchor_ts = clock.unix_timestamp;
            self.anchor_at = Some(seen_at);
            self.anchor_slot = clock.slot;
            self.high_slot = clock.slot;
            return true;
        }

        // Reject a tick from an endpoint that has fallen far behind the chain.
        if clock.slot.saturating_add(FORK_TOLERANCE_SLOTS) < self.high_slot {
            return false;
        }

        self.high_slot = self.high_slot.max(clock.slot);

        // Re-anchor only on the first sighting of a *new* timestamp value.
        // Later ticks carrying the same second must not reset `anchor_at`, or
        // the projection would drift later by however long that second took to
        // tick over.
        if clock.unix_timestamp > self.anchor_ts {
            self.anchor_ts = clock.unix_timestamp;
            self.anchor_at = Some(seen_at);
            self.anchor_slot = clock.slot;
        }

        true
    }

    /// True once at least one tick has been observed.
    pub fn is_ready(&self) -> bool {
        self.anchor_at.is_some()
    }

    /// The most recent on-chain timestamp observed.
    pub fn anchor_ts(&self) -> i64 {
        self.anchor_ts
    }

    /// The slot that carried the current anchor.
    pub fn anchor_slot(&self) -> u64 {
        self.anchor_slot
    }

    /// Highest slot seen, across all datasources.
    pub fn high_slot(&self) -> u64 {
        self.high_slot
    }

    /// Estimated current on-chain unix timestamp, as a continuous value.
    pub fn now_ts(&self) -> Option<f64> {
        let at = self.anchor_at?;
        Some(self.anchor_ts as f64 + at.elapsed().as_secs_f64())
    }

    /// Estimated current on-chain unix timestamp, floored to whole seconds.
    ///
    /// Use this when evaluating readiness between ticks: the on-chain gate
    /// compares whole seconds, and the anchor alone would lag by up to a second
    /// because it only advances when a tick carrying a new value arrives.
    pub fn anchor_ts_projected(&self) -> i64 {
        match self.now_ts() {
            Some(ts) => ts.floor() as i64,
            None => self.anchor_ts,
        }
    }

    /// Local instant at which the on-chain clock is projected to reach `ts`.
    ///
    /// For a `ts` already in the past this returns an instant in the past, which
    /// is what callers want: `sleep_until` on it fires immediately.
    pub fn instant_for_ts(&self, ts: i64) -> Option<Instant> {
        let at = self.anchor_at?;
        let delta = ts - self.anchor_ts;
        Some(if delta >= 0 {
            at + Duration::from_secs(delta as u64)
        } else {
            at.checked_sub(Duration::from_secs(delta.unsigned_abs()))
                .unwrap_or(at)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock(slot: u64, unix_timestamp: i64) -> Clock {
        Clock {
            slot,
            unix_timestamp,
            ..Clock::default()
        }
    }

    #[test]
    fn first_observation_anchors() {
        let mut c = ClockRef::new();
        assert!(!c.is_ready());
        assert!(c.observe(&clock(100, 1_000), Instant::now()));
        assert!(c.is_ready());
        assert_eq!(c.anchor_ts(), 1_000);
        assert_eq!(c.high_slot(), 100);
    }

    #[test]
    fn repeated_timestamp_does_not_move_anchor() {
        let mut c = ClockRef::new();
        let t0 = Instant::now();
        c.observe(&clock(100, 1_000), t0);

        // Three more slots land within the same on-chain second.
        let later = t0 + Duration::from_millis(400);
        c.observe(&clock(101, 1_000), later);
        c.observe(&clock(102, 1_000), later + Duration::from_millis(400));

        // The projection for a future timestamp must still be measured from the
        // instant the second *began*, not from the latest tick.
        assert_eq!(c.instant_for_ts(1_001), Some(t0 + Duration::from_secs(1)));
        assert_eq!(c.high_slot(), 102);
    }

    #[test]
    fn new_timestamp_reanchors() {
        let mut c = ClockRef::new();
        let t0 = Instant::now();
        c.observe(&clock(100, 1_000), t0);

        let t1 = t0 + Duration::from_millis(900);
        c.observe(&clock(103, 1_001), t1);

        assert_eq!(c.anchor_ts(), 1_001);
        assert_eq!(c.instant_for_ts(1_002), Some(t1 + Duration::from_secs(1)));
    }

    #[test]
    fn backwards_timestamp_does_not_regress_projection() {
        let mut c = ClockRef::new();
        let t0 = Instant::now();
        c.observe(&clock(100, 1_005), t0);

        // A fork delivers an older timestamp at a nearby slot: accepted as a
        // tick, but it must not drag the projection backwards.
        assert!(c.observe(&clock(101, 1_002), t0 + Duration::from_millis(400)));
        assert_eq!(c.anchor_ts(), 1_005);
        assert_eq!(c.instant_for_ts(1_006), Some(t0 + Duration::from_secs(1)));
    }

    #[test]
    fn far_behind_slot_is_rejected() {
        let mut c = ClockRef::new();
        let t0 = Instant::now();
        c.observe(&clock(1_000, 1_000), t0);

        // A datasource lagging well beyond the fork window contributes nothing.
        assert!(!c.observe(&clock(500, 999), t0 + Duration::from_millis(10)));
        assert_eq!(c.high_slot(), 1_000);

        // Just inside the window is still accepted.
        assert!(c.observe(&clock(1_000 - FORK_TOLERANCE_SLOTS, 1_000), t0));
    }

    #[test]
    fn projected_timestamp_advances_between_ticks() {
        // The anchor only moves when a tick carrying a new second arrives, so
        // readiness evaluated on the raw anchor would lag by up to a second.
        // The projection must advance with local time instead.
        let mut c = ClockRef::new();
        let t0 = Instant::now() - Duration::from_millis(1_500);
        c.observe(&clock(100, 1_000), t0);

        assert_eq!(c.anchor_ts(), 1_000);
        assert_eq!(c.anchor_ts_projected(), 1_001);
    }

    #[test]
    fn projection_falls_back_to_anchor_before_any_tick() {
        let c = ClockRef::new();
        assert_eq!(c.anchor_ts_projected(), 0);
        assert!(c.now_ts().is_none());
        assert!(c.instant_for_ts(1_000).is_none());
    }

    #[test]
    fn past_timestamp_projects_into_the_past() {
        let mut c = ClockRef::new();
        let t0 = Instant::now();
        c.observe(&clock(100, 1_000), t0);
        let due = c.instant_for_ts(995).unwrap();
        assert!(due < t0);
    }
}
