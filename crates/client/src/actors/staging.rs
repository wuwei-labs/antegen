//! Staging Actor
//!
//! Owns thread scheduling: which threads exist, when each is next due, and which
//! are currently dispatched. The scheduling decisions themselves live in
//! [`crate::actors::sched::Sched`], which is pure and unit-tested; this actor is
//! the I/O shell around it.
//!
//! Two things drive dispatch:
//! - a timer armed on the earliest pending time trigger, projected onto the local
//!   clock by [`ClockRef`], which is what keeps firing off the WebSocket's
//!   critical path; and
//! - clock ticks, which advance the projection and drive slot/epoch triggers.
//!
//! Only trigger metadata is tracked here. The cache remains the single source of
//! truth for account data.

use crate::actors::messages::{ProcessorMessage, ReadyThread, StagingMessage, StagingStatus};
use crate::actors::processor::LATENCY_TARGET;
use crate::actors::sched::{Dispatched, Kind, Outcome, Sched};
use crate::clockref::ClockRef;
use crate::config::ClientConfig;
use crate::load_balancer::LoadBalancer;
use crate::resources::SharedResources;
use crate::trace::ExecTrace;
use anchor_lang::AccountDeserialize;
use antegen_thread_program::state::{Schedule, Thread, Trigger};
use log::{debug, info, warn};
use ractor::{Actor, ActorProcessingErr, ActorRef};
use solana_sdk::{clock::Clock, pubkey::Pubkey};
use std::collections::HashSet;
use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

/// How often the load balancer's tracking map is pruned of dead threads.
const PRUNE_INTERVAL: Duration = Duration::from_secs(300);

/// How often the scheduler heartbeat is logged.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);

/// Cap on cache-eviction refetches issued at once.
const MAX_EVICTIONS_PER_BATCH: usize = 32;

#[derive(Default)]
pub struct StagingActor;

pub struct StagingState {
    /// Scheduling state: what is tracked, when it is due, and its phase.
    sched: Sched,

    /// Maps on-chain time onto the local monotonic clock, so a trigger deadline
    /// can be expressed as a local instant.
    clock_ref: ClockRef,

    /// Last clock values seen, used to evaluate slot and epoch triggers and to
    /// place watchdogs in the right units.
    last_slot: u64,
    last_epoch: u64,

    /// Clock dedup. Only an exact repeat is dropped — see `handle_clock_tick`.
    last_processed_slot: u64,

    /// Periodic maintenance, gated on elapsed time rather than on slot numbers.
    /// A `slot % N` gate silently never fires if that slot is skipped or its
    /// tick is dropped.
    last_prune: Instant,
    last_heartbeat: Instant,

    processor_ref: Option<ActorRef<ProcessorMessage>>,
    resources: SharedResources,
    load_balancer: Arc<LoadBalancer>,

    /// Cache eviction receiver - threads to refetch after TTL expiry
    eviction_rx: mpsc::UnboundedReceiver<Pubkey>,

    /// Drives the single scheduling timer. Holds the local instant at which the
    /// earliest pending time trigger is projected to become due.
    timer_tx: watch::Sender<Option<Instant>>,
}

impl StagingState {
    /// Present value of a kind's clock, for placing watchdogs and evaluating
    /// readiness.
    fn current(&self, kind: Kind) -> u64 {
        match kind {
            Kind::Time => self.clock_ref.anchor_ts_projected().max(0) as u64,
            Kind::Slot => self.last_slot,
            Kind::Epoch => self.last_epoch,
        }
    }
}

