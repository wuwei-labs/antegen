use anyhow::Result;
use std::process::{Child, Command, Stdio};
use std::path::PathBuf;

use super::config::{GeyserClientConfig, CarbonClientConfig};
pub use super::config::ClientConfig;

/// Status of a client
#[derive(Debug, Clone)]
pub struct ClientStatus {
    pub name: String,
    pub client_type: String,
    pub running: bool,
    pub pid: Option<u32>,
}

/// Trait for client implementations
pub trait ClientRunner: Send + Sync {
    /// Start the client
    fn start(&mut self) -> Result<()>;
    
    /// Stop the client
    fn stop(&mut self) -> Result<()>;
    
    /// Get client status
    fn status(&self) -> ClientStatus;
}

/// Geyser plugin client (runs as part of validator)
pub struct GeyserClient {
    name: String,
    config: GeyserClientConfig,
    config_path: Option<PathBuf>,
    plugin_path: PathBuf,
}

impl GeyserClient {
    pub fn new(name: String, config: GeyserClientConfig, plugin_path: PathBuf) -> Self {
        Self {
            name,
            config,
            config_path: None,
            plugin_path,
        }
    }
    
    /// Create the plugin config file
    pub fn create_config_file(&mut self, target_dir: &PathBuf) -> Result<PathBuf> {
        // Use a consistent name for the plugin config
        let config_path = target_dir.join("geyser-plugin-config.json");
        
        // Ensure plugin path is absolute
        let absolute_plugin_path = if self.plugin_path.is_absolute() {
            self.plugin_path.clone()
        } else {
            std::env::current_dir()?.join(&self.plugin_path)
        };
        
        // Expand keypath if it starts with ~
        let expanded_keypath = if self.config.keypath.starts_with("~/") {
            dirs_next::home_dir()
                .map(|home| home.join(&self.config.keypath[2..]))
                .and_then(|p| p.to_str().map(|s| s.to_string()))
                .unwrap_or(self.config.keypath.clone())
        } else {
            self.config.keypath.clone()
        };
        
        // Create plugin config JSON with ALL required fields
        let plugin_config = serde_json::json!({
            "name": "antegen",
            "libpath": absolute_plugin_path.display().to_string(),
            "keypath": expanded_keypath,
            "rpc_url": self.config.rpc_url.as_ref().unwrap_or(&"http://localhost:8899".to_string()),
            "ws_url": self.config.ws_url.as_ref().unwrap_or(&"ws://localhost:8900".to_string()),
            "thread_count": self.config.thread_count,
            "transaction_timeout_threshold": self.config.transaction_timeout_threshold,
            "forgo_executor_commission": self.config.forgo_commission,
            "enable_replay": self.config.enable_replay,
            "nats_url": self.config.nats_url,
            "replay_delay_ms": self.config.replay_delay_ms,
        });
        
        std::fs::write(&config_path, serde_json::to_string_pretty(&plugin_config)?)?;
        self.config_path = Some(config_path.clone());
        
        Ok(config_path)
    }
    
    /// Get the validator args needed for this plugin
    pub fn get_validator_args(&self) -> Vec<String> {
        if let Some(config_path) = &self.config_path {
            vec![
                "--geyser-plugin-config".to_string(),
                config_path.to_string_lossy().to_string(),
            ]
        } else {
            vec![]
        }
    }
}

impl ClientRunner for GeyserClient {
    fn start(&mut self) -> Result<()> {
        println!("Geyser client '{}' configured (runs as validator plugin)", self.name);
        // Geyser runs as part of the validator, so no separate process
        Ok(())
    }
    
    fn stop(&mut self) -> Result<()> {
        // Stopped when validator stops
        Ok(())
    }
    
    fn status(&self) -> ClientStatus {
        ClientStatus {
            name: self.name.clone(),
            client_type: "geyser".to_string(),
            running: true, // Assume running if validator is running
            pid: None, // Part of validator process
        }
    }
}

/// Carbon standalone client
pub struct CarbonClient {
    name: String,
    config: CarbonClientConfig,
    process: Option<Child>,
    binary_path: PathBuf,
}

