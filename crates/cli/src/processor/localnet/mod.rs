pub mod client;
pub mod config;
pub mod validator;

use antegen_sdk::state::Trigger;
use anyhow::Result;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::read_keypair_file;
use std::path::PathBuf;
use std::str::FromStr;

use self::client::{create_client, ClientRunner, GeyserClient};
use self::config::{ClientConfig, GeyserClientConfig, LocalnetConfig};
use self::validator::{create_validator, SolanaValidator, Validator};
use crate::client::Client;
use crate::processor::thread;

/// Main localnet orchestrator
pub struct LocalnetOrchestrator {
    config: LocalnetConfig,
    validator: Option<Box<dyn Validator>>,
    clients: Vec<Box<dyn ClientRunner>>,
    runtime_dir: PathBuf,
    is_dev: bool,
}

impl LocalnetOrchestrator {
    /// Create a new orchestrator with default config
    pub fn new(is_dev: bool) -> Result<Self> {
        let config = LocalnetConfig::default()?;
        let runtime_dir = Self::get_runtime_dir(is_dev);

        Ok(Self {
            config,
            validator: None,
            clients: Vec::new(),
            runtime_dir,
            is_dev,
        })
    }

    /// Create with custom config file
    pub fn with_config(config_path: &PathBuf, is_dev: bool) -> Result<Self> {
        let config = LocalnetConfig::from_file(config_path)
            .map_err(|e| anyhow::anyhow!("Failed to load config: {}", e))?;
        let runtime_dir = Self::get_runtime_dir(is_dev);

        Ok(Self {
            config,
            validator: None,
            clients: Vec::new(),
            runtime_dir,
            is_dev,
        })
    }

    /// Override validator type
    pub fn set_validator(&mut self, validator_type: String) {
        self.config.validator.validator_type = validator_type;
    }

    /// Add a client configuration
    pub fn add_client(&mut self, client_type: String, name: Option<String>) {
        let name = name.unwrap_or_else(|| format!("{}-{}", client_type, self.clients.len()));

        // For now, just use an empty toml::Value - users should use config file for custom settings
        let config = toml::Value::Table(Default::default());

        self.config.clients.push(ClientConfig {
            client_type,
            name,
            config,
        });
    }

    /// Start everything
    pub fn start(&mut self) -> Result<()> {
        println!(
            "\n🚀 Starting Antegen localnet ({})",
            if self.is_dev { "dev" } else { "release" }
        );

        // Ensure runtime dir exists
        if !self.is_dev {
            std::fs::create_dir_all(&self.runtime_dir)?;
        }

        // Download dependencies if needed
        self.ensure_dependencies()?;

        // Start validator
        self.start_validator()?;

        // Start clients
        self.start_clients()?;

        // Initialize thread system
        if let Err(e) = self.initialize_thread_system() {
            eprintln!("Warning: Failed to initialize thread system: {}", e);
            eprintln!("The localnet is running but threads won't be available");
        }

        println!("\n✅ Localnet is running!");
        self.print_compact_status();

        Ok(())
    }

    /// Stop everything
    pub fn stop(&mut self) -> Result<()> {
        println!("Stopping Antegen localnet");

        // Stop clients first
        for client in &mut self.clients {
            if let Err(e) = client.stop() {
                eprintln!("Failed to stop client: {}", e);
            }
        }

        // Stop validator
        if let Some(validator) = &mut self.validator {
            validator.stop()?;
        }

        println!("Localnet stopped");
        Ok(())
    }

    /// Get status of all components
    pub fn status(&self) -> LocalnetStatus {
        LocalnetStatus {
            validator: self.validator.as_ref().map(|v| v.status()),
            clients: self.clients.iter().map(|c| c.status()).collect(),
        }
    }

    /// Print compact status to console
    pub fn print_compact_status(&self) {
        println!("\n  RPC URL:      {}", self.config.validator.rpc_url);
        println!("  WebSocket:    {}", self.config.validator.ws_url);
        println!("  Logs:         validator.log");
        println!("  Test Thread:  Runs every 30 seconds");
        println!("\n  💡 Tip: Use 'antegen localnet stop' to stop the validator");
    }

    /// Print detailed status to console
    pub fn print_status(&self) {
        let status = self.status();

        println!("\n=== Localnet Status ===");

        if let Some(validator_status) = status.validator {
            println!("Validator:");
            println!("  Running: {}", validator_status.running);
            println!("  RPC: {}", validator_status.rpc_url);
            println!("  WebSocket: {}", validator_status.ws_url);
            if let Some(pid) = validator_status.pid {
                println!("  PID: {}", pid);
            }
        }

        if !status.clients.is_empty() {
            println!("\nClients:");
            for client in status.clients {
                println!("  {} ({}):", client.name, client.client_type);
                println!("    Running: {}", client.running);
                if let Some(pid) = client.pid {
                    println!("    PID: {}", pid);
                }
            }
        }

        println!("\nLogs:");
        println!("  Validator: validator.log");
        for client in &self.clients {
            let client_status = client.status();
            if client_status.client_type == "carbon" {
                println!(
                    "  {}: carbon-{}.log",
                    client_status.name, client_status.name
                );
            }
        }
    }

    // Private helper methods

    fn initialize_thread_system(&self) -> Result<()> {
        print!("  Initializing thread system... ");
        std::io::Write::flush(&mut std::io::stdout()).ok();

        // Create RPC client
        let client = self.create_rpc_client()?;

        // Wait a bit for validator to be fully ready
        std::thread::sleep(std::time::Duration::from_secs(2));

        // Initialize thread config
        thread::init_config(&client)
            .map_err(|e| anyhow::anyhow!("Failed to initialize thread config: {}", e))?;

        // Create test thread
        let thread_id = "test-thread".to_string();
        let trigger = Trigger::Interval {
            seconds: 30,
            skippable: true,
        };

        thread::create(&client, thread_id.clone(), trigger)
            .map_err(|e| anyhow::anyhow!("Failed to create test thread: {}", e))?;

        // Create fiber with memo instruction
        let memo_program_id = Pubkey::from_str("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr")
            .map_err(|e| anyhow::anyhow!("Failed to parse memo program ID: {}", e))?;
        let test_instruction = solana_sdk::instruction::Instruction {
            program_id: memo_program_id,
            accounts: vec![],
            data: b"Thread execution test!".to_vec(),
        };

        thread::create_fiber(&client, thread_id, 0, test_instruction)
            .map_err(|e| anyhow::anyhow!("Failed to create test fiber: {}", e))?;

        println!("✓");
        Ok(())
    }

    fn create_rpc_client(&self) -> Result<Client> {
        // Use the default keypair
        let keypair_path = dirs_next::home_dir()
            .map(|mut path| {
                path.extend([".config", "solana", "id.json"]);
                path
            })
            .ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?;

        let payer = read_keypair_file(&keypair_path)
            .map_err(|e| anyhow::anyhow!("Failed to read keypair: {}", e))?;

        Ok(Client::new(payer, self.config.validator.rpc_url.clone()))
    }

    fn ensure_dependencies(&self) -> Result<()> {
        if self.is_dev {
            // In dev mode, download deps to target/VERSION directory
            let antegen_version = env!("CARGO_PKG_VERSION");
            let versioned_dir = PathBuf::from("target").join(antegen_version);
            let validator_path = versioned_dir.join("solana-test-validator");

            if !validator_path.exists() {
                // Try to download deps
                crate::deps::download_deps(
                    &PathBuf::from("target"),
                    false, // force_init
                    None,  // solana_archive
                    None,  // antegen_archive
                    true,  // dev
                )?;
            }

            // Check if Thread program is built
            let program_path = PathBuf::from("target/deploy/antegen_thread_program.so");
            if !program_path.exists() {
                println!("Building Thread program...");
                std::process::Command::new("anchor")
                    .args(&["build"])
                    .status()
                    .map_err(|e| anyhow::anyhow!("Failed to build Thread program: {}", e))?;
            }
        } else {
            // In release mode, download to runtime dir if needed
            crate::deps::download_deps(
                &self.runtime_dir,
                false, // force_init
                None,  // solana_archive
                None,  // antegen_archive
                false, // dev
            )?;
        }
        Ok(())
    }

    fn start_validator(&mut self) -> Result<()> {
        // Configure Geyser plugin args BEFORE creating validator
        if self.config.validator.validator_type == "solana" {
            for client_config in &self.config.clients {
                if client_config.client_type == "geyser" {
                    // Create Geyser client to get config
                    let mut geyser_client = self.create_geyser_client(client_config)?;
                    let _config_path = geyser_client.create_config_file(&self.get_config_dir())?;

                    // Add to validator args
                    for arg in geyser_client.get_validator_args() {
                        self.config.validator.extra_args.push(arg);
                    }
                }
            }
        }

        let mut validator = create_validator(self.config.validator.clone(), &self.runtime_dir)?;

        // Add Thread program for Solana validator
        if self.config.validator.validator_type == "solana" {
            if let Some(solana) = validator.as_any_mut().downcast_mut::<SolanaValidator>() {
                let program_path = if self.is_dev {
                    PathBuf::from("target/deploy/antegen_thread_program.so")
                } else {
                    self.runtime_dir.join("antegen_thread_program.so")
                };

                solana.add_program(antegen_sdk::ID, program_path);
            }
        }

        validator.start()?;
        self.validator = Some(validator);

        Ok(())
    }

