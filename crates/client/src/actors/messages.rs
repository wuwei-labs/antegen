//! Message types for actor communication

use crate::actors::scheduler::Outcome;
use crate::trace::ExecTrace;
use crate::types::AccountUpdate;
use solana_clock::Clock;
use solana_pubkey::Pubkey;
use tokio::sync::oneshot;

// ============================================================================
// Root Supervisor Messages
// ============================================================================

#[derive(Debug, Clone)]
pub enum RootMessage {
    Shutdown,
}

// ============================================================================
// Datasource Supervisor Messages
// ============================================================================

#[derive(Debug, Clone)]
pub enum DatasourceMessage {
    AccountUpdate(AccountUpdate),
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum RpcSourceMessage {
    UpdateReceived(AccountUpdate),
    ClockReceived(Clock, ClockSource),
    /// Signal that WebSocket reconnected - trigger backfill
    Reconnected,
    /// The spawned backfill task has finished, so another may start
    BackfillFinished,
    /// A subscription background task has exited (name identifies which one)
    SubscriptionDied(String),
}

/// Where a clock reading came from.
///
/// Not all RPC implementations push `accountSubscribe` notifications for the
/// Clock sysvar — some acknowledge the subscription and then never send
/// anything. Since the clock is the only thing that advances scheduling, that
/// failure is silent and total: the node connects, backfills, reports no errors,
/// and never fires a single thread. Tracking the source lets a polling fallback
/// engage only when the subscription is actually delivering nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockSource {
    /// Pushed by the WebSocket subscription — the fast path.
    Subscription,
    /// Fetched by the fallback poller because the subscription went quiet.
    Poll,
}

#[derive(Debug, Clone)]
pub enum GeyserSourceMessage {
    /// Signal to stop consuming the channel
    Shutdown,
}

// ============================================================================
// Staging Actor Messages
// ============================================================================

#[derive(Debug)]
pub enum StagingMessage {
    AccountUpdate(AccountUpdate),
    ClockTick(Clock),
    /// The projected on-chain clock has reached the earliest pending trigger.
    /// Emitted by the staging actor's own timer, not by a datasource.
    Fire,
    /// Results of an off-actor cache-eviction refetch. `None` means the account
    /// is genuinely gone; failures are simply omitted so the thread stays
    /// tracked.
    Refetched(Vec<(Pubkey, Option<antegen_thread_program::state::Thread>)>),
    /// Every thread pubkey the program currently owns, from the periodic
    /// reconciliation scan. Compared against what is tracked to find threads the
    /// subscription never delivered.
    Reconciled(Vec<Pubkey>),
    /// The reconciliation scan failed. Distinct from `Reconciled(vec![])`, which
    /// would mean the program genuinely owns no threads and every tracked thread
    /// should be dropped.
    ReconcileFailed,
    ThreadCompleted {
        thread_pubkey: Pubkey,
        outcome: Outcome,
        /// The exec_count this attempt was dispatched with, so a completion that
        /// has been overtaken by a fresher account update can be discarded.
        exec_count: u64,
    },
    SetProcessorRef(ractor::ActorRef<ProcessorMessage>),
    QueryStatus(oneshot::Sender<StagingStatus>),
    Shutdown,
}

#[derive(Debug, Clone)]
pub struct StagingStatus {
    pub total_threads: usize,
    /// Threads dispatched and awaiting a completion report.
    pub in_flight: usize,
    pub time_queue_size: usize,
    pub slot_queue_size: usize,
    pub epoch_queue_size: usize,
}

// ============================================================================
// Processor Factory Messages
// ============================================================================

#[derive(Debug)]
pub enum ProcessorMessage {
    /// Process a ready thread - ProcessorFactory will fetch Thread from cache
    ProcessReady(ReadyThread),
    CancelThread(Pubkey),
    WorkerCompleted(ExecutionResult),
    QueryStatus(oneshot::Sender<ProcessorStatus>),
    Shutdown,
}

#[derive(Debug, Clone)]
pub struct ProcessorStatus {
    pub pending_queue_size: usize,
    pub active_workers: usize,
    pub available_permits: usize,
}

// ============================================================================
// Worker Messages
// ============================================================================

#[derive(Debug, Clone)]
pub enum WorkerMessage {
    Cancel,
}

// ============================================================================
// Shared Types
// ============================================================================

/// Thread ready for execution (sent from Staging to Processor)
/// Contains only trigger metadata - ProcessorFactory fetches full Thread from cache
#[derive(Debug, Clone)]
pub struct ReadyThread {
    pub thread_pubkey: Pubkey,
    pub exec_count: u64,
    pub is_overdue: bool,
    pub overdue_seconds: i64,
    /// Latency timeline for this execution attempt, anchored on the trigger
    /// deadline. Travels with the attempt and is rendered on completion.
    pub trace: ExecTrace,
}

/// Result of worker execution (sent from Worker to Processor)
/// Note: Does not include Thread data - cache is the source of truth
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub thread_pubkey: Pubkey,
    /// exec_count at dispatch, carried back so staging can tell whether this
    /// result still describes the thread's current state.
    pub exec_count: u64,
    /// What this means for scheduling. Set where the failure happens, rather
    /// than inferred downstream by matching on the error text.
    pub outcome: Outcome,
    pub error: Option<String>,
    pub attempt_count: u32,
    pub trace: ExecTrace,
}

impl ExecutionResult {
    fn new(
        thread_pubkey: Pubkey,
        outcome: Outcome,
        error: Option<String>,
        attempt_count: u32,
        trace: ExecTrace,
    ) -> Self {
        Self {
            thread_pubkey,
            exec_count: trace.exec_count,
            outcome,
            error,
            attempt_count,
            trace,
        }
    }

    /// Landed on-chain.
    pub fn success(thread_pubkey: Pubkey, trace: ExecTrace) -> Self {
        Self::new(thread_pubkey, Outcome::Succeeded, None, 0, trace)
    }

    /// Nothing to submit — the fiber had no compiled instruction.
    pub fn empty_fiber(thread_pubkey: Pubkey, trace: ExecTrace) -> Self {
        Self::new(thread_pubkey, Outcome::EmptyFiber, None, 0, trace)
    }

    /// The chain moved under us; an account update is already on its way.
    pub fn superseded(thread_pubkey: Pubkey, error: String, trace: ExecTrace) -> Self {
        Self::new(thread_pubkey, Outcome::Superseded, Some(error), 0, trace)
    }

    /// Declined by the load balancer.
    pub fn lb_skip(thread_pubkey: Pubkey, error: String, trace: ExecTrace) -> Self {
        Self::new(
            thread_pubkey,
            Outcome::LoadBalancerSkip,
            Some(error),
            0,
            trace,
        )
    }

    /// Failed, but worth another attempt.
    pub fn retryable(
        thread_pubkey: Pubkey,
        error: String,
        attempt_count: u32,
        trace: ExecTrace,
    ) -> Self {
        Self::new(
            thread_pubkey,
            Outcome::Retryable,
            Some(error),
            attempt_count,
            trace,
        )
    }

    /// Failed in a way retrying cannot fix.
    pub fn fatal(
        thread_pubkey: Pubkey,
        error: String,
        attempt_count: u32,
        trace: ExecTrace,
    ) -> Self {
        Self::new(
            thread_pubkey,
            Outcome::Fatal,
            Some(error),
            attempt_count,
            trace,
        )
    }
}
