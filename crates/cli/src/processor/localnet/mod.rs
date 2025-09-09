pub mod config;
pub mod daemon;
pub mod templates;

// Public API functions for CLI integration

use crate::{client::Client, errors::CliError};
use once_cell::sync::Lazy;
use serde_json;
use solana_sdk::signature::read_keypair_file;
use std::process::Command;
use tokio::runtime::Runtime;

use self::config::ConfigBuilder;
use self::daemon::LocalnetDaemon;

// Global runtime for async operations
static RUNTIME: Lazy<Runtime> =
    Lazy::new(|| Runtime::new().expect("Failed to create tokio runtime"));

// Required Solana version for compatibility with Geyser plugin
const REQUIRED_SOLANA_VERSION: &str = "2.2";

/// Initialize the thread program config
fn initialize_thread_config() -> Result<(), CliError> {
    // Get the executor keypair path
    let runtime_dir = dirs_next::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".antegen")
        .join("localnet");
    
    let keypair_path = runtime_dir.join("executor-keypair.json");
    
    // Load the executor keypair
    let payer = read_keypair_file(&keypair_path)
        .map_err(|e| CliError::FailedLocalnet(format!("Failed to read executor keypair: {}", e)))?;
    
    // Create a client with localnet RPC
    let client = Client::new(payer, "http://localhost:8899".to_string());
    
    // Airdrop SOL to the executor account
    println!("  Airdropping SOL to executor account...");
    let airdrop_result = Command::new("solana")
        .args(&[
            "airdrop",
            "10",
            &client.payer_pubkey().to_string(),
            "--url", "http://localhost:8899"
        ])
        .output();
    
    if let Err(e) = airdrop_result {
        eprintln!("  Warning: Failed to airdrop SOL: {}", e);
    }
    
    // Wait a moment for airdrop to be confirmed
    std::thread::sleep(std::time::Duration::from_secs(1));
    
    // Try to initialize the config with retries (program may not be fully deployed yet)
    let mut attempts = 0;
    let max_attempts = 3;
    
    while attempts < max_attempts {
        attempts += 1;
        
        match crate::processor::config::init(&client, None) {
            Ok(_) => return Ok(()),
            Err(e) => {
                if attempts == max_attempts {
                    // On final attempt, log the error but don't fail
                    eprintln!("  Warning: Could not initialize thread config after {} attempts: {}", max_attempts, e);
                    eprintln!("  You can manually initialize it later with: antegen config init");
                    return Ok(()); // Return success anyway - config can be initialized later
                } else {
                    // Wait before retry
                    println!("  Config initialization attempt {} failed, retrying...", attempts);
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
            }
        }
    }
    
    Ok(())
}

/// Check if the installed Solana version matches requirements
fn check_solana_version() -> Result<(), CliError> {
    // Run solana --version command
    let output = Command::new("solana")
        .arg("--version")
        .output()
        .map_err(|e| CliError::FailedLocalnet(format!("Failed to check Solana version: {}", e)))?;

    if !output.status.success() {
        return Err(CliError::FailedLocalnet(
            "Failed to get Solana version. Is Solana CLI installed?".to_string()
        ));
    }

    let version_str = String::from_utf8_lossy(&output.stdout);
    
    // Parse version (format: "solana-cli 2.2.1 (src:xxx; feat:xxx, client:Agave)")
    if let Some(version_part) = version_str.split_whitespace().nth(1) {
        // Check if it starts with 2.2
        if version_part.starts_with(REQUIRED_SOLANA_VERSION) {
            println!("✓ Solana version {} is compatible", version_part);
            return Ok(());
        } else {
            eprintln!("⚠️  Solana version mismatch detected!");
            eprintln!("   Current version: {}", version_part);
            eprintln!("   Required version: {}.*", REQUIRED_SOLANA_VERSION);
            eprintln!("");
            eprintln!("   The Geyser plugin requires Solana {} for ABI compatibility.", REQUIRED_SOLANA_VERSION);
            eprintln!("   Please install the correct version:");
            eprintln!("");
            eprintln!("   sh -c \"$(curl -sSfL https://release.anza.xyz/v2.2.1/install)\"");
            eprintln!("");
            return Err(CliError::FailedLocalnet(
                format!("Solana version {} required, found {}", REQUIRED_SOLANA_VERSION, version_part)
            ));
        }
    }

    Err(CliError::FailedLocalnet(
        "Could not parse Solana version".to_string()
    ))
}