impl Actor for StagingActor {
    type Msg = StagingMessage;
    type State = StagingState;
    type Arguments = (
        ClientConfig,
        SharedResources,
        Arc<LoadBalancer>,
        mpsc::UnboundedReceiver<Pubkey>,
    );

    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        (_config, resources, load_balancer, eviction_rx): Self::Arguments,
    ) -> Result<Self::State, Box<dyn Error + Send + Sync>> {
        log::debug!("StagingActor starting...");
        log::debug!("Thread program ID: {}", resources.program_id);

        let (timer_tx, timer_rx) = watch::channel(None);
        spawn_scheduling_timer(myself.clone(), timer_rx);

        let now = Instant::now();
        Ok(StagingState {
            sched: Sched::new(),
            clock_ref: ClockRef::new(),
            last_slot: 0,
            last_epoch: 0,
            last_processed_slot: 0,
            last_prune: now,
            last_heartbeat: now,
            processor_ref: None, // Will be set by RootSupervisor after processor spawns
            resources,
            load_balancer,
            eviction_rx,
            timer_tx,
        })
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            StagingMessage::AccountUpdate(update) => {
                self.handle_account_update(state, update).await?;
                // An update may have introduced an earlier deadline than the one
                // the timer is currently armed on.
                self.rearm_timer(state);
                Ok(())
            }
            StagingMessage::ClockTick(clock) => {
                self.handle_clock_tick(myself, state, clock).await?;
                Ok(())
            }
            StagingMessage::Fire => {
                self.handle_fire(state);
                self.rearm_timer(state);
                Ok(())
            }
            StagingMessage::ThreadCompleted {
                thread_pubkey,
                outcome,
            } => {
                let kind = state
                    .sched
                    .get(&thread_pubkey)
                    .map(|e| e.kind)
                    .unwrap_or(Kind::Time);
                let current = state.current(kind);
                state
                    .sched
                    .complete(&thread_pubkey, outcome, current, Instant::now());

                debug!("Thread {} completed: {:?}", thread_pubkey, outcome);
                self.rearm_timer(state);
                Ok(())
            }
            StagingMessage::Refetched(results) => {
                for (pubkey, thread) in results {
                    match thread {
                        Some(thread) => {
                            self.track(state, pubkey, &thread);
                        }
                        None => {
                            debug!("Thread {} no longer exists, dropping tracking", pubkey);
                            state.sched.remove(&pubkey);
                            state.load_balancer.remove_thread(&pubkey).await;
                        }
                    }
                }
                self.rearm_timer(state);
                Ok(())
            }
            StagingMessage::SetProcessorRef(processor_ref) => {
                log::debug!("StagingActor received processor reference");
                state.processor_ref = Some(processor_ref);
                Ok(())
            }
            StagingMessage::QueryStatus(tx) => {
                let status = StagingStatus {
                    total_threads: state.sched.len(),
                    in_flight: state.sched.in_flight(),
                    time_queue_size: state.sched.len_of(Kind::Time),
                    slot_queue_size: state.sched.len_of(Kind::Slot),
                    epoch_queue_size: state.sched.len_of(Kind::Epoch),
                };
                let _ = tx.send(status);
                Ok(())
            }
            StagingMessage::Shutdown => {
                log::info!("StagingActor shutting down...");
                Err(From::from("Shutdown signal received"))
            }
        }
    }

    async fn post_stop(
        &self,
        _myself: ActorRef<Self::Msg>,
        state: &mut Self::State,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        log::info!(
            "StagingActor stopped. {} threads tracked, {} in flight",
            state.sched.len(),
            state.sched.in_flight()
        );
        Ok(())
    }
}

impl StagingActor {
    /// Handle incoming account update.
    ///
    /// The data is already in the cache; only trigger metadata is extracted here.
    async fn handle_account_update(
        &self,
        state: &mut StagingState,
        update: crate::types::AccountUpdate,
    ) -> Result<(), ActorProcessingErr> {
        match self.classify_account(&update.data, &update.pubkey) {
            AccountType::Thread(thread) => {
                // Cancel an in-flight worker only when the *schedule* changed.
                // An advancing exec_count is normal progress from our own
                // worker's continuation batches; cancelling on that would cause a
                // cancel-restart loop and drain the crank wallet.
                //
                // Compared against `original_due`, not `due`: watchdogs and
                // backoff move `due`, so comparing that would report a schedule
                // change on every retry and reintroduce exactly that loop.
                let schedule_changed = state
                    .sched
                    .get(&update.pubkey)
                    .is_some_and(|e| due_value(&thread).is_some_and(|next| next != e.original_due));

                if schedule_changed
                    && state
                        .sched
                        .get(&update.pubkey)
                        .is_some_and(|e| e.is_in_flight())
                {
                    if let Some(ref processor_ref) = state.processor_ref {
                        if let Err(e) = processor_ref
                            .send_message(ProcessorMessage::CancelThread(update.pubkey))
                        {
                            warn!(
                                "Failed to send cancel for thread {}: {:?}",
                                update.pubkey, e
                            );
                        } else {
                            info!("Cancelled thread {} due to schedule change", update.pubkey);
                        }
                    }
                }

                if !state.sched.contains(&update.pubkey) {
                    info!(
                        "Thread {} discovered (exec_count={})",
                        update.pubkey, thread.exec_count
                    );
                }

                // Ingest latency: frame decoded off the socket -> thread
                // scheduled. Measured separately from execution latency because
                // it is bounded by the actor loop, not by the trigger deadline.
                // A large value here means this actor was blocked.
                log::debug!(
                    target: LATENCY_TARGET,
                    "ingest thread={} slot={} ingest_ms={}",
                    update.pubkey,
                    update.slot,
                    update.received_at.elapsed().as_millis()
                );

                self.track(state, update.pubkey, &thread);
            }
            AccountType::Clock => {
                // Clock updates arrive as ClockTick, not as account updates.
            }
            AccountType::Deleted => {
                debug!("Thread {} deleted", update.pubkey);
                state.sched.remove(&update.pubkey);
                state.load_balancer.remove_thread(&update.pubkey).await;
            }
            AccountType::Other => {
                // Not a thread account (could be Fiber, ThreadConfig, etc.)
            }
        }

        Ok(())
    }

