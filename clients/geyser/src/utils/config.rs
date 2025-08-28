use {
    agave_geyser_plugin_interface::geyser_plugin_interface::{
        GeyserPluginError, Result as PluginResult,
    },
    serde::{Deserialize, Serialize},
    std::{fs::File, path::Path},
};

static DEFAULT_TRANSACTION_TIMEOUT_THRESHOLD: u64 = 150;
static DEFAULT_THREAD_COUNT: usize = 10;

/// Plugin config.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginConfig {
    pub name: String,
    pub keypath: Option<String>,
    pub libpath: Option<String>,
    pub thread_count: usize,
    pub transaction_timeout_threshold: u64,
    pub rpc_url: Option<String>,
    pub ws_url: Option<String>,
    pub forgo_executor_commission: Option<bool>,
    pub enable_replay: Option<bool>,
    pub nats_url: Option<String>,
    pub replay_delay_ms: Option<u64>,
    pub metrics: Option<crate::metrics::MetricsConfig>,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            name: "antegen".to_string(),
            keypath: None,
            libpath: None,
            transaction_timeout_threshold: DEFAULT_TRANSACTION_TIMEOUT_THRESHOLD,
            thread_count: DEFAULT_THREAD_COUNT,
            rpc_url: Some("http://localhost:8899".to_string()),
            ws_url: Some("ws://localhost:8900".to_string()),
            forgo_executor_commission: None,
            enable_replay: None,
            nats_url: None,
            replay_delay_ms: None,
            metrics: None,
        }
    }
}

impl PluginConfig {
    /// Read plugin from JSON file.
    pub fn read_from<P: AsRef<Path>>(config_path: P) -> PluginResult<Self> {
        let file = File::open(config_path)?;
        let mut this: Self = serde_json::from_reader(file)
            .map_err(|e| GeyserPluginError::ConfigFileReadError { msg: e.to_string() })?;
        
        // Apply environment variable overrides
        this.apply_env_overrides();
        
        Ok(this)
    }
    
    /// Apply environment variable overrides to the configuration
    fn apply_env_overrides(&mut self) {
        // Override keypath if env var is set
        if let Ok(keypath) = std::env::var("ANTEGEN_KEYPATH") {
            self.keypath = Some(keypath);
        }
        
        // Override RPC URL if env var is set
        if let Ok(rpc_url) = std::env::var("ANTEGEN_RPC_URL") {
            self.rpc_url = Some(rpc_url);
        }
        
        // Override WS URL if env var is set
        if let Ok(ws_url) = std::env::var("ANTEGEN_WS_URL") {
            self.ws_url = Some(ws_url);
        }
        
        
        // Override thread count if env var is set
        if let Ok(thread_count) = std::env::var("ANTEGEN_THREAD_COUNT") {
            if let Ok(count) = thread_count.parse::<usize>() {
                self.thread_count = count;
            }
        }
        
        // Override transaction timeout threshold if env var is set
        if let Ok(timeout) = std::env::var("ANTEGEN_TRANSACTION_TIMEOUT_THRESHOLD") {
            if let Ok(threshold) = timeout.parse::<u64>() {
                self.transaction_timeout_threshold = threshold;
            }
        }
        
        // Override forgo_executor_commission if env var is set
        if let Ok(forgo) = std::env::var("ANTEGEN_FORGO_EXECUTOR_COMMISSION") {
            self.forgo_executor_commission = Some(
                forgo.to_lowercase() == "true" || forgo == "1"
            );
        }
        
        // Override enable_replay if env var is set
        if let Ok(replay) = std::env::var("ANTEGEN_ENABLE_REPLAY") {
            self.enable_replay = Some(
                replay.to_lowercase() == "true" || replay == "1"
            );
        }
        
        // Override NATS URL if env var is set
        if let Ok(nats_url) = std::env::var("ANTEGEN_NATS_URL") {
            self.nats_url = Some(nats_url);
        }
        
        // Override replay delay if env var is set
        if let Ok(delay) = std::env::var("ANTEGEN_REPLAY_DELAY_MS") {
            if let Ok(delay_ms) = delay.parse::<u64>() {
                self.replay_delay_ms = Some(delay_ms);
            }
        }
    }
}