/// Start the localnet with specified configuration
pub fn start(
    _config_path: Option<String>,
    validator: Option<String>,
    clients: Vec<String>,
    release: bool,
    verbose: bool,
) -> Result<(), CliError> {
    // Check Solana version compatibility first
    check_solana_version()?;

    // Default to dev mode (release = false means dev mode)
    let is_dev = !release;

    RUNTIME.block_on(async {
        println!("🚀 Starting Antegen localnet with PMDaemon...");

        // Create daemon
        let mut daemon = LocalnetDaemon::new(is_dev)
            .await
            .map_err(|e| CliError::FailedLocalnet(e.to_string()))?;

        println!("✓ PMDaemon initialized");

        // Build configuration
        let mut config_builder = ConfigBuilder::new(is_dev, verbose);

        // Add validator
        let validator_type = validator.unwrap_or_else(|| "solana".to_string());
        config_builder.add_validator(validator_type);

        // Add clients
        for client_type in clients {
            config_builder.add_client(client_type, None);
        }

        // Write configuration
        println!("✓ Building configuration...");
        let config_path = config_builder
            .write()
            .map_err(|e| CliError::FailedLocalnet(e.to_string()))?;
        println!("✓ Config written to: {:?}", config_path);

        // Start everything
        println!("✓ Starting services via PMDaemon...");
        daemon
            .start()
            .await
            .map_err(|e| CliError::FailedLocalnet(e.to_string()))?;
        println!("✓ All services started");

        // Wait a moment for validator to be ready
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        
        // Initialize thread config
        println!("✓ Initializing thread program config...");
        initialize_thread_config().ok(); // Best effort - warning already printed if fails

        Ok(())
    })
}

/// Start localnet with Geyser plugin enabled
pub fn start_with_geyser(release: bool, verbose: bool) -> Result<(), CliError> {
    let is_dev = !release;

    RUNTIME.block_on(async {
        println!("🚀 Starting Antegen localnet with Geyser plugin...");

        // Create daemon
        let mut daemon = LocalnetDaemon::new(is_dev)
            .await
            .map_err(|e| CliError::FailedLocalnet(e.to_string()))?;

        println!("✓ PMDaemon initialized");

        // Create Geyser plugin configuration
        let runtime_dir = dirs_next::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".antegen")
            .join("localnet");

        // Ensure runtime directory exists
        std::fs::create_dir_all(&runtime_dir).map_err(|e| {
            CliError::FailedLocalnet(format!("Failed to create runtime dir: {}", e))
        })?;

        // Create Geyser plugin config file
        let geyser_config_path = runtime_dir.join("geyser-plugin-config.json");
        let lib_extension = if cfg!(target_os = "macos") { "dylib" } else { "so" };
        let geyser_config = serde_json::json!({
            "libpath": if is_dev {
                // Use absolute path to the library in dev mode
                std::env::current_dir()
                    .unwrap()
                    .join(format!("target/debug/libantegen_client_geyser.{}", lib_extension))
                    .to_string_lossy()
                    .to_string()
            } else {
                runtime_dir.join(format!("libantegen_client_geyser.{}", lib_extension)).to_string_lossy().to_string()
            },
            "name": "antegen",
            "rpc_url": "http://localhost:8899",
            "ws_url": "ws://localhost:8900",
            "keypath": runtime_dir.join("executor-keypair.json").to_string_lossy(),
            "thread_count": 10,
            "transaction_timeout_threshold": 150
        });

        std::fs::write(
            &geyser_config_path,
            serde_json::to_string_pretty(&geyser_config).unwrap(),
        )
        .map_err(|e| CliError::FailedLocalnet(format!("Failed to write Geyser config: {}", e)))?;

        println!("✓ Created Geyser plugin configuration");

        // Build configuration with Geyser-enabled validator
        let mut config_builder = ConfigBuilder::new(is_dev, verbose);

        // Add validator with Geyser plugin
        config_builder.add_validator_with_geyser(Some(geyser_config_path));

        // Write configuration
        println!("✓ Building configuration...");
        let config_path = config_builder
            .write()
            .map_err(|e| CliError::FailedLocalnet(e.to_string()))?;
        println!("✓ Config written to: {:?}", config_path);

        // Start everything
        println!("✓ Starting services via PMDaemon...");
        daemon
            .start()
            .await
            .map_err(|e| CliError::FailedLocalnet(e.to_string()))?;

        // Wait a moment for validator to be ready
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        
        // Initialize thread config
        println!("✓ Initializing thread program config...");
        initialize_thread_config().ok(); // Best effort - warning already printed if fails
        
        println!("\n✨ Localnet with Geyser plugin is running!");
        println!("\n📝 Available endpoints:");
        println!("  • RPC:     http://localhost:8899");
        println!("  • Faucet:  http://localhost:9900");
        println!("  • Geyser:  Integrated with validator");
        println!("\n💡 Tips:");
        println!("  • View logs:   antegen localnet status");
        println!("  • Stop:        antegen localnet stop");
        println!("  • Add client:  antegen localnet client add --type rpc");

        Ok(())
    })
}

/// Stop the running localnet
pub fn stop() -> Result<(), CliError> {
    RUNTIME.block_on(async {
        // Always create a new daemon instance to connect to existing processes
        let mut daemon = LocalnetDaemon::new(std::path::Path::new("target").exists())
            .await
            .map_err(|e| CliError::FailedLocalnet(e.to_string()))?;

        daemon
            .stop()
            .await
            .map_err(|e| CliError::FailedLocalnet(e.to_string()))?;

        println!("✓ Localnet stopped");
        Ok(())
    })
}

/// Get status of the running localnet
pub fn status() -> Result<(), CliError> {
    RUNTIME.block_on(async {
        // Always create a new daemon instance to connect to existing processes
        let daemon = LocalnetDaemon::new(std::path::Path::new("target").exists())
            .await
            .map_err(|e| CliError::FailedLocalnet(e.to_string()))?;

        let services = daemon
            .status()
            .await
            .map_err(|e| CliError::FailedLocalnet(e.to_string()))?;

        if services.is_empty() {
            println!("No localnet services are running");
        } else {
            // Print formatted status
            println!("\n📊 Localnet Status");
            println!("━━━━━━━━━━━━━━━━");

            for service in services {
                let state = if service.status == "Online" {
                    "✅"
                } else {
                    "⚠️"
                };
                print!("  {} {} ({})", state, service.name, service.status);
                if let Some(pid) = service.pid {
                    print!(" - PID: {}", pid);
                }
                if let Some(started) = service.started_at {
                    print!(" - Started: {}", started);
                }
                println!();
            }
        }

        Ok(())
    })
}

/// Add a client to the running localnet
pub fn add_client(
    client_type: String,
    name: Option<String>,
    rpc_url: Option<String>,
    keypair: Option<String>,
    verbose: bool,
) -> Result<(), CliError> {
    RUNTIME.block_on(async {
        // Create a daemon instance to connect to existing processes
        let mut daemon = LocalnetDaemon::new(std::path::Path::new("target").exists())
            .await
            .map_err(|e| CliError::FailedLocalnet(e.to_string()))?;

        // Check if localnet is actually running
        let services = daemon
            .status()
            .await
            .map_err(|e| CliError::FailedLocalnet(e.to_string()))?;

        if services.is_empty() {
            return Err(CliError::FailedLocalnet(
                "No localnet is running".to_string(),
            ));
        }

        // Generate name if not provided
        let client_name =
            name.unwrap_or_else(|| format!("{}-{}", client_type, chrono::Utc::now().timestamp()));

        // Get client template
        let app_config =
            templates::get_client_template(&client_type, &client_name, rpc_url, keypair, verbose)
                .map_err(|e| CliError::FailedLocalnet(e.to_string()))?;

        // Add the service
        daemon
            .add_service(app_config)
            .await
            .map_err(|e| CliError::FailedLocalnet(format!("Failed to add client: {}", e)))?;

        println!("✅ Client '{}' added successfully", client_name);
        Ok(())
    })
}

/// Remove a client from the running localnet
pub fn remove_client(name: Option<String>) -> Result<(), CliError> {
    RUNTIME.block_on(async {
        // Create a daemon instance to connect to existing processes
        let mut daemon = LocalnetDaemon::new(std::path::Path::new("target").exists())
            .await
            .map_err(|e| CliError::FailedLocalnet(e.to_string()))?;

        // Get the client name, either from the argument or from interactive selection
        let client_name = match name {
            Some(n) => n,
            None => {
                // Get list of running clients
                let services = daemon
                    .status()
                    .await
                    .map_err(|e| CliError::FailedLocalnet(e.to_string()))?;

                // Filter to only show clients (not validator)
                let clients: Vec<_> = services
                    .into_iter()
                    .filter(|s| !s.name.contains("validator"))
                    .map(|s| s.name)
                    .collect();

                if clients.is_empty() {
                    return Err(CliError::FailedLocalnet(
                        "No clients currently running".to_string(),
                    ));
                }

                // Show interactive selection
                use dialoguer::Select;
                let selection = Select::new()
                    .with_prompt("Select a client to remove")
                    .items(&clients)
                    .default(0) // Start with first item selected
                    .interact()
                    .map_err(|e| {
                        CliError::FailedLocalnet(format!("Failed to select client: {}", e))
                    })?;

                clients[selection].clone()
            }
        };

        daemon
            .remove_service(&client_name)
            .await
            .map_err(|e| CliError::FailedLocalnet(format!("Failed to remove client: {}", e)))?;

        println!("✅ Client '{}' removed successfully", client_name);
        Ok(())
    })
}

/// List all clients in the running localnet
pub fn list_clients() -> Result<(), CliError> {
    RUNTIME.block_on(async {
        // Create a daemon instance to connect to existing processes
        let daemon = LocalnetDaemon::new(std::path::Path::new("target").exists())
            .await
            .map_err(|e| CliError::FailedLocalnet(e.to_string()))?;

        let services = daemon
            .status()
            .await
            .map_err(|e| CliError::FailedLocalnet(e.to_string()))?;

        // Filter to show only client services (not validator)
        let clients: Vec<_> = services
            .into_iter()
            .filter(|s| !s.name.contains("validator"))
            .collect();

        if clients.is_empty() {
            println!("No clients currently running");
        } else {
            println!("Running clients:");
            for client in clients {
                let state = if client.status == "Online" {
                    "✅"
                } else {
                    "⚠️"
                };
                print!("  {} {}", state, client.name);
                if let Some(pid) = client.pid {
                    print!(" - PID: {}", pid);
                }
                println!();
            }
        }

        Ok(())
    })
}