    /// Record a thread's schedule, replacing any previous entry in place.
    fn track(&self, state: &mut StagingState, pubkey: Pubkey, thread: &Thread) {
        let Some((kind, due)) = kind_and_due(thread) else {
            // `Trigger::Account` is implemented on-chain but has no off-chain
            // watcher, so such a thread would never be dispatched. Warn rather
            // than tracking something that can never fire.
            warn!(
                "Thread {} has an unsupported trigger for off-chain scheduling: {:?}",
                pubkey, thread.trigger
            );
            return;
        };

        state
            .sched
            .upsert(pubkey, kind, due, thread.exec_count, thread.paused);
    }

    /// Handle a clock tick: advance the projection, run maintenance, and
    /// evaluate the trigger kinds that are tick-driven.
    async fn handle_clock_tick(
        &self,
        myself: ActorRef<StagingMessage>,
        state: &mut StagingState,
        clock: Clock,
    ) -> Result<(), ActorProcessingErr> {
        let now = Instant::now();

        // Anchor local time to on-chain time before the dedup, so every tick
        // contributes to the projection even if it doesn't drive readiness.
        // Stamping here rather than at the socket is deliberate: any delay
        // getting into this actor is real scheduling delay, and folding it into
        // the anchor is what makes `tick_ms` report it.
        //
        // `observe` also rejects a datasource that has fallen far behind, which
        // a plain high-water mark cannot distinguish from a fork.
        if !state.clock_ref.observe(&clock, now) {
            debug!(
                "Dropping clock tick from a lagging source (slot={}, high={})",
                clock.slot,
                state.clock_ref.high_slot()
            );
            return Ok(());
        }

        // Dedup: skip only an exact repeat of the slot we just handled.
        //
        // At `processed` commitment slots are not monotone — they can be skipped
        // or rolled back — so a strict high-water mark would let one forked-ahead
        // tick blackhole every subsequent tick from the canonical chain until it
        // caught up. Dispatch is idempotent (an entry in flight is not
        // re-dispatched), so processing an out-of-order tick is harmless where
        // dropping a real one is not.
        if clock.slot == state.last_processed_slot {
            return Ok(());
        }
        state.last_processed_slot = clock.slot;
        state.last_slot = state.last_slot.max(clock.slot);
        state.last_epoch = state.last_epoch.max(clock.epoch);

        self.run_maintenance(myself, state, now).await;

        // Time triggers are normally dispatched by the timer; evaluating them
        // here too covers the window before the first tick has armed it.
        let mut ready = self.collect(state, Kind::Time, now);
        ready.extend(self.collect(state, Kind::Slot, now));
        ready.extend(self.collect(state, Kind::Epoch, now));

        self.dispatch(state, ready);
        self.rearm_timer(state);

        Ok(())
    }

    /// Fire because the projected on-chain clock has reached the earliest
    /// deadline, rather than because a WebSocket message happened to arrive.
    ///
    /// Only time triggers are evaluated: slot and epoch triggers have no
    /// wall-clock deadline to project, so they stay tick-driven.
    fn handle_fire(&self, state: &mut StagingState) {
        let ready = self.collect(state, Kind::Time, Instant::now());
        self.dispatch(state, ready);
    }

    /// Take everything of `kind` the chain now considers ready and turn it into
    /// dispatchable work.
    fn collect(&self, state: &mut StagingState, kind: Kind, now: Instant) -> Vec<ReadyThread> {
        let current = state.current(kind);
        let due = state.sched.take_due(kind, current, now);
        if due.is_empty() {
            return Vec::new();
        }

        due.into_iter()
            .map(|d| self.to_ready(state, kind, d, current))
            .collect()
    }

