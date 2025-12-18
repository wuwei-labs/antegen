//! Geyser plugin commands

use crate::download::{
    current_version, download_geyser_plugin, get_library_filename, needs_update, save_version_info,
};
use antegen_client::ClientConfig;
use anyhow::Result;
use std::path::PathBuf;

/// Initialize Geyser plugin for validator
pub async fn init(output: PathBuf, config_path: PathBuf) -> Result<()> {
    println!("Initializing Geyser plugin...");

    // Determine plugin directory
    let plugin_dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?
        .join("antegen");

    // Create plugin directory if needed
    std::fs::create_dir_all(&plugin_dir)?;

    // Determine plugin .so extraction path
    let so_filename = get_library_filename();
    let so_path = plugin_dir.join(&so_filename);

    // Download the plugin if needed
    let version = current_version();
    if needs_update(&so_path, version)? {
        download_geyser_plugin(version, &so_path).await?;
        save_version_info(&so_path, version)?;
    } else {
        println!("Plugin already downloaded (version {})", version);
    }
    println!("  Plugin: {}", so_path.display());

    // Determine config file path
    // If default "antegen.toml", put it in the plugin directory
    // Otherwise use the user-specified path
    let final_config_path = if config_path == PathBuf::from("antegen.toml") {
        plugin_dir.join("antegen.toml")
    } else {
        config_path
    };

    // Generate config if it doesn't exist
    if !final_config_path.exists() {
        ClientConfig::default().save(&final_config_path)?;
        println!("  Generated config: {}", final_config_path.display());
        println!();
        println!(
            "  IMPORTANT: Edit {} before running the validator!",
            final_config_path.display()
        );
        println!("   - Set your executor keypair path");
        println!("   - Configure RPC endpoints");
    } else {
        println!(
            "  Config file already exists: {}",
            final_config_path.display()
        );
    }

    // Convert config path to absolute path for validator config
    let absolute_config_path = std::fs::canonicalize(&final_config_path)
        .unwrap_or_else(|_| final_config_path.clone());

    // Generate validator plugin config with config file path
    let plugin_config = serde_json::json!({
        "libpath": so_path.display().to_string(),
        "config": absolute_config_path.display().to_string(),
    });

    std::fs::write(&output, serde_json::to_string_pretty(&plugin_config)?)?;
    println!("  Generated validator config: {}", output.display());

    println!();
    println!("Next steps:");
    println!("  1. Review and edit {}", absolute_config_path.display());
    println!(
        "  2. Run: agave-validator --geyser-plugin-config {}",
        output.display()
    );

    Ok(())
}

/// Extract plugin .so to custom location
pub async fn extract(output: PathBuf) -> Result<()> {
    println!("Downloading Geyser plugin...");

    let version = current_version();
    download_geyser_plugin(version, &output).await?;
    save_version_info(&output, version)?;

    println!("  Extracted plugin to: {}", output.display());

    Ok(())
}
