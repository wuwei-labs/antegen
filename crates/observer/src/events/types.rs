use antegen_thread_program::state::Thread;
use solana_program::pubkey::Pubkey;
use solana_sdk::account::Account;

/// Events that can be observed from event sources
#[derive(Debug, Clone)]
pub enum ObservedEvent {
    /// Thread became executable
    ThreadExecutable {
        thread_pubkey: Pubkey,
        thread: Thread,
        slot: u64,
    },
    /// Thread was updated
    ThreadUpdate {
        thread_pubkey: Pubkey,
        thread: Thread,
        slot: u64,
    },
    /// Clock update
    ClockUpdate {
        slot: u64,
        epoch: u64,
        unix_timestamp: i64,
    },
    /// Account update (generic)
    AccountUpdate {
        pubkey: Pubkey,
        account: Account,
        slot: u64,
    },
}

/// Configuration for event sources
#[derive(Debug)]
pub enum EventSourceConfig {
    /// Geyser plugin event source (receives updates from plugin)
    Geyser {
        /// Channel to receive updates from Geyser plugin
        receiver: tokio::sync::mpsc::Receiver<ObservedEvent>,
    },
    /// Carbon indexer event source
    Carbon {
        /// Carbon endpoint URL
        url: String,
        /// API key if required
        api_key: Option<String>,
    },
    /// RPC polling event source
    RpcPolling {
        /// RPC endpoint URL
        rpc_url: String,
        /// Polling interval in milliseconds
        poll_interval_ms: u64,
    },
    /// Mock event source for testing
    Mock { events: Vec<ObservedEvent> },
}