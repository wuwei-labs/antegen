use anyhow::Result;
use solana_sdk::pubkey::Pubkey;
use std::process::{Child, Command, Stdio};
use std::path::PathBuf;

use super::config::ValidatorConfig;

/// Status of a validator
#[derive(Debug, Clone)]
pub struct ValidatorStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub rpc_url: String,
    pub ws_url: String,
    pub validator_type: String,
}

/// Trait for validator implementations
pub trait Validator: Send + Sync {
    /// Start the validator
    fn start(&mut self) -> Result<()>;
    
    /// Stop the validator
    fn stop(&mut self) -> Result<()>;
    
    /// Get validator status
    fn status(&self) -> ValidatorStatus;
    
    /// Wait for validator to be ready
    fn wait_ready(&self, timeout_secs: u64) -> Result<()>;
    
    /// Get as Any for downcasting
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

/// Solana test validator implementation
pub struct SolanaValidator {
    config: ValidatorConfig,
    process: Option<Child>,
    binary_path: PathBuf,
    programs: Vec<(Pubkey, PathBuf)>,
    runtime_dir: PathBuf,
}

impl SolanaValidator {
    pub fn new(config: ValidatorConfig, binary_path: PathBuf, runtime_dir: PathBuf) -> Self {
        Self {
            config,
            process: None,
            binary_path,
            programs: Vec::new(),
            runtime_dir,
        }
    }
    
    /// Add a program to deploy
    pub fn add_program(&mut self, program_id: Pubkey, path: PathBuf) {
        self.programs.push((program_id, path));
    }
    
    fn build_command(&self) -> Command {
        let mut cmd = Command::new(&self.binary_path);
        
        // Basic args
        if self.config.reset {
            cmd.arg("--reset");
        }
        
        cmd.arg("--ledger").arg(&self.config.ledger_dir);
        cmd.arg("--rpc-port").arg(self.get_rpc_port());
        cmd.arg("--log");
        
        // Add programs
        for (program_id, path) in &self.programs {
            cmd.arg("--bpf-program")
               .arg(program_id.to_string())
               .arg(path);
        }
        
        // Add extra args
        for arg in &self.config.extra_args {
            cmd.arg(arg);
        }
        
        cmd
    }
    
    fn get_rpc_port(&self) -> String {
        // Extract port from URL
        self.config.rpc_url
            .split(':')
            .last()
            .unwrap_or("8899")
            .trim_end_matches('/')
            .to_string()
    }
}

impl Validator for SolanaValidator {
    fn start(&mut self) -> Result<()> {
        if self.process.is_some() {
            return Ok(()); // Already running
        }
        
        print!("  Starting Solana validator... ");
        std::io::Write::flush(&mut std::io::stdout()).ok();
        
        // Ensure runtime directory exists
        std::fs::create_dir_all(&self.runtime_dir)?;
        
        // Ensure ledger directory exists  
        std::fs::create_dir_all(&self.config.ledger_dir)?;
        
        // Create log file in runtime directory
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.runtime_dir.join("validator.log"))?;
        
        let mut cmd = self.build_command();
        
        // Set RUST_LOG to filter for Antegen-related logs
        cmd.env("RUST_LOG", "antegen=debug,antegen_thread_program=debug,antegen_network_program=debug,antegen_client_geyser=debug,antegen_submitter=debug,antegen_processor=debug,antegen_adapter=debug,antegen_client=debug,solana_runtime::system_instruction_processor=error,solana_runtime::bank=error");
        
        let process = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::from(log_file.try_clone()?))
            .stderr(Stdio::from(log_file))
            .spawn()?;
        
        self.process = Some(process);
        
        self.wait_ready(30)?;
        
        Ok(())
    }
    
    fn stop(&mut self) -> Result<()> {
        if let Some(mut process) = self.process.take() {
            println!("Stopping Solana validator");
            process.kill()?;
            process.wait()?;
        }
        Ok(())
    }
    
    fn status(&self) -> ValidatorStatus {
        ValidatorStatus {
            running: self.process.is_some(),
            pid: self.process.as_ref().and_then(|p| p.id().try_into().ok()),
            rpc_url: self.config.rpc_url.clone(),
            ws_url: self.config.ws_url.clone(),
            validator_type: self.config.validator_type.clone(),
        }
    }
    
    fn wait_ready(&self, timeout_secs: u64) -> Result<()> {
        use solana_client::rpc_client::RpcClient;
        use std::time::{Duration, Instant};
        
        let client = RpcClient::new(&self.config.rpc_url);
        let timeout = Duration::from_secs(timeout_secs);
        let start = Instant::now();
        
        while start.elapsed() < timeout {
            match client.get_version() {
                Ok(_) => {
                    println!("✓");
                    return Ok(());
                }
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
        
        Err(anyhow::anyhow!("Validator failed to start within {} seconds", timeout_secs))
    }
    
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Factory function to create appropriate validator
pub fn create_validator(config: ValidatorConfig, runtime_dir: &PathBuf) -> Result<Box<dyn Validator>> {
    match config.validator_type.as_str() {
        "solana" => {
            // First try to find solana-test-validator in PATH
            let binary_path = if let Ok(path_output) = std::process::Command::new("which")
                .arg("solana-test-validator")
                .output()
            {
                let path_str = String::from_utf8_lossy(&path_output.stdout);
                let path_str = path_str.trim();
                if !path_str.is_empty() && PathBuf::from(path_str).exists() {
                    PathBuf::from(path_str)
                } else {
                    // Try runtime dir
                    let runtime_path = runtime_dir.join("solana-test-validator");
                    if !runtime_path.exists() {
                        // Download if not present
                        println!("Downloading solana-test-validator...");
                        crate::deps::download_deps(
                            runtime_dir,
                            false, // force_init
                            None,  // solana_archive
                            None,  // antegen_archive
                            false, // dev mode
                        )?;
                    }
                    runtime_path
                }
            } else {
                // Try runtime dir
                let runtime_path = runtime_dir.join("solana-test-validator");
                if !runtime_path.exists() {
                    // Download if not present
                    println!("Downloading solana-test-validator...");
                    crate::deps::download_deps(
                        runtime_dir,
                        false, // force_init
                        None,  // solana_archive
                        None,  // antegen_archive
                        false, // dev mode
                    )?;
                }
                runtime_path
            };
            
            Ok(Box::new(SolanaValidator::new(config, binary_path, runtime_dir.clone())))
        }
        // Future: Add support for other validators
        // "firedancer" => ...
        other => Err(anyhow::anyhow!("Unsupported validator type: {}", other))
    }
}