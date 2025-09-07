pub mod builder;
pub mod datasources;
pub mod utils;

pub use builder::{AntegenClient, AntegenClientBuilder, DatasourceBuilder};

// Re-export commonly used datasources
pub use datasources::{
    rpc::{RpcDatasource, RpcConfig},
    geyser::{GeyserDatasource, GeyserConfig, GeyserPluginHelper},
};

// Re-export Carbon datasources when feature is enabled
#[cfg(feature = "carbon")]
pub use datasources::carbon::{
    CarbonRpcDatasource,
    CarbonHeliusDatasource,
    CarbonYellowstoneDatasource,
    CarbonConfig,
};