    fn to_ready(
        &self,
        state: &StagingState,
        kind: Kind,
        d: Dispatched,
        current: u64,
    ) -> ReadyThread {
        // Only time triggers have a meaningful overdue measure; it drives
        // load-balancer takeover and on-chain commission decay.
        let overdue_seconds = if kind == Kind::Time {
            current as i64 - d.due as i64
        } else {
            0
        };

        let due_ts = d.due as i64;
        let due_at = if kind == Kind::Time {
            state.clock_ref.instant_for_ts(due_ts)
        } else {
            None
        };

        ReadyThread {
            thread_pubkey: d.pubkey,
            exec_count: d.exec_count,
            is_overdue: overdue_seconds > 0,
            overdue_seconds,
            trace: ExecTrace::new(d.pubkey, due_ts, due_at),
        }
    }

    /// Arm the single scheduling timer on the earliest pending time trigger.
    ///
    /// One timer for the whole actor, re-armed whenever the head of the queue
    /// may have changed — not one timer per thread.
    fn rearm_timer(&self, state: &StagingState) {
        let target = state
            .sched
            .next_due(Kind::Time)
            .and_then(|due| state.clock_ref.instant_for_ts(due as i64));
        let _ = state.timer_tx.send(target);
    }

    /// Push ready threads to the ProcessorFactory.
    fn dispatch(&self, state: &mut StagingState, ready_threads: Vec<ReadyThread>) {
        if ready_threads.is_empty() {
            return;
        }
        info!("Found {} ready threads", ready_threads.len());

        for ready_thread in ready_threads {
            let Some(ref processor_ref) = state.processor_ref else {
                warn!(
                    "ProcessorFactory not initialized yet, returning thread {} to the queue",
                    ready_thread.thread_pubkey
                );
                // Put it back so it is retried rather than silently lost.
                let kind = state
                    .sched
                    .get(&ready_thread.thread_pubkey)
                    .map(|e| e.kind)
                    .unwrap_or(Kind::Time);
                let current = state.current(kind);
                state.sched.complete(
                    &ready_thread.thread_pubkey,
                    Outcome::Retryable,
                    current,
                    Instant::now(),
                );
                continue;
            };

            let pubkey = ready_thread.thread_pubkey;
            let overdue = ready_thread.overdue_seconds;
            if let Err(e) = processor_ref.send_message(ProcessorMessage::ProcessReady(ready_thread))
            {
                warn!("Failed to send thread {} to processor: {:?}", pubkey, e);
                let kind = state
                    .sched
                    .get(&pubkey)
                    .map(|e| e.kind)
                    .unwrap_or(Kind::Time);
                let current = state.current(kind);
                state
                    .sched
                    .complete(&pubkey, Outcome::Retryable, current, Instant::now());
            } else {
                info!(
                    "Pushed thread {} to processor (overdue_seconds={})",
                    pubkey, overdue
                );
            }
        }
    }

    /// Periodic upkeep, gated on elapsed time.
    async fn run_maintenance(
        &self,
        myself: ActorRef<StagingMessage>,
        state: &mut StagingState,
        now: Instant,
    ) {
        // Return anything stuck in flight past its watchdog. A worker that dies
        // without reporting would otherwise strand its thread.
        let reclaimed = state.sched.reclaim_stalled(now);
        if !reclaimed.is_empty() {
            warn!(
                "Reclaimed {} thread(s) stalled in flight past the watchdog",
                reclaimed.len()
            );
        }

        if now.duration_since(state.last_heartbeat) >= HEARTBEAT_INTERVAL {
            state.last_heartbeat = now;
            info!(
                "Scheduler heartbeat: slot={}, tracked={}, in_flight={}",
                state.last_slot,
                state.sched.len(),
                state.sched.in_flight()
            );

            // Only worth reporting when more than one datasource is configured —
            // with a single endpoint it wins every race by definition.
            if state.resources.ingest_stats.is_racing() {
                for s in state.resources.ingest_stats.snapshot() {
                    info!(
                        "ingest endpoint={} clock_win={}% ({}/{}) lag_avg_ms={} lag_max_ms={} account_win={}/{}",
                        s.endpoint,
                        s.clock_win_pct(),
                        s.clocks_won,
                        s.clocks_seen,
                        s.clock_lag_avg_ms,
                        s.clock_lag_max_ms,
                        s.accounts_won,
                        s.accounts_seen,
                    );
                }
            }
        }

        if now.duration_since(state.last_prune) >= PRUNE_INTERVAL {
            state.last_prune = now;
            let known: HashSet<Pubkey> = state.sched.tracked().collect();
            state.load_balancer.prune_stale(&known).await;
        }

        self.drain_evictions(myself, state);
    }