    fn start_clients(&mut self) -> Result<()> {
        for client_config in &self.config.clients {
            if client_config.client_type == "geyser" {
                // Geyser is already configured with validator
                continue;
            }

            let mut client = create_client(client_config.clone(), &self.runtime_dir)?;
            client.start()?;
            self.clients.push(client);
        }

        Ok(())
    }

    fn create_geyser_client(&self, config: &ClientConfig) -> Result<GeyserClient> {
        let mut geyser_config: GeyserClientConfig = config.config.clone().try_into()?;

        // Populate RPC and WS URLs from validator config if not specified
        if geyser_config.rpc_url.is_none() {
            geyser_config.rpc_url = Some(self.config.validator.rpc_url.clone());
        }
        if geyser_config.ws_url.is_none() {
            geyser_config.ws_url = Some(self.config.validator.ws_url.clone());
        }

        let plugin_path = if self.is_dev {
            // On macOS, use .dylib extension, otherwise .so
            if cfg!(target_os = "macos") {
                PathBuf::from("target/debug/libantegen_client_geyser.dylib")
            } else {
                PathBuf::from("target/debug/libantegen_client_geyser.so")
            }
        } else {
            self.runtime_dir.join("libantegen_client_geyser.so")
        };

        Ok(GeyserClient::new(
            config.name.clone(),
            geyser_config,
            plugin_path,
        ))
    }

    fn get_runtime_dir(is_dev: bool) -> PathBuf {
        if is_dev {
            // In dev mode, use workspace root
            PathBuf::from(".")
        } else {
            // In release mode, use home directory
            dirs_next::home_dir()
                .map(|mut path| {
                    path.extend([".config", "antegen", "localnet", "runtime"]);
                    path
                })
                .unwrap_or_else(|| PathBuf::from("."))
        }
    }

    fn get_config_dir(&self) -> PathBuf {
        if self.is_dev {
            PathBuf::from("target/debug")
        } else {
            self.runtime_dir.join("config")
        }
    }
}

/// Status of the entire localnet
#[derive(Debug)]
pub struct LocalnetStatus {
    pub validator: Option<validator::ValidatorStatus>,
    pub clients: Vec<client::ClientStatus>,
}

// Public API functions for CLI integration

use crate::errors::CliError;
use crate::{print::print_style, print_status};
use once_cell::sync::Lazy;
use std::sync::Mutex;

// Global orchestrator for managing state across commands
static ORCHESTRATOR: Lazy<Mutex<Option<LocalnetOrchestrator>>> = Lazy::new(|| Mutex::new(None));

/// Start the localnet with specified configuration
pub fn start(
    config_path: Option<String>,
    validator: Option<String>,
    clients: Vec<String>,
    release: bool,
) -> Result<(), CliError> {
    // Default to dev mode (release = false means dev mode)
    let is_dev = !release;

    // Create orchestrator
    let mut orchestrator = if let Some(config_path) = config_path {
        let path = PathBuf::from(config_path);
        LocalnetOrchestrator::with_config(&path, is_dev)
            .map_err(|e| CliError::FailedLocalnet(e.to_string()))?
    } else {
        LocalnetOrchestrator::new(is_dev).map_err(|e| CliError::FailedLocalnet(e.to_string()))?
    };

    // Override validator if specified
    if let Some(validator_type) = validator {
        orchestrator.set_validator(validator_type);
    }

    // Add clients if specified
    for client in clients {
        // Parse client type and optional name (format: "type" or "type:name")
        let parts: Vec<&str> = client.split(':').collect();
        let client_type = parts[0].to_string();
        let name = parts.get(1).map(|s| s.to_string());
        orchestrator.add_client(client_type, name);
    }

    // Start everything
    orchestrator
        .start()
        .map_err(|e| CliError::FailedLocalnet(e.to_string()))?;

    // Store orchestrator for stop/status commands
    let mut guard = ORCHESTRATOR.lock().unwrap();
    *guard = Some(orchestrator);

    Ok(())
}

/// Stop the running localnet
pub fn stop() -> Result<(), CliError> {
    let mut guard = ORCHESTRATOR.lock().unwrap();

    if let Some(mut orchestrator) = guard.take() {
        orchestrator
            .stop()
            .map_err(|e| CliError::FailedLocalnet(e.to_string()))?;

        print_status!("Localnet", "Stopped successfully");
    } else {
        return Err(CliError::FailedLocalnet(
            "No localnet is running".to_string(),
        ));
    }

    Ok(())
}

/// Get status of the running localnet
pub fn status() -> Result<(), CliError> {
    let guard = ORCHESTRATOR.lock().unwrap();

    if let Some(orchestrator) = guard.as_ref() {
        orchestrator.print_status();
    } else {
        return Err(CliError::FailedLocalnet(
            "No localnet is running".to_string(),
        ));
    }

    Ok(())
}
