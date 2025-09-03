pub mod client;
pub mod config;
pub mod manager;
pub mod validator;

use antegen_sdk::state::Trigger;
use anyhow::Result;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::read_keypair_file;
use solana_sdk::signer::Signer;
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
        std::fs::create_dir_all(&self.runtime_dir)?;
        
        // Ensure executor keypair exists
        self.ensure_executor_keypair()?;

        // Download dependencies if needed
        self.ensure_dependencies()?;

        // Start validator
        self.start_validator()?;
        
        // Fund executor keypair
        self.fund_executor()?;

        // Start clients
        self.start_clients()?;

        // Initialize thread system
        if let Err(e) = self.initialize_thread_system() {
            eprintln!("Warning: Failed to initialize thread system: {}", e);
            eprintln!("The localnet is running but threads won't be available");
        }

        // Show status
        self.print_status();

        Ok(())
    }

    /// Stop everything
    pub fn stop(&mut self) -> Result<()> {
        println!("\n🛑 Stopping Antegen localnet");

        // Stop clients first
        for client in &mut self.clients {
            client.stop().ok(); // Ignore errors on stop
        }

        // Stop validator
        if let Some(validator) = &mut self.validator {
            validator.stop()?;
        }

        println!("✓ Localnet stopped");
        Ok(())
    }

    /// Get status of all components
    pub fn status(&self) -> LocalnetStatus {
        LocalnetStatus {
            validator: self.validator.as_ref().map(|v| v.status()),
            clients: self.clients.iter().map(|c| c.status()).collect(),
        }
    }

    /// Print a formatted status report
    pub fn print_status(&self) {
        let status = self.status();

        println!("\n📊 Localnet Status");
        println!("━━━━━━━━━━━━━━━━");

        // Validator status
        if let Some(validator) = status.validator {
            let state = if validator.running { "✅ Running" } else { "⚠️  Stopped" };
            println!("Validator ({}):", validator.validator_type);
            println!("  State: {}", state);
            if let Some(pid) = validator.pid {
                println!("  PID: {}", pid);
            }
            println!("  RPC: {}", validator.rpc_url);
            println!("  WebSocket: {}", validator.ws_url);
        } else {
            println!("Validator: ⚠️  Not initialized");
        }

        // Client status
        println!("\nClients:");
        if status.clients.is_empty() {
            println!("  None configured (use --client to add)");
            println!("  Options: geyser, carbon");
        } else {
            for client in status.clients {
                let state = if client.running { "✅" } else { "⚠️ " };
                print!("  {} {} ({})", state, client.name, client.client_type);
                if let Some(pid) = client.pid {
                    print!(" - PID: {}", pid);
                }
                println!();
            }
        }
    }

    /// Initialize the thread system (thread config and test thread)
    fn initialize_thread_system(&self) -> Result<()> {
        print!("\nInitializing thread system... ");

        let keypair_path = self.get_executor_keypair_path();
        let rpc_url = "http://localhost:8899";

        let keypair = read_keypair_file(&keypair_path)
            .map_err(|e| anyhow::anyhow!("Failed to read executor keypair: {}", e))?;

        let client = Client::new(
            keypair.insecure_clone(),
            rpc_url.to_string(),
        );

        // Initialize thread config
        thread::init_config(&client)?;

        // Create a test thread with cron trigger
        let thread_id = "localnet-test-thread".to_string();
        let trigger = Trigger::Cron {
            schedule: "*/15 * * * * * *".to_string(), // Every 15 seconds
            skippable: false,
        };

        // Create thread (includes default fiber)
        thread::create(&client, thread_id.clone(), trigger)?;

        // Create an additional test fiber (optional)
        // The thread already has a default fiber, but we can add more
        let test_instruction = solana_sdk::instruction::Instruction {
            program_id: Pubkey::from_str("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr")?
                .to_owned(),
            accounts: vec![],
            data: b"Thread execution test!".to_vec(),
        };

        thread::create_fiber(&client, thread_id, 0, test_instruction, None)
            .map_err(|e| anyhow::anyhow!("Failed to create test fiber: {}", e))?;

        println!("✓");
        Ok(())
    }

    fn get_executor_keypair_path(&self) -> PathBuf {
        self.runtime_dir.join("executor-keypair.json")
    }
    
    /// Ensure executor keypair exists, create if not
    fn ensure_executor_keypair(&self) -> Result<()> {
        let keypair_path = self.get_executor_keypair_path();
        
        if !keypair_path.exists() {
            println!("  Creating executor keypair...");
            
            // Use solana-keygen to create a new keypair
            let output = std::process::Command::new("solana-keygen")
                .args(&[
                    "new",
                    "--no-bip39-passphrase",
                    "--outfile", keypair_path.to_str().unwrap(),
                    "--force"
                ])
                .output()
                .map_err(|e| anyhow::anyhow!("Failed to run solana-keygen: {}. Make sure Solana CLI is installed.", e))?;
            
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(anyhow::anyhow!("Failed to create keypair: {}", stderr));
            }
            
            println!("  ✓ Executor keypair created");
        }
        
        Ok(())
    }
    
    /// Fund the executor keypair if needed
    fn fund_executor(&self) -> Result<()> {
        use solana_client::rpc_client::RpcClient;
        use std::time::{Duration, Instant};
        
        let keypair_path = self.get_executor_keypair_path();
        let keypair = read_keypair_file(&keypair_path)
            .map_err(|e| anyhow::anyhow!("Failed to read executor keypair: {}", e))?;
        let rpc_client = RpcClient::new("http://localhost:8899");
        
        // Wait for validator to be ready
        let timeout = Duration::from_secs(10);
        let start = Instant::now();
        while start.elapsed() < timeout {
            if rpc_client.get_version().is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        
        // Check balance
        let balance = rpc_client.get_balance(&keypair.pubkey())
            .unwrap_or(0);
        
        // If balance is less than 1 SOL, airdrop 10 SOL
        if balance < 1_000_000_000 {
            print!("  Funding executor account...");
            std::io::Write::flush(&mut std::io::stdout()).ok();
            
            match rpc_client.request_airdrop(&keypair.pubkey(), 10_000_000_000) {
                Ok(sig) => {
                    // Wait for confirmation
                    let timeout = Duration::from_secs(30);
                    let start = Instant::now();
                    
                    while start.elapsed() < timeout {
                        if let Ok(confirmed) = rpc_client.confirm_transaction(&sig) {
                            if confirmed {
                                println!(" ✓ (10 SOL)");
                                return Ok(());
                            }
                        }
                        std::thread::sleep(Duration::from_millis(500));
                    }
                    
                    println!(" ✓ (10 SOL)");
                }
                Err(e) => {
                    // Non-fatal - localnet will run but thread system won't work
                    eprintln!(" ⚠️  Failed to airdrop: {}", e);
                    eprintln!("     You may need to fund the executor manually");
                }
            }
        }
        
        Ok(())
    }

    fn get_runtime_dir(is_dev: bool) -> PathBuf {
        // Always use ~/.antegen/localnet for consistency
        dirs_next::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".antegen")
            .join("localnet")
    }

    fn ensure_dependencies(&self) -> Result<()> {
        if self.is_dev {
            // In dev mode, check if solana-test-validator exists in target
            let validator_path = PathBuf::from("target/debug/solana-test-validator");
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
        // Check if any Geyser clients are configured
        let has_geyser = self.config.clients
            .iter()
            .any(|c| c.client_type == "geyser");
        
        // Only configure Geyser plugin if requested
        if has_geyser && self.config.validator.validator_type == "solana" {
            println!("  Configuring Geyser plugin...");
            
            for client_config in &self.config.clients {
                if client_config.client_type == "geyser" {
                    // Ensure plugin binary exists
                    self.ensure_geyser_plugin()?;
                    
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

        // Create and start validator with programs
        let mut validator = create_validator(
            self.config.validator.clone(),
            &self.runtime_dir,
        )?;
        
        // Add thread programs to deploy
        if self.config.validator.validator_type == "solana" {
            // Get the program path
            let program_path = if self.is_dev {
                PathBuf::from("target/deploy/antegen_thread_program.so")
            } else {
                self.runtime_dir.join("antegen_thread_program.so")
            };
            
            // Check if program exists
            if program_path.exists() {
                // Cast to SolanaValidator to add programs
                if let Some(solana_validator) = validator.as_any_mut().downcast_mut::<SolanaValidator>() {
                    use solana_sdk::pubkey::Pubkey;
                    use std::str::FromStr;
                    
                    // Thread program ID
                    let thread_program_id = Pubkey::from_str("AgThdyi1P5RkVeZD2rQahTvs8HePJoGFFxKtvok5s2J1")
                        .expect("Valid program ID");
                    
                    println!("  Deploying thread program...");
                    solana_validator.add_program(thread_program_id, program_path);
                }
            } else if self.is_dev {
                println!("  Warning: Thread program not found at {:?}", program_path);
                println!("  Programs will need to be deployed manually");
            }
        }

        validator.start()?;
        self.validator = Some(validator);

        Ok(())
    }

    fn ensure_geyser_plugin(&self) -> Result<()> {
        let plugin_name = if cfg!(target_os = "macos") {
            "libantegen_client_geyser.dylib"
        } else {
            "libantegen_client_geyser.so"
        };
        
        let plugin_path = self.runtime_dir.join("plugins").join(plugin_name);
        
        if !plugin_path.exists() {
            // Create plugins directory
            std::fs::create_dir_all(plugin_path.parent().unwrap())?;
            
            if self.is_dev {
                // In dev mode, build the plugin
                println!("    Building Geyser plugin...");
                std::process::Command::new("cargo")
                    .args(&["build", "--release", "-p", "antegen-client-geyser"])
                    .status()
                    .map_err(|e| anyhow::anyhow!("Failed to build Geyser plugin: {}", e))?;
                
                // Copy from target/release to runtime dir
                let built_path = PathBuf::from("target/release").join(plugin_name);
                if built_path.exists() {
                    std::fs::copy(&built_path, &plugin_path)?;
                } else {
                    return Err(anyhow::anyhow!("Built plugin not found at {:?}", built_path));
                }
            } else {
                // In production mode, download from releases
                return Err(anyhow::anyhow!(
                    "Plugin download not yet implemented. Please build in dev mode first."
                ));
            }
        }
        
        Ok(())
    }
    
    fn create_geyser_client(
        &self,
        client_config: &ClientConfig,
    ) -> Result<GeyserClient> {
        // Try to parse as Geyser config
        let geyser_config: GeyserClientConfig = client_config.config.clone().try_into()?;

        // Determine plugin path
        let plugin_name = if cfg!(target_os = "macos") {
            "libantegen_client_geyser.dylib"
        } else {
            "libantegen_client_geyser.so"
        };
        let plugin_path = self.runtime_dir.join("plugins").join(plugin_name);

        Ok(GeyserClient::new(
            client_config.name.clone(),
            geyser_config,
            plugin_path,
        ))
    }

    fn start_clients(&mut self) -> Result<()> {
        for client_config in &self.config.clients {
            match client_config.client_type.as_str() {
                "geyser" => {
                    // Geyser is handled as validator plugin
                    println!("Geyser client '{}' configured", client_config.name);
                }
                _ => {
                    // Other clients (carbon, etc.)
                    let mut client = create_client(client_config.clone(), &self.runtime_dir)?;
                    client.start()?;
                    self.clients.push(client);
                }
            }
        }

        Ok(())
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

    // Add clients
    for client_type in clients {
        orchestrator.add_client(client_type, None);
    }

    // Start everything
    orchestrator
        .start()
        .map_err(|e| CliError::FailedLocalnet(e.to_string()))?;

    // Store in global for other commands
    let mut guard = ORCHESTRATOR
        .lock()
        .map_err(|_| CliError::FailedLocalnet("Failed to acquire lock".to_string()))?;
    *guard = Some(orchestrator);

    Ok(())
}

/// Stop the running localnet
pub fn stop() -> Result<(), CliError> {
    let mut guard = ORCHESTRATOR
        .lock()
        .map_err(|_| CliError::FailedLocalnet("Failed to acquire lock".to_string()))?;

    if let Some(mut orchestrator) = guard.take() {
        orchestrator
            .stop()
            .map_err(|e| CliError::FailedLocalnet(e.to_string()))?;
    } else {
        return Err(CliError::FailedLocalnet(
            "No localnet is running".to_string(),
        ));
    }

    Ok(())
}

/// Get status of the running localnet
pub fn status() -> Result<(), CliError> {
    let guard = ORCHESTRATOR
        .lock()
        .map_err(|_| CliError::FailedLocalnet("Failed to acquire lock".to_string()))?;

    if let Some(orchestrator) = guard.as_ref() {
        orchestrator.print_status();
    } else {
        return Err(CliError::FailedLocalnet(
            "No localnet is running".to_string(),
        ));
    }

    Ok(())
}

/// Add a client to the running localnet
pub fn add_client(
    client_type: String,
    name: Option<String>,
    rpc_url: Option<String>,
    keypair: Option<String>,
) -> Result<(), CliError> {
    use self::manager::{ClientManager, default_carbon_config};
    
    // Get runtime dir
    let runtime_dir = dirs_next::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".antegen")
        .join("localnet");
    
    // Create manager
    let mut manager = ClientManager::new(runtime_dir.clone())
        .map_err(|e| CliError::FailedLocalnet(format!("Failed to create client manager: {}", e)))?;
    
    // Generate name if not provided
    let client_name = name.unwrap_or_else(|| {
        format!("{}-{}", client_type, chrono::Utc::now().timestamp())
    });
    
    // Build client config based on type
    let config = if client_type == "carbon" {
        let mut config = default_carbon_config(client_name.clone());
        
        // Override with provided options
        if let Some(rpc_url) = rpc_url {
            if let toml::Value::Table(ref mut table) = config.config {
                table.insert("rpc_url".to_string(), toml::Value::String(rpc_url));
            }
        }
        
        if let Some(keypair) = keypair {
            if let toml::Value::Table(ref mut table) = config.config {
                table.insert("keypair_path".to_string(), toml::Value::String(keypair));
            }
        }
        
        config
    } else {
        return Err(CliError::BadParameter(format!("Unsupported client type: {}", client_type)));
    };
    
    // Add the client
    manager.add_client(config)
        .map_err(|e| CliError::FailedLocalnet(format!("Failed to add client: {}", e)))?;
    
    Ok(())
}

/// Remove a client from the running localnet
pub fn remove_client(name: String) -> Result<(), CliError> {
    use self::manager::ClientManager;
    
    // Get runtime dir
    let runtime_dir = dirs_next::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".antegen")
        .join("localnet");
    
    // Create manager
    let mut manager = ClientManager::new(runtime_dir)
        .map_err(|e| CliError::FailedLocalnet(format!("Failed to create client manager: {}", e)))?;
    
    // Remove the client
    manager.remove_client(&name)
        .map_err(|e| CliError::FailedLocalnet(format!("Failed to remove client: {}", e)))?;
    
    Ok(())
}

/// List all clients in the running localnet
pub fn list_clients() -> Result<(), CliError> {
    use self::manager::ClientManager;
    
    // Get runtime dir
    let runtime_dir = dirs_next::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".antegen")
        .join("localnet");
    
    // Create manager
    let manager = ClientManager::new(runtime_dir)
        .map_err(|e| CliError::FailedLocalnet(format!("Failed to create client manager: {}", e)))?;
    
    // List clients
    let clients = manager.list_clients();
    
    if clients.is_empty() {
        println!("No clients currently running");
    } else {
        println!("Running clients:");
        for client in clients {
            println!("  {} ({}): PID={:?}", 
                client.name, 
                client.client_type,
                client.pid.unwrap_or(0)
            );
        }
    }
    
    Ok(())
}