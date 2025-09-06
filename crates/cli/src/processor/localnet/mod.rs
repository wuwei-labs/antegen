pub mod config;
pub mod daemon;
pub mod templates;

// Public API functions for CLI integration

use crate::errors::CliError;
use once_cell::sync::Lazy;
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
