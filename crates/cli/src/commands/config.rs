//! Config file commands

use antegen_client::ClientConfig;
use anyhow::Result;
use std::path::PathBuf;

/// Strip surrounding quotes from a string (handles user input with accidental quotes)
fn strip_quotes(s: String) -> String {
    s.trim_matches(|c| c == '"' || c == '\'').to_string()
}

/// Generate a default configuration file
pub fn init(
    output: PathBuf,
    rpc: Option<String>,
    keypair_path: Option<String>,
    storage_path: Option<String>,
    force: bool,
) -> Result<()> {
    if output.exists() && !force {
        anyhow::bail!(
            "Config file already exists: {}. Use --force to overwrite.",
            output.display()
        );
    }

    let mut config = ClientConfig::default();

    // Apply overrides if provided (strip quotes for user-friendliness)
    if let Some(url) = rpc {
        config.rpc.endpoints[0].url = strip_quotes(url);
    }
    if let Some(path) = keypair_path {
        config.executor.keypair_path = strip_quotes(path);
    }
    if let Some(path) = storage_path {
        config.observability.storage_path = strip_quotes(path);
    }

    config.save(&output)?;

    // Set file permissions (640) and ownership (root:antegen) if possible
    #[cfg(unix)]
    {
        use file_owner::PathExt;
        use std::os::unix::fs::PermissionsExt;

        // Set permissions to 640 (owner rw, group r, others none)
        let _ = std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o640));

        // Set ownership to root:antegen (silently fails if not root or group doesn't exist)
        let _ = output.set_owner("root");
        let _ = output.set_group("antegen");
    }

    println!("✓ Generated config: {}", output.display());

    // Generate keypair if it doesn't exist
    let keypair_path = super::expand_tilde(&config.executor.keypair_path)?;
    let pubkey = super::ensure_keypair_exists(&keypair_path)?;
    println!("✓ Keypair: {} ({})", keypair_path.display(), pubkey);
    println!();
    println!("Next steps:");
    println!("  1. Fund address {} with SOL", pubkey);
    println!("  2. Run: antegen start -c {}", output.display());

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
    println!(
        "  Max concurrent threads: {}",
        config.processor.max_concurrent_threads
    );
    println!("  RPC endpoints: {}", config.rpc.endpoints.len());

    let datasource_count = config
        .rpc
        .endpoints
        .iter()
        .filter(|e| {
            matches!(
                e.role,
                antegen_client::config::EndpointRole::Datasource
                    | antegen_client::config::EndpointRole::Both
            )
        })
        .count();
    let submission_count = config
        .rpc
        .endpoints
        .iter()
        .filter(|e| {
            matches!(
                e.role,
                antegen_client::config::EndpointRole::Submission
                    | antegen_client::config::EndpointRole::Both
            )
        })
        .count();

    println!("    - Datasource endpoints: {}", datasource_count);
    println!("    - Submission endpoints: {}", submission_count);

    if config.observability.enabled {
        println!(
            "  Observability: enabled (storage: {})",
            config.observability.storage_path
        );
    } else {
        println!("  Observability: disabled");
    }

    Ok(())
}
