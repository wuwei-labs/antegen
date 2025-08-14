use antegen_network_program::state::Builder;
use antegen_thread_program::state::Thread;
use anyhow::Result;
use async_trait::async_trait;
use solana_program::pubkey::Pubkey;
use solana_sdk::account::Account;

/// Events that can be observed from data sources
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
    /// Builder state changed
    BuilderUpdate {
        builder_pubkey: Pubkey,
        builder: Builder,
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

/// Trait for different data sources (Geyser, Carbon, RPC polling, etc.)
#[async_trait]
pub trait DataSource: Send + Sync {
    /// Start receiving events from the data source
    async fn start(&mut self) -> Result<()>;

    /// Stop receiving events
    async fn stop(&mut self) -> Result<()>;

    /// Get next event (non-blocking)
    async fn next_event(&mut self) -> Result<Option<ObservedEvent>>;

    /// Subscribe to specific thread updates
    async fn subscribe_thread(&mut self, thread_pubkey: Pubkey) -> Result<()>;

    /// Unsubscribe from thread updates
    async fn unsubscribe_thread(&mut self, thread_pubkey: Pubkey) -> Result<()>;

    /// Get current slot
    async fn get_current_slot(&self) -> Result<u64>;

    /// Get data source name for logging
    fn name(&self) -> &str;
}

/// Configuration for data sources
#[derive(Debug)]
pub enum DataSourceConfig {
    /// Geyser plugin data source (receives updates from plugin)
    Geyser {
        /// Channel to receive updates from Geyser plugin
        receiver: tokio::sync::mpsc::Receiver<ObservedEvent>,
    },
    /// Carbon indexer data source
    Carbon {
        /// Carbon endpoint URL
        url: String,
        /// API key if required
        api_key: Option<String>,
    },
    /// RPC polling data source
    RpcPolling {
        /// RPC endpoint URL
        rpc_url: String,
        /// Polling interval in milliseconds
        poll_interval_ms: u64,
    },
    /// Mock data source for testing
    Mock { events: Vec<ObservedEvent> },
}
