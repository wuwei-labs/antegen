use {
    solana_sdk::commitment_config::CommitmentConfig,
    std::{
        env,
        fs,
        path::PathBuf,
        time::Duration,
    },
};

pub const DEFAULT_RPC_TIMEOUT_SECONDS: Duration = Duration::from_secs(30);
pub const DEFAULT_CONFIRM_TX_TIMEOUT_SECONDS: Duration = Duration::from_secs(5);

/// The combination of solana config file and our own config file
#[derive(Debug, PartialEq)]
pub struct CliConfig {
    pub json_rpc_url: String,
    pub websocket_url: String,
    pub keypair_path: String,
    pub rpc_timeout: Duration,
    pub commitment: CommitmentConfig,
    pub confirm_transaction_initial_timeout: Duration,

    pub active_version: String,
    pub dev: bool,
}

impl CliConfig {
    pub fn load() -> Self {
        let solana_config_file = solana_cli_config::CONFIG_FILE.as_ref().unwrap().as_str();
        let solana_config = solana_cli_config::Config::load(solana_config_file).unwrap();
        CliConfig {
            json_rpc_url: solana_config.json_rpc_url,
            websocket_url: solana_config.websocket_url,
            keypair_path: solana_config.keypair_path,
            rpc_timeout: DEFAULT_RPC_TIMEOUT_SECONDS,
            commitment: CommitmentConfig::confirmed(),
            confirm_transaction_initial_timeout: DEFAULT_CONFIRM_TX_TIMEOUT_SECONDS,
            active_version: env!("CARGO_PKG_VERSION").to_owned(),
            dev: false,
        }
    }

    // These methods are kept for potential future use but marked as allow(dead_code)
    // since deps.rs still references the constants in this module
    #[allow(dead_code)]
    pub fn default_home() -> PathBuf {
        dirs_next::home_dir()
            .map(|mut path| {
                path.extend([".config", "antegen"]);
                path
            })
            .unwrap()
    }

    #[allow(dead_code)]
    pub fn default_runtime_dir() -> PathBuf {
        let mut path = Self::default_home();
        path.extend(["localnet", "runtime_deps"]);
        path
    }

    #[allow(dead_code)]
    pub fn active_runtime_dir(&self) -> PathBuf {
        Self::default_runtime_dir().join(&self.active_version)
    }

    #[allow(dead_code)]
    pub fn target_dir(&self) -> PathBuf {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.extend(["..", "..", "target"]);
        fs::canonicalize(path).unwrap()
    }

    #[allow(dead_code)]
    pub fn active_runtime(&self, filename: &str) -> String {
        let result = if self.dev == true {
            if filename.contains("solana") {
                self.active_runtime_dir().join(filename).display().to_string()
            } else if filename.contains("program") {
                self.target_dir().join("deploy").join(filename).display().to_string()
            } else {
                self.target_dir().join("debug").join(filename).display().to_string()
            }
        } else {
            self.active_runtime_dir().join(filename).display().to_string()
        };
        println!("DEBUG: active_runtime('{}') -> {}", filename, result);
        result
    }

    /// This assumes the path for the signatory keypair created by solana-test-validator
    /// is test-ledger/validator-keypair.json
    #[allow(dead_code)]
    pub fn signatory(&self) -> String {
        env::current_dir()
            .map(|mut path| {
                path.extend(["test-ledger", "validator-keypair.json"]);
                path
            })
            .expect(&format!(
                "Unable to find location of validator-keypair.json"
            ))
            .display()
            .to_string()
    }

    #[allow(dead_code)]
    pub fn geyser_config(&self) -> String {
        self.active_runtime("geyser-plugin-config.json")
    }

    #[allow(dead_code)]
    pub fn geyser_lib(&self) -> String {
        if self.dev == true && env::consts::OS.to_lowercase().contains("mac") {
            self.active_runtime("libantegen_client_geyser.dylib")
        } else {
            // in the release process, we always rename dylib to so anyway
            self.active_runtime("libantegen_client_geyser.so")
        }
    }
}


