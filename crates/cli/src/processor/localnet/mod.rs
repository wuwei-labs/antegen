pub mod config;
pub mod daemon;
pub mod templates;

// Public API functions for CLI integration

use crate::errors::CliError;
use once_cell::sync::Lazy;
use serde_json;
use tokio::runtime::Runtime;

use self::config::ConfigBuilder;
use self::daemon::LocalnetDaemon;

// Global runtime for async operations
static RUNTIME: Lazy<Runtime> =
    Lazy::new(|| Runtime::new().expect("Failed to create tokio runtime"));

/// Start the localnet with specified configuration
pub fn start(
    _config_path: Option<String>,
    validator: Option<String>,
    clients: Vec<String>,
    release: bool,
) -> Result<(), CliError> {
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
        let mut config_builder = ConfigBuilder::new(is_dev);

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

        Ok(())
    })
}

/// Start localnet with Geyser plugin enabled
pub fn start_with_geyser(release: bool) -> Result<(), CliError> {
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
        let geyser_config = serde_json::json!({
            "libpath": if is_dev {
                "./target/debug/libantegen_geyser.so".to_string()
            } else {
                runtime_dir.join("libantegen_geyser.so").to_string_lossy().to_string()
            },
            "rpc_url": "http://localhost:8899",
            "keypair_path": runtime_dir.join("executor-keypair.json").to_string_lossy(),
            "forgo_commission": true,
            "enable_replay": false
        });

        std::fs::write(
            &geyser_config_path,
            serde_json::to_string_pretty(&geyser_config).unwrap(),
        )
        .map_err(|e| CliError::FailedLocalnet(format!("Failed to write Geyser config: {}", e)))?;

        println!("✓ Created Geyser plugin configuration");

        // Build configuration with Geyser-enabled validator
        let mut config_builder = ConfigBuilder::new(is_dev);

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
            templates::get_client_template(&client_type, &client_name, rpc_url, keypair)
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
