#[cfg(feature = "carbon")]
mod helius;
#[cfg(feature = "carbon")]
mod processor;
#[cfg(feature = "carbon")]
mod rpc;
#[cfg(feature = "carbon")]
mod yellowstone;

#[cfg(feature = "carbon")]
pub use helius::CarbonHeliusDatasource;
#[cfg(feature = "carbon")]
pub use rpc::CarbonRpcDatasource;
#[cfg(feature = "carbon")]
pub use yellowstone::CarbonYellowstoneDatasource;

use solana_program::pubkey::Pubkey;

/// Configuration for Carbon datasources
#[derive(Clone, Debug)]
pub struct CarbonConfig {
    /// Thread program ID to monitor
    pub thread_program_id: Pubkey,
    /// RPC URL for blockchain connection
    pub rpc_url: String,
}

impl CarbonConfig {
    /// Create a new Carbon configuration
    pub fn new(thread_program_id: Pubkey, rpc_url: String) -> Self {
        Self {
            thread_program_id,
            rpc_url,
        }
    }
}
