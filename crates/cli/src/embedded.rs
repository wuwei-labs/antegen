//! Embedded assets for the CLI
//!
//! This module uses rust-embed to bundle the example config into the CLI binary.
//! The Geyser plugin is downloaded from GitHub releases at runtime.

use anyhow::Result;
use rust_embed::RustEmbed;
use std::fs;
use std::path::Path;

// Embed the example config file (bundled with CLI crate)
#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/"]
#[include = "antegen.example.toml"]
pub struct ConfigAsset;

impl ConfigAsset {
    /// Extract the example config file to the specified path
    pub fn extract_example_config(dest: &Path) -> Result<()> {
        let config_data = Self::get("antegen.example.toml").ok_or_else(|| {
            anyhow::anyhow!("Embedded example config not found. This is a build error.")
        })?;

        // Create parent directory if needed
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write the config file
        fs::write(dest, config_data.data.as_ref())?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use antegen_client::ClientConfig;
    use tempfile::TempDir;

    #[test]
    fn test_extract_example_config() {
        let temp_dir = TempDir::new().unwrap();
        let dest = temp_dir.path().join("antegen.toml");

        let result = ConfigAsset::extract_example_config(&dest);

        // Config should always be available
        assert!(
            result.is_ok(),
            "Failed to extract example config: {:?}",
            result
        );
        assert!(dest.exists());

        // Verify it contains expected sections
        let content = std::fs::read_to_string(&dest).unwrap();
        assert!(content.contains("[executor]"));
        assert!(content.contains("[cache]"));
        assert!(content.contains("[load_balancer]"));
    }

    #[test]
    fn test_example_config_is_valid() {
        let temp_dir = TempDir::new().unwrap();
        let dest = temp_dir.path().join("antegen.toml");

        // Extract example config
        ConfigAsset::extract_example_config(&dest).unwrap();

        // Verify it can be loaded as valid ClientConfig
        let config = ClientConfig::load(&dest).expect("Example config should be valid");

        // Verify values match ClientConfig::default()
        let defaults = ClientConfig::default();

        assert_eq!(
            config.cache.max_capacity, defaults.cache.max_capacity,
            "cache.max_capacity should match default"
        );
        assert_eq!(
            config.load_balancer.grace_period_secs, defaults.load_balancer.grace_period_secs,
            "load_balancer.grace_period_secs should match default"
        );
        assert_eq!(
            config.datasources.commitment, defaults.datasources.commitment,
            "datasources.commitment should match default"
        );
        assert_eq!(
            config.processor.max_concurrent_threads, defaults.processor.max_concurrent_threads,
            "processor.max_concurrent_threads should match default"
        );
    }
}