impl CarbonClient {
    pub fn new(name: String, config: CarbonClientConfig, binary_path: PathBuf) -> Self {
        Self {
            name,
            config,
            process: None,
            binary_path,
        }
    }
    
    fn build_command(&self) -> Command {
        let mut cmd = Command::new(&self.binary_path);
        
        // Set datasource
        cmd.arg("--datasource").arg(&self.config.datasource);
        
        // Set URLs based on datasource
        match self.config.datasource.as_str() {
            "rpc" => {
                if let Some(url) = &self.config.rpc_url {
                    cmd.arg("--rpc-url").arg(url);
                }
            }
            "yellowstone" => {
                if let Some(endpoint) = &self.config.endpoint {
                    cmd.arg("--yellowstone-endpoint").arg(endpoint);
                }
                if let Some(token) = &self.config.token {
                    cmd.arg("--yellowstone-token").arg(token);
                }
            }
            "helius" => {
                if let Some(url) = &self.config.rpc_url {
                    cmd.arg("--helius-ws-url").arg(url);
                }
                if let Some(token) = &self.config.token {
                    cmd.arg("--helius-api-key").arg(token);
                }
            }
            _ => {}
        }
        
        // Common args
        cmd.arg("--thread-program-id").arg(&self.config.thread_program_id);
        cmd.arg("--keypair").arg(&self.config.keypair_path);
        
        if self.config.forgo_commission.unwrap_or(false) {
            cmd.arg("--forgo-commission");
        }
        
        cmd
    }
}

impl ClientRunner for CarbonClient {
    fn start(&mut self) -> Result<()> {
        if self.process.is_some() {
            return Ok(()); // Already running
        }
        
        println!("Starting Carbon client '{}'", self.name);
        println!("  Datasource: {}", self.config.datasource);
        
        // Create log file
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(format!("carbon-{}.log", self.name))?;
        
        let mut cmd = self.build_command();
        let process = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::from(log_file.try_clone()?))
            .stderr(Stdio::from(log_file))
            .spawn()?;
        
        self.process = Some(process);
        println!("Carbon client '{}' started", self.name);
        
        Ok(())
    }
    
    fn stop(&mut self) -> Result<()> {
        if let Some(mut process) = self.process.take() {
            println!("Stopping Carbon client '{}'", self.name);
            process.kill()?;
            process.wait()?;
        }
        Ok(())
    }
    
    fn status(&self) -> ClientStatus {
        ClientStatus {
            name: self.name.clone(),
            client_type: "carbon".to_string(),
            running: self.process.is_some(),
            pid: self.process.as_ref().and_then(|p| p.id().try_into().ok()),
        }
    }
}

/// Factory function to create appropriate client
pub fn create_client(config: super::config::ClientConfig, runtime_dir: &PathBuf) -> Result<Box<dyn ClientRunner>> {
    match config.client_type.as_str() {
        "geyser" => {
            let geyser_config: GeyserClientConfig = config.config.try_into()?;
            let plugin_path = runtime_dir.join("libantegen_client_geyser.so");
            Ok(Box::new(GeyserClient::new(config.name, geyser_config, plugin_path)))
        }
        "carbon" => {
            let carbon_config: CarbonClientConfig = config.config.try_into()?;
            // In dev mode, try to use the local build first
            let binary_path = if let Ok(local_path) = std::env::current_exe() {
                let exe_dir = local_path.parent().unwrap();
                let local_carbon = exe_dir.join("antegen-carbon");
                if local_carbon.exists() {
                    local_carbon
                } else {
                    // Fall back to runtime dir
                    runtime_dir.join("antegen-carbon")
                }
            } else {
                runtime_dir.join("antegen-carbon")
            };
            
            if !binary_path.exists() {
                return Err(anyhow::anyhow!(
                    "Carbon client binary not found at {:?}. In dev mode, build with 'cargo build -p antegen-carbon'",
                    binary_path
                ));
            }
            
            Ok(Box::new(CarbonClient::new(config.name, carbon_config, binary_path)))
        }
        other => Err(anyhow::anyhow!("Unsupported client type: {}", other))
    }
}