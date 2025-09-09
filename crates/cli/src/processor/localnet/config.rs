use anyhow::Result;
use std::path::PathBuf;

use super::templates;
use super::daemon::{AppConfig, ConfigFile};

/// Configuration builder for localnet services
pub struct ConfigBuilder {
    apps: Vec<AppConfig>,
    runtime_dir: PathBuf,
    is_dev: bool,
    verbose: bool,
}

impl ConfigBuilder {
    /// Create a new configuration builder
    pub fn new(is_dev: bool, verbose: bool) -> Self {
        let runtime_dir = dirs_next::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".antegen")
            .join("localnet");
        
        Self {
            apps: Vec::new(),
            runtime_dir,
            is_dev,
            verbose,
        }
    }
    
    /// Add the validator service
    pub fn add_validator(&mut self, _validator_type: String) -> &mut Self {
        // For now, we only support solana validator
        let config = templates::validator_service(None, &self.runtime_dir, self.is_dev);
        self.apps.push(config);
        self
    }
    
    /// Add the validator service with Geyser plugin
    pub fn add_validator_with_geyser(&mut self, geyser_config_path: Option<std::path::PathBuf>) -> &mut Self {
        let config = templates::validator_service(geyser_config_path, &self.runtime_dir, self.is_dev);
        self.apps.push(config);
        self
    }
    
    
    /// Add an RPC client
    pub fn add_rpc_client(&mut self, name: &str, rpc_url: Option<&str>) -> &mut Self {
        let url = rpc_url.unwrap_or("http://localhost:8899");
        let config = templates::rpc_service(name, url, &self.runtime_dir, self.is_dev, self.verbose);
        self.apps.push(config);
        self
    }
    
    /// Add a carbon client
    pub fn add_carbon_client(&mut self, name: &str, rpc_url: Option<&str>) -> &mut Self {
        let url = rpc_url.unwrap_or("http://localhost:8899");
        let config = templates::carbon_service(name, url, &self.runtime_dir, self.is_dev, self.verbose);
        self.apps.push(config);
        self
    }
    
    /// Add a generic client based on type
    pub fn add_client(&mut self, client_type: String, name: Option<String>) -> &mut Self {
        match client_type.as_str() {
            "rpc" => {
                let client_name = name.unwrap_or_else(|| format!("rpc-{}", chrono::Utc::now().timestamp()));
                self.add_rpc_client(&client_name, None)
            }
            "carbon" => {
                let client_name = name.unwrap_or_else(|| format!("carbon-{}", chrono::Utc::now().timestamp()));
                self.add_carbon_client(&client_name, None)
            }
            _ => {
                // For unknown types, skip silently for now
                self
            }
        }
    }
    
    /// Build the configuration
    pub fn build(&self) -> ConfigFile {
        ConfigFile {
            apps: self.apps.clone(),
        }
    }
    
    /// Write configuration to file
    pub fn write(&self) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.runtime_dir)?;
        let config_file = self.runtime_dir.join("config.json");
        
        let config = self.build();
        let json = serde_json::to_string_pretty(&config)?;
        std::fs::write(&config_file, json)?;
        
        Ok(config_file)
    }
    
    
    
}