    /// Refetch threads whose cache entries expired.
    ///
    /// Runs off the actor loop. Doing this inline meant up to ten sequential RPC
    /// round trips inside the message handler, freezing every clock tick and
    /// account update for their duration — and the ticks that queued up behind
    /// were then discarded by the dedup.
    ///
    /// With the never-pop invariant this is belt-and-braces: a thread is already
    /// re-armed on a watchdog, so a missed refetch delays rather than strands it.
    fn drain_evictions(&self, myself: ActorRef<StagingMessage>, state: &mut StagingState) {
        let mut pubkeys = Vec::new();
        while pubkeys.len() < MAX_EVICTIONS_PER_BATCH {
            match state.eviction_rx.try_recv() {
                Ok(pk) => pubkeys.push(pk),
                Err(_) => break,
            }
        }
        if pubkeys.is_empty() {
            return;
        }

        let cache = state.resources.cache.clone();
        let rpc = state.resources.rpc_client.clone();

        tokio::spawn(async move {
            let mut results = Vec::with_capacity(pubkeys.len());
            for pk in pubkeys {
                match cache.get_thread_or_fetch(&pk, &rpc).await {
                    Ok(thread) => results.push((pk, Some(thread))),
                    Err(e) if e.is_gone() => results.push((pk, None)),
                    Err(e) => {
                        // A transport or decode failure says nothing about
                        // whether the thread exists. Leave it tracked.
                        debug!("Refetch of {} failed, leaving tracked: {}", pk, e);
                    }
                }
            }
            if results.is_empty() {
                return;
            }
            let _ = myself.send_message(StagingMessage::Refetched(results));
        });
    }

    /// Classify account type
    fn classify_account(&self, data: &[u8], pubkey: &Pubkey) -> AccountType {
        // Check if it's the clock sysvar
        if *pubkey == solana_sdk::sysvar::clock::ID {
            return AccountType::Clock;
        }

        // Check if data is empty (deleted account)
        if data.is_empty() {
            return AccountType::Deleted;
        }

        // Need at least 8 bytes for discriminator + some data
        if data.len() < 8 {
            return AccountType::Other;
        }

        // Try to deserialize as Thread
        if let Ok(thread) = Thread::try_deserialize(&mut &data[..]) {
            return AccountType::Thread(thread);
        }

        AccountType::Other
    }
}

/// Map a thread's trigger and schedule onto the clock it is scheduled against.
///
/// `Trigger::Account` has no off-chain watcher and returns `None`.
fn kind_and_due(thread: &Thread) -> Option<(Kind, u64)> {
    match &thread.trigger {
        Trigger::Immediate { .. }
        | Trigger::Timestamp { .. }
        | Trigger::Interval { .. }
        | Trigger::Cron { .. } => match thread.schedule {
            Schedule::Timed { next, .. } => Some((Kind::Time, next.max(0) as u64)),
            _ => None,
        },
        Trigger::Slot { .. } => match thread.schedule {
            Schedule::Block { next, .. } => Some((Kind::Slot, next)),
            _ => None,
        },
        Trigger::Epoch { .. } => match thread.schedule {
            Schedule::Block { next, .. } => Some((Kind::Epoch, next)),
            _ => None,
        },
        Trigger::Account { .. } => None,
    }
}

/// Just the due value, for change detection.
fn due_value(thread: &Thread) -> Option<u64> {
    kind_and_due(thread).map(|(_, due)| due)
}

/// Drive a single timer that fires when the projected on-chain clock reaches the
/// earliest pending trigger.
///
/// One task for the whole actor, re-armed via a watch channel whenever the head
/// of the time queue may have changed — not one timer per thread. Firing on a
/// timer rather than on the next clock notification is what removes the tick
/// source from the critical path: without it, a thread due at T is not noticed
/// until the next WebSocket message happens to arrive.
fn spawn_scheduling_timer(
    actor: ActorRef<StagingMessage>,
    mut target_rx: watch::Receiver<Option<Instant>>,
) {
    tokio::spawn(async move {
        loop {
            let target = *target_rx.borrow_and_update();

            let Some(at) = target else {
                // Nothing scheduled; wait for an arm.
                if target_rx.changed().await.is_err() {
                    return; // actor gone
                }
                continue;
            };

            tokio::select! {
                _ = tokio::time::sleep_until(at.into()) => {
                    if actor.send_message(StagingMessage::Fire).is_err() {
                        return; // actor gone
                    }
                    // Wait to be re-armed rather than re-firing on the same
                    // deadline, which is now in the past and would spin.
                    if target_rx.changed().await.is_err() {
                        return;
                    }
                }
                changed = target_rx.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
            }
        }
    });
}

#[derive(Debug)]
enum AccountType {
    Thread(Thread),
    Clock,
    Deleted,
    Other,
}
