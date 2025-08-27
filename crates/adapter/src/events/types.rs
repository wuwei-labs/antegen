use solana_program::pubkey::Pubkey;
use solana_sdk::account::Account;

/// Events that can be observed from event sources
/// Simplified to just account updates - all processing logic moved to submitter
#[derive(Debug, Clone)]
pub enum ObservedEvent {
    /// Account update (includes Clock, Thread, and all other accounts)
    Account {
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
        receiver: crossbeam::channel::Receiver<ObservedEvent>,
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