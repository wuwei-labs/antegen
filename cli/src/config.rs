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
pub const ANTEGEN_RELEASE_URL: &str = "https://github.com/wuwei-labs/antegen/releases/download";
pub const ANTEGEN_DEPS: &[&str] = &[
    "antegen_network_program.so",
    "antegen_thread_program.so",
    "libantegen_plugin.so",
];
pub const SOLANA_RELEASE_BASE_URL: &str = "https://github.com/anza-xyz/agave/releases/download";
pub const SOLANA_DEPS: &[&str] = &["solana-test-validator"];

#[derive(Debug, PartialEq, Clone)]
pub enum DevMode {
    None,          // Not in dev mode
    All,          // Both programs and plugin from local
    Programs,      // Only programs from local
    Plugin,        // Only plugin from local
}

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
    pub dev_mode: DevMode,
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
            dev_mode: DevMode::None,
        }
    }

    pub fn default_home() -> PathBuf {
        dirs_next::home_dir()
            .map(|mut path| {
                path.extend([".config", "antegen"]);
                path
            })
            .unwrap()
    }

    pub fn default_runtime_dir() -> PathBuf {
        let mut path = Self::default_home();
        path.extend(["localnet", "runtime_deps"]);
        path
    }

    pub fn active_runtime_dir(&self) -> PathBuf {
        Self::default_runtime_dir().join(&self.active_version)
    }

    pub fn target_dir(&self) -> PathBuf {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.extend(["..", "target"]);
        fs::canonicalize(path).unwrap()
    }

    pub fn active_runtime(&self, filename: &str) -> String {
        match self.dev_mode {
            DevMode::None => {
                self.active_runtime_dir().join(filename).to_string()
            },
            DevMode::All => {
                if filename.contains("solana") {
                    self.active_runtime_dir().join(filename).to_string()
                } else if filename.contains("program") {
                    self.target_dir().join("deploy").join(filename).to_string()
                } else {
                    self.target_dir().join("debug").join(filename).to_string()
                }
            },
            DevMode::Programs => {
                // Only programs from local, plugin from archive
                if filename.contains("program") {
                    self.target_dir().join("deploy").join(filename).to_string()
                } else {
                    self.active_runtime_dir().join(filename).to_string()
                }
            },
            DevMode::Plugin => {
                if filename.contains("plugin") {
                    self.target_dir().join("debug").join(filename).to_string()
                } else {
                    self.active_runtime_dir().join(filename).to_string()
                }
            },
        }
    }

    /// This assumes the path for the signatory keypair created by solana-test-validator
    /// is test-ledger/validator-keypair.json
    pub fn signatory(&self) -> String {
        env::current_dir()
            .map(|mut path| {
                path.extend(["test-ledger", "validator-keypair.json"]);
                path
            })
            .expect(&format!(
                "Unable to find location of validator-keypair.json"
            ))
            .to_string()
    }

    pub fn geyser_config(&self) -> String {
        self.active_runtime("geyser-plugin-config.json")
    }

    pub fn geyser_lib(&self) -> String {
        if matches!(self.dev_mode, DevMode::All | DevMode::Plugin) && env::consts::OS.to_lowercase().contains("mac") {
            self.active_runtime("libantegen_plugin.dylib")
        } else {
            self.active_runtime("libantegen_plugin.so")
        }
    }

    // Helper to check if we should use local programs
    pub fn use_local_programs(&self) -> bool {
        matches!(self.dev_mode, DevMode::All | DevMode::Programs)
    }
    
    // Helper to check if we should use local plugin
    pub fn use_local_plugin(&self) -> bool {
        matches!(self.dev_mode, DevMode::All | DevMode::Plugin)
    }
}

pub trait PathToString {
    fn to_string(&self) -> String;
}

impl PathToString for PathBuf {
    fn to_string(&self) -> String {
        self.clone().into_os_string().into_string().unwrap()
    }
}

// Antegen Deps Helpers
impl CliConfig {
    // #[tokio::main]
    fn detect_target_triplet() -> String {
        let output = std::process::Command::new("cargo")
            .arg("-vV")
            .output()
            .expect("failed to execute process");

        let host_prefix = "host:";
        String::from_utf8(output.stdout)
            .expect("Unable to get output from cargo -vV")
            .split('\n')
            .find(|line| line.trim_start().to_lowercase().starts_with(&host_prefix))
            .map(|line| line.trim_start_matches(&host_prefix).trim())
            .expect("Unable to detect target 'host' from cargo -vV")
            .to_owned()
    }

    pub fn antegen_release_url(tag: &str) -> String {
        format!(
            "{}/v{}/{}",
            ANTEGEN_RELEASE_URL,
            tag,
            &Self::antegen_release_archive()
        )
    }

    pub fn antegen_release_archive() -> String {
        let target_triplet = Self::detect_target_triplet();
        format!("antegen-geyser-plugin-release-{}.tar.xz", target_triplet)
    }

    pub fn solana_release_url(tag: &str) -> String {
        format!(
            "{}/{}/{}",
            SOLANA_RELEASE_BASE_URL,
            tag.replace("=", ""),
            &Self::solana_release_archive()
        )
    }

    pub fn solana_release_archive() -> String {
        let target_triplet = Self::detect_target_triplet();
        format!("solana-release-{}.tar.bz2", target_triplet)
    }
}
