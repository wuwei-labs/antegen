//! Config file commands

use crate::embedded::ConfigAsset;
use anyhow::Result;
use antegen_client::ClientConfig;
use std::path::PathBuf;

/// Generate a default configuration file with full documentation
pub fn init(output: PathBuf) -> Result<()> {
    if output.exists() {
        anyhow::bail!("Config file already exists: {}", output.display());
    }

    // Extract the example config with full documentation
    ConfigAsset::extract_example_config(&output)?;

    println!("✓ Generated config: {}", output.display());
    println!();
    println!("Next steps:");
    println!("  1. Edit the config file with your RPC endpoints and keypair path");
    println!("  2. Run: antegen run --config {}", output.display());

    Ok(())
}

/// Validate a configuration file
pub fn validate(config_path: PathBuf) -> Result<()> {
    println!("Validating config: {}", config_path.display());

    let config = ClientConfig::load(&config_path)?;

    println!("✓ Config is valid");
    println!();
    println!("Configuration summary:");
    println!("  Executor keypair: {}", config.executor.keypair_path);
    println!("  Thread program: {}", config.datasources.program_id());
    println!("  Max concurrent threads: {}", config.processor.max_concurrent_threads);
    println!("  RPC endpoints: {}", config.rpc.endpoints.len());

    let datasource_count = config.rpc.endpoints.iter()
        .filter(|e| matches!(e.role, antegen_client::config::EndpointRole::Datasource | antegen_client::config::EndpointRole::Both))
        .count();
    let submission_count = config.rpc.endpoints.iter()
        .filter(|e| matches!(e.role, antegen_client::config::EndpointRole::Submission | antegen_client::config::EndpointRole::Both))
        .count();

    println!("    - Datasource endpoints: {}", datasource_count);
    println!("    - Submission endpoints: {}", submission_count);

    if config.observability.enabled {
        println!("  Observability: enabled (storage: {})", config.observability.storage_path);
    } else {
        println!("  Observability: disabled");
    }

    Ok(())
}
