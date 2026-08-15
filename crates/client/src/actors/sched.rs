//! Thread scheduling state.
//!
//! Pure and synchronous: no I/O, no actors, no clock reads beyond the `now`
//! passed in. All of the scheduler's real decisions live here so they can be
//! tested directly.
//!
//! # Never pop
//!
//! The central invariant is that a tracked thread **always** has an entry with a
//! due value. Dispatching transitions its phase; it never removes it. A thread
//! can therefore only leave the scheduler by being deleted on-chain.
//!
//! The previous design popped entries off a binary heap when they became ready
//! and relied on something else re-adding them. Anything that failed after the
//! pop but produced no on-chain state change — a build failure, a confirmation
//! timeout, an expired trigger window, an empty fiber — left the thread with no
//! entry and nothing to reschedule it. Recovery depended on a cache eviction
//! firing, which for moka is lazy and can be arbitrarily delayed on a quiet
//! cache.
//!
//! # Two clocks
//!
//! `due` is in chain units — a unix timestamp for time triggers, a slot or epoch
//! number otherwise — and answers "when does the chain consider this ready?".
//! `retry_after` is a local `Instant` and answers "when may we next *try*?". They
//! are deliberately separate: backoff after a failure is a property of this node,
//! not of the chain.

use solana_sdk::pubkey::Pubkey;
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

/// Which trigger clock an entry is scheduled against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Time,
    Slot,
    Epoch,
}

impl Kind {
    /// How long a dispatched entry may stay in flight before it is assumed lost
    /// and returned to `Due`, expressed in this kind's own units.
    fn stall_watchdog(self) -> u64 {
        match self {
            Kind::Time => 45,  // seconds
            Kind::Slot => 120, // slots (~48s)
            Kind::Epoch => 1,
        }
    }

    /// How far ahead a parked entry is re-armed, so a thread that stops
    /// producing account updates is still re-examined eventually.
    fn parked_watchdog(self) -> u64 {
        match self {
            Kind::Time => 120, // seconds
            Kind::Slot => 300, // slots (~2min)
            Kind::Epoch => 1,
        }
    }
}

/// What happened to a dispatched thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Landed on-chain. The resulting account update carries the next due value.
    Succeeded,
    /// The chain advanced under us — another executor got there first, or the
    /// thread's `exec_count` moved between dispatch and execution. Not our
    /// success, but the same scheduling consequence: park and wait for the
    /// update that is already on its way.
    Superseded,
    /// Declined by the load balancer — another executor owns it.
    LoadBalancerSkip,
    /// Nothing to submit. No on-chain state changed, so no update is coming.
    EmptyFiber,
    /// Failed in a way worth retrying.
    Retryable,
    /// Failed in a way that retrying cannot fix.
    Fatal,
}

/// Where an entry is in the dispatch cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Eligible for dispatch once `due` and `retry_after` are satisfied.
    Due,
    /// Handed to the processor; awaiting a completion report.
    InFlight { since: Instant },
    /// Executed or deliberately idle; waiting for an account update.
    Parked,
}

impl Entry {
    /// True while this entry is dispatched and awaiting a completion report.
    pub fn is_in_flight(&self) -> bool {
        matches!(self.phase, Phase::InFlight { .. })
    }
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub kind: Kind,
    /// Trigger value in chain units.
    pub due: u64,
    pub exec_count: u64,
    pub phase: Phase,
    pub attempts: u32,
    pub paused: bool,
    /// Earliest local instant at which this may be dispatched again.
    pub retry_after: Option<Instant>,
    /// The `due` value this entry was originally scheduled for, before any
    /// backoff or watchdog moved it. Overdue accounting — which drives
    /// load-balancer takeover and on-chain commission decay — must be measured
    /// against the real deadline, not against a retry time we chose.
    pub original_due: u64,
}

/// A thread selected for dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dispatched {
    pub pubkey: Pubkey,
    pub exec_count: u64,
    /// The thread's real trigger deadline.
    pub due: u64,
}

