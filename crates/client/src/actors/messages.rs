//! Message types for actor communication

use crate::types::AccountUpdate;
use solana_sdk::{clock::Clock, pubkey::Pubkey};
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
    ClockReceived(Clock),
    /// Signal that WebSocket reconnected - trigger backfill
    Reconnected,
}

#[derive(Debug, Clone)]
pub enum GeyserSourceMessage {
    /// Signal to stop consuming the channel
    Shutdown,
}

// ============================================================================
// Staging Actor Messages
// ============================================================================

/// Reason for thread completion - determines if re-scheduling is needed
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionReason {
    /// Successfully executed (or failed execution) - will be re-scheduled when account updates
    Executed,
    /// Load balancer skipped - re-queue for later takeover attempt
    Skipped,
}

#[derive(Debug)]
pub enum StagingMessage {
    AccountUpdate(AccountUpdate),
    ClockTick(Clock),
    ThreadCompleted {
        thread_pubkey: Pubkey,
        reason: CompletionReason,
    },
    SetProcessorRef(ractor::ActorRef<ProcessorMessage>),
    QueryStatus(oneshot::Sender<StagingStatus>),
    Shutdown,
}

#[derive(Debug, Clone)]
pub struct StagingStatus {
    pub total_threads: usize,
    pub queued_threads: usize,
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
}

/// Result of worker execution (sent from Worker to Processor)
/// Note: Does not include Thread data - cache is the source of truth
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub thread_pubkey: Pubkey,
    pub success: bool,
    pub error: Option<String>,
    pub attempt_count: u32,
}

impl ExecutionResult {
    pub fn success(thread_pubkey: Pubkey) -> Self {
        Self {
            thread_pubkey,
            success: true,
            error: None,
            attempt_count: 0,
        }
    }

    pub fn failed(thread_pubkey: Pubkey, error: String, attempt_count: u32) -> Self {
        Self {
            thread_pubkey,
            success: false,
            error: Some(error),
            attempt_count,
        }
    }
}

// ============================================================================
// Internal Types (for queue management)
// ============================================================================

/// Scheduled thread in priority queue
#[derive(Debug, Clone)]
pub(crate) struct ScheduledThread {
    /// The trigger value (timestamp, slot, or epoch) when this thread should execute
    pub trigger_value: u64,
    /// The thread's public key
    pub thread_pubkey: Pubkey,
    /// The expected exec_count (for stale detection)
    pub exec_count: u64,
}

impl PartialEq for ScheduledThread {
    fn eq(&self, other: &Self) -> bool {
        self.trigger_value == other.trigger_value
    }
}

impl Eq for ScheduledThread {}

impl PartialOrd for ScheduledThread {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledThread {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.trigger_value.cmp(&other.trigger_value)
    }
}