/// How long a parked thread waits for its account update before we fetch the
/// state ourselves.
///
/// Purely a safety net for an RPC that does not deliver program notifications;
/// a healthy subscription re-arms the thread in ~100ms. Set aggressively (1.5s)
/// this fires constantly on threads that were merely a little slow, and the
/// redundant fetches saturate the same RPC pool the execution path needs —
/// measurably worse under load than not having it at all. Long enough to stay
/// silent in normal operation, far shorter than the watchdog it backs up.
const REFRESH_AFTER: Duration = Duration::from_secs(10);

/// Cap on how many parked threads are refreshed in one pass, so a large fleet
/// cannot turn the safety net into a request storm.
const MAX_REFRESH_PER_PASS: usize = 32;

/// Backoff schedule for retryable failures: ~1s, 2s, 4s … capped.
fn retry_backoff(attempts: u32) -> Duration {
    const CAP: Duration = Duration::from_secs(60);
    Duration::from_secs(1)
        .saturating_mul(1u32.checked_shl(attempts.min(6)).unwrap_or(u32::MAX))
        .min(CAP)
}

#[derive(Debug, Default)]
pub struct Sched {
    /// Ordered index. Keyed by `(kind, due, pubkey)` so the earliest entry of a
    /// kind is the first in range, and so a given thread has exactly one key.
    order: BTreeMap<(Kind, u64, Pubkey), ()>,
    /// Authoritative state.
    entries: HashMap<Pubkey, Entry>,
}

impl Sched {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, pk: &Pubkey) -> Option<&Entry> {
        self.entries.get(pk)
    }

    pub fn contains(&self, pk: &Pubkey) -> bool {
        self.entries.contains_key(pk)
    }

    /// Number of entries of a given kind.
    pub fn len_of(&self, kind: Kind) -> usize {
        self.entries.values().filter(|e| e.kind == kind).count()
    }

    /// Every tracked thread.
    pub fn tracked(&self) -> impl Iterator<Item = Pubkey> + '_ {
        self.entries.keys().copied()
    }

    /// Entries eligible to run right now, ignoring their due value. A large
    /// backlog here means dispatch, not scheduling, is the bottleneck.
    pub fn count_due(&self) -> usize {
        self.entries
            .values()
            .filter(|e| e.phase == Phase::Due && !e.paused)
            .count()
    }

    /// Number of entries currently dispatched.
    pub fn in_flight(&self) -> usize {
        self.entries
            .values()
            .filter(|e| matches!(e.phase, Phase::InFlight { .. }))
            .count()
    }

    /// Insert or replace a thread's schedule.
    ///
    /// Replace-in-place: the stale ordered key is removed before the new one is
    /// inserted, so duplicates are structurally impossible and no compaction
    /// pass is needed.
    ///
    /// An advancing `exec_count` or a changed due value means the chain moved,
    /// which clears any backoff this node had applied.
    pub fn upsert(
        &mut self,
        pk: Pubkey,
        kind: Kind,
        due: u64,
        exec_count: u64,
        paused: bool,
    ) -> bool {
        let progressed = match self.entries.get(&pk) {
            Some(prev) => exec_count > prev.exec_count || due != prev.due,
            None => true,
        };

        self.unlink(&pk);

        let entry = Entry {
            kind,
            due,
            exec_count,
            phase: Phase::Due,
            attempts: 0,
            paused,
            retry_after: None,
            original_due: due,
        };
        self.order.insert((kind, due, pk), ());
        self.entries.insert(pk, entry);
        progressed
    }

    /// Stop tracking a thread entirely. Only correct when it is gone on-chain.
    pub fn remove(&mut self, pk: &Pubkey) -> Option<Entry> {
        self.unlink(pk);
        self.entries.remove(pk)
    }

    /// Update the paused flag without disturbing scheduling.
    pub fn set_paused(&mut self, pk: &Pubkey, paused: bool) {
        if let Some(e) = self.entries.get_mut(pk) {
            e.paused = paused;
        }
    }

    /// Earliest due value of a kind that is actually dispatchable, for arming a
    /// timer. Entries already in flight or parked are skipped.
    pub fn next_due(&self, kind: Kind) -> Option<u64> {
        self.range_of(kind)
            .filter_map(|pk| self.entries.get(&pk))
            .find(|e| e.phase == Phase::Due && !e.paused)
            .map(|e| e.due)
    }

    /// Take everything of `kind` that the chain now considers ready.
    ///
    /// Transitions each to `InFlight` and re-arms it on a stall watchdog, so a
    /// worker that never reports back cannot strand the thread.
    pub fn take_due(&mut self, kind: Kind, current: u64, now: Instant) -> Vec<Dispatched> {
        let candidates: Vec<Pubkey> = self
            .range_of(kind)
            .filter(|pk| {
                self.entries.get(pk).is_some_and(|e| {
                    e.due <= current
                        && e.phase == Phase::Due
                        && !e.paused
                        && e.retry_after.is_none_or(|t| t <= now)
                })
            })
            .collect();

        let mut out = Vec::with_capacity(candidates.len());
        for pk in candidates {
            let (exec_count, due) = {
                let e = self.entries.get(&pk).expect("filtered above");
                (e.exec_count, e.original_due)
            };
            self.reschedule(
                &pk,
                current.saturating_add(kind.stall_watchdog()),
                Phase::InFlight { since: now },
                None,
            );
            out.push(Dispatched {
                pubkey: pk,
                exec_count,
                due,
            });
        }
        out
    }

    /// Parked threads whose account update has not arrived in time.
    ///
    /// Clearing the marker as they are handed out means each parked thread is
    /// refreshed at most once per execution, so a provider that never pushes
    /// notifications costs one extra fetch per execution rather than a poll
    /// loop.
    pub fn take_stale_parked(&mut self, now: Instant) -> Vec<Pubkey> {
        let stale: Vec<Pubkey> = self
            .entries
            .iter()
            .filter(|(_, e)| e.phase == Phase::Parked && e.retry_after.is_some_and(|t| t <= now))
            .map(|(pk, _)| *pk)
            .take(MAX_REFRESH_PER_PASS)
            .collect();

        for pk in &stale {
            if let Some(e) = self.entries.get_mut(pk) {
                e.retry_after = None;
            }
        }
        stale
    }

    /// Return anything stuck in flight past its watchdog to `Due`.
    ///
    /// A worker that dies without reporting would otherwise leave the thread
    /// permanently in flight.
    pub fn reclaim_stalled(&mut self, now: Instant) -> Vec<Pubkey> {
        let stalled: Vec<Pubkey> = self
            .entries
            .iter()
            .filter_map(|(pk, e)| match e.phase {
                Phase::InFlight { since }
                    if now.duration_since(since) > Duration::from_secs(e.kind.stall_watchdog()) =>
                {
                    Some(*pk)
                }
                _ => None,
            })
            .collect();

        for pk in &stalled {
            if let Some(e) = self.entries.get_mut(pk) {
                e.phase = Phase::Due;
                e.retry_after = Some(now);
            }
        }
        stalled
    }

    /// Record the result of a dispatch.
    ///
    /// `dispatched_exec_count` is the value the entry had when it was handed
    /// out. If the entry has since advanced, the thread's account update already
    /// arrived and re-armed it with the real next deadline, and this completion
    /// is describing a run that is now history — applying it would overwrite a
    /// correct schedule with a stale one.
    ///
    /// That race is the common case, not the exception: the account
    /// notification lands as soon as the transaction confirms, while the
    /// completion still has to travel worker -> processor -> staging. Without
    /// this guard every successful execution parks a thread that was already
    /// correctly scheduled, and it stays parked until a watchdog fires.
    ///
    /// `current` is the present value of the entry's clock, used to place the
    /// watchdog. Every branch leaves the entry with a due value — that is what
    /// makes a lost execution recoverable without depending on cache eviction.
    pub fn complete(
        &mut self,
        pk: &Pubkey,
        outcome: Outcome,
        dispatched_exec_count: u64,
        current: u64,
        now: Instant,
    ) {
        let Some(entry) = self.entries.get(pk) else {
            return;
        };
        if entry.exec_count != dispatched_exec_count {
            return; // superseded by a fresher account update
        }
        let kind = entry.kind;
        let attempts = entry.attempts;

        match outcome {
            // The chain advanced, so the next deadline arrives with the account
            // update. Park on a watchdog in case it never does — and mark the
            // entry for a cheap refresh well before that watchdog, since an RPC
            // that does not push program notifications would otherwise leave the
            // thread idle for the whole watchdog period on every execution.
            Outcome::Succeeded | Outcome::Superseded => {
                self.reschedule(
                    pk,
                    current.saturating_add(kind.parked_watchdog()),
                    Phase::Parked,
                    Some(now + REFRESH_AFTER),
                );
                if let Some(e) = self.entries.get_mut(pk) {
                    e.attempts = 0;
                }
            }

            // Nothing executed on-chain, so no update is coming. Park on a
            // watchdog rather than retrying a fiber that has nothing to run.
            Outcome::EmptyFiber | Outcome::Fatal => {
                self.reschedule(
                    pk,
                    current.saturating_add(kind.parked_watchdog()),
                    Phase::Parked,
                    None,
                );
            }

            // Another executor owns it. Retrying before the takeover delay
            // cannot succeed by construction, so back off rather than
            // re-dispatching on the next tick.
            Outcome::LoadBalancerSkip => {
                self.reschedule(pk, current, Phase::Due, Some(now + Duration::from_secs(5)));
            }

            Outcome::Retryable => {
                let next = attempts.saturating_add(1);
                self.reschedule(pk, current, Phase::Due, Some(now + retry_backoff(next)));
                if let Some(e) = self.entries.get_mut(pk) {
                    e.attempts = next;
                }
            }
        }
    }

    /// Ordered pubkeys of a kind, earliest due first.
    fn range_of(&self, kind: Kind) -> impl Iterator<Item = Pubkey> + '_ {
        self.order
            .range((kind, u64::MIN, Pubkey::default())..)
            .take_while(move |((k, _, _), _)| *k == kind)
            .map(|((_, _, pk), _)| *pk)
    }

    /// Move an entry to a new due value and phase, keeping the index consistent.
    /// `original_due` is preserved so overdue accounting still refers to the
    /// real deadline.
    fn reschedule(&mut self, pk: &Pubkey, due: u64, phase: Phase, retry_after: Option<Instant>) {
        self.unlink(pk);
        if let Some(e) = self.entries.get_mut(pk) {
            e.due = due;
            e.phase = phase;
            e.retry_after = retry_after;
            let kind = e.kind;
            self.order.insert((kind, due, *pk), ());
        }
    }

    /// Drop a thread's ordered key, if present.
    fn unlink(&mut self, pk: &Pubkey) {
        if let Some(e) = self.entries.get(pk) {
            self.order.remove(&(e.kind, e.due, *pk));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pk(n: u8) -> Pubkey {
        let mut b = [0u8; 32];
        b[0] = n;
        Pubkey::new_from_array(b)
    }

    #[test]
    fn upsert_replaces_in_place() {
        let mut s = Sched::new();
        s.upsert(pk(1), Kind::Time, 100, 0, false);
        s.upsert(pk(1), Kind::Time, 200, 1, false);

        // One entry, one ordered key — a second schedule must not leave a
        // duplicate behind for a later compaction pass to clean up.
        assert_eq!(s.len(), 1);
        assert_eq!(s.order.len(), 1);
        assert_eq!(s.get(&pk(1)).unwrap().due, 200);
        assert_eq!(s.next_due(Kind::Time), Some(200));
    }

    #[test]
    fn take_due_returns_only_reached_entries() {
        let mut s = Sched::new();
        s.upsert(pk(1), Kind::Time, 100, 0, false);
        s.upsert(pk(2), Kind::Time, 300, 0, false);

        let got = s.take_due(Kind::Time, 200, Instant::now());
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].pubkey, pk(1));
        assert_eq!(got[0].due, 100);
    }

    #[test]
    fn dispatch_does_not_remove_the_entry() {
        let mut s = Sched::new();
        s.upsert(pk(1), Kind::Time, 100, 0, false);
        s.take_due(Kind::Time, 200, Instant::now());

        // The thread is still tracked — this is what makes a lost execution
        // recoverable.
        assert!(s.contains(&pk(1)));
        assert!(matches!(
            s.get(&pk(1)).unwrap().phase,
            Phase::InFlight { .. }
        ));
    }

    #[test]
    fn in_flight_entries_are_not_dispatched_again() {
        let mut s = Sched::new();
        s.upsert(pk(1), Kind::Time, 100, 0, false);
        let now = Instant::now();

        assert_eq!(s.take_due(Kind::Time, 200, now).len(), 1);
        assert_eq!(s.take_due(Kind::Time, 200, now).len(), 0);
    }

    #[test]
    fn paused_entries_are_never_dispatched() {
        let mut s = Sched::new();
        s.upsert(pk(1), Kind::Time, 100, 0, true);

        assert!(s.take_due(Kind::Time, 200, Instant::now()).is_empty());
        assert_eq!(s.next_due(Kind::Time), None);
    }

    #[test]
    fn retryable_failure_is_rescheduled_with_backoff() {
        let mut s = Sched::new();
        s.upsert(pk(1), Kind::Time, 100, 0, false);
        let now = Instant::now();
        s.take_due(Kind::Time, 200, now);

        s.complete(&pk(1), Outcome::Retryable, 0, 200, now);

        let e = s.get(&pk(1)).unwrap();
        assert_eq!(e.phase, Phase::Due);
        assert_eq!(e.attempts, 1);
        // Backoff not yet elapsed, so not dispatchable.
        assert!(s.take_due(Kind::Time, 200, now).is_empty());
        // Once it has, it is.
        assert_eq!(
            s.take_due(Kind::Time, 200, now + Duration::from_secs(5))
                .len(),
            1
        );
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        assert_eq!(retry_backoff(1), Duration::from_secs(2));
        assert_eq!(retry_backoff(2), Duration::from_secs(4));
        assert_eq!(retry_backoff(100), Duration::from_secs(60));
    }

    #[test]
    fn load_balancer_skip_does_not_respin_immediately() {
        // The previous design rescheduled a skip at its original, already-past
        // trigger value, so the next tick re-dispatched it — a worker spawn and
        // a load-balancer write lock every ~400ms, forever, per skipped thread.
        let mut s = Sched::new();
        s.upsert(pk(1), Kind::Time, 100, 0, false);
        let now = Instant::now();
        s.take_due(Kind::Time, 200, now);

        s.complete(&pk(1), Outcome::LoadBalancerSkip, 0, 200, now);

        assert!(s
            .take_due(Kind::Time, 200, now + Duration::from_millis(400))
            .is_empty());
        assert_eq!(
            s.take_due(Kind::Time, 200, now + Duration::from_secs(6))
                .len(),
            1
        );
    }

    #[test]
    fn empty_fiber_parks_instead_of_stranding() {
        // Nothing executed on-chain, so no account update will arrive. Under the
        // old design this was reported as success and the thread was never
        // rescheduled — a fiber cleared mid-flight stalled its thread forever.
        let mut s = Sched::new();
        s.upsert(pk(1), Kind::Time, 100, 0, false);
        let now = Instant::now();
        s.take_due(Kind::Time, 200, now);

        s.complete(&pk(1), Outcome::EmptyFiber, 0, 200, now);

        let e = s.get(&pk(1)).unwrap();
        assert_eq!(e.phase, Phase::Parked);
        // Still scheduled, on a watchdog.
        assert_eq!(e.due, 200 + Kind::Time.parked_watchdog());
    }

    #[test]
    fn fatal_failure_still_leaves_a_due_value() {
        let mut s = Sched::new();
        s.upsert(pk(1), Kind::Time, 100, 0, false);
        let now = Instant::now();
        s.take_due(Kind::Time, 200, now);
        s.complete(&pk(1), Outcome::Fatal, 0, 200, now);

        assert!(s.contains(&pk(1)));
        assert!(s.get(&pk(1)).unwrap().due > 200);
    }

    #[test]
    fn stalled_dispatch_is_reclaimed() {
        let mut s = Sched::new();
        s.upsert(pk(1), Kind::Time, 100, 0, false);
        let now = Instant::now();
        s.take_due(Kind::Time, 200, now);

        // A worker that dies without reporting must not strand the thread.
        assert!(s.reclaim_stalled(now + Duration::from_secs(10)).is_empty());
        let reclaimed = s.reclaim_stalled(now + Duration::from_secs(60));
        assert_eq!(reclaimed, vec![pk(1)]);
        assert_eq!(s.get(&pk(1)).unwrap().phase, Phase::Due);
    }

    #[test]
    fn account_update_clears_backoff() {
        let mut s = Sched::new();
        s.upsert(pk(1), Kind::Time, 100, 0, false);
        let now = Instant::now();
        s.take_due(Kind::Time, 200, now);
        s.complete(&pk(1), Outcome::Retryable, 0, 200, now);
        assert_eq!(s.get(&pk(1)).unwrap().attempts, 1);

        // Fresh on-chain state supersedes whatever this node had decided.
        let progressed = s.upsert(pk(1), Kind::Time, 500, 1, false);
        assert!(progressed);
        let e = s.get(&pk(1)).unwrap();
        assert_eq!(e.attempts, 0);
        assert_eq!(e.phase, Phase::Due);
        assert!(e.retry_after.is_none());
    }

    #[test]
    fn unchanged_update_is_not_reported_as_progress() {
        let mut s = Sched::new();
        s.upsert(pk(1), Kind::Time, 100, 0, false);
        assert!(!s.upsert(pk(1), Kind::Time, 100, 0, false));
    }

    #[test]
    fn overdue_is_measured_against_the_real_deadline() {
        // Watchdogs and backoff move `due`, but takeover and commission decay
        // depend on how late we are against the actual trigger.
        let mut s = Sched::new();
        s.upsert(pk(1), Kind::Time, 100, 0, false);
        let now = Instant::now();

        s.take_due(Kind::Time, 500, now);
        s.complete(&pk(1), Outcome::Retryable, 0, 500, now);
        let got = s.take_due(Kind::Time, 500, now + Duration::from_secs(10));

        assert_eq!(got.len(), 1);
        assert_eq!(got[0].due, 100, "should remain the original deadline");
    }

    #[test]
    fn parked_threads_are_refreshed_when_no_update_arrives() {
        // An RPC that acknowledges programSubscribe but never delivers leaves a
        // thread parked after every execution. Without this it would idle for
        // the whole watchdog period each time.
        let mut s = Sched::new();
        s.upsert(pk(1), Kind::Time, 100, 0, false);
        let now = Instant::now();
        s.take_due(Kind::Time, 200, now);
        s.complete(&pk(1), Outcome::Succeeded, 0, 200, now);

        assert!(s.take_stale_parked(now).is_empty(), "not stale yet");
        assert!(
            s.take_stale_parked(now + Duration::from_secs(2)).is_empty(),
            "must stay quiet while a healthy subscription would still deliver"
        );

        let later = now + REFRESH_AFTER + Duration::from_millis(1);
        assert_eq!(s.take_stale_parked(later), vec![pk(1)]);

        // Marked once per execution, not repeatedly.
        assert!(s.take_stale_parked(later).is_empty());
    }

    #[test]
    fn refresh_is_capped_per_pass() {
        // A large fleet parking at once must not turn the safety net into a
        // request storm against the pool the execution path depends on.
        let mut s = Sched::new();
        let now = Instant::now();
        for i in 0..100u8 {
            s.upsert(pk(i), Kind::Time, 100, 0, false);
        }
        s.take_due(Kind::Time, 200, now);
        for i in 0..100u8 {
            s.complete(&pk(i), Outcome::Succeeded, 0, 200, now);
        }

        let later = now + REFRESH_AFTER + Duration::from_millis(1);
        assert_eq!(s.take_stale_parked(later).len(), MAX_REFRESH_PER_PASS);
    }

    #[test]
    fn an_arriving_update_cancels_the_refresh() {
        let mut s = Sched::new();
        s.upsert(pk(1), Kind::Time, 100, 0, false);
        let now = Instant::now();
        s.take_due(Kind::Time, 200, now);
        s.complete(&pk(1), Outcome::Succeeded, 0, 200, now);

        // The subscription delivered, so no fetch is needed.
        s.upsert(pk(1), Kind::Time, 300, 1, false);
        let later = now + REFRESH_AFTER + Duration::from_millis(1);
        assert!(s.take_stale_parked(later).is_empty());
    }

    #[test]
    fn a_completion_cannot_overwrite_a_fresher_schedule() {
        // The account notification lands as soon as the transaction confirms,
        // while the completion still has to travel worker -> processor ->
        // staging. The update almost always wins, so an unguarded completion
        // would park a thread that was already correctly re-armed, losing the
        // wake-up until a watchdog fired.
        let mut s = Sched::new();
        s.upsert(pk(1), Kind::Time, 100, 0, false);
        let now = Instant::now();

        let dispatched = s.take_due(Kind::Time, 200, now);
        assert_eq!(dispatched[0].exec_count, 0);

        // The chain's update arrives first, re-arming for the next deadline.
        s.upsert(pk(1), Kind::Time, 500, 1, false);
        assert_eq!(s.get(&pk(1)).unwrap().phase, Phase::Due);

        // The now-stale completion must not undo that.
        s.complete(
            &pk(1),
            Outcome::Succeeded,
            dispatched[0].exec_count,
            200,
            now,
        );

        let e = s.get(&pk(1)).unwrap();
        assert_eq!(e.phase, Phase::Due, "must stay armed");
        assert_eq!(e.due, 500, "must keep the new deadline");
    }

    #[test]
    fn a_completion_still_applies_when_no_update_arrived() {
        let mut s = Sched::new();
        s.upsert(pk(1), Kind::Time, 100, 0, false);
        let now = Instant::now();
        let dispatched = s.take_due(Kind::Time, 200, now);

        s.complete(
            &pk(1),
            Outcome::Succeeded,
            dispatched[0].exec_count,
            200,
            now,
        );
        assert_eq!(s.get(&pk(1)).unwrap().phase, Phase::Parked);
    }

    #[test]
    fn superseded_parks_like_success() {
        let mut s = Sched::new();
        s.upsert(pk(1), Kind::Time, 100, 0, false);
        let now = Instant::now();
        s.take_due(Kind::Time, 200, now);
        s.complete(&pk(1), Outcome::Superseded, 0, 200, now);

        let e = s.get(&pk(1)).unwrap();
        assert_eq!(e.phase, Phase::Parked);
        assert_eq!(e.attempts, 0);
    }

    #[test]
    fn kinds_are_scheduled_independently() {
        let mut s = Sched::new();
        s.upsert(pk(1), Kind::Time, 100, 0, false);
        s.upsert(pk(2), Kind::Slot, 100, 0, false);
        s.upsert(pk(3), Kind::Epoch, 100, 0, false);

        // A slot value must not make a timestamp entry look due.
        assert_eq!(s.take_due(Kind::Slot, 150, Instant::now()).len(), 1);
        assert_eq!(s.len_of(Kind::Time), 1);
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn removal_clears_both_index_and_entry() {
        let mut s = Sched::new();
        s.upsert(pk(1), Kind::Time, 100, 0, false);
        s.remove(&pk(1));

        assert!(s.is_empty());
        assert!(s.order.is_empty());
        assert_eq!(s.next_due(Kind::Time), None);
    }

    #[test]
    fn next_due_skips_entries_that_cannot_run() {
        let mut s = Sched::new();
        s.upsert(pk(1), Kind::Time, 100, 0, false);
        s.upsert(pk(2), Kind::Time, 200, 0, false);
        let now = Instant::now();

        // Once the earliest is dispatched, the timer must arm on the next one
        // that can actually run, not on the in-flight entry.
        s.take_due(Kind::Time, 100, now);
        assert_eq!(s.next_due(Kind::Time), Some(200));
    }
}
