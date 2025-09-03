use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command};

use super::client::{create_client, ClientConfig, ClientRunner, ClientStatus};

/// Information about a running client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientInfo {
    pub name: String,
    pub client_type: String,
    pub pid: Option<u32>,
    pub config: toml::Value,
    pub log_file: Option<String>,
}

/// Manages standalone clients for a running localnet
pub struct ClientManager {
    state_file: PathBuf,
    runtime_dir: PathBuf,
    clients: HashMap<String, Box<dyn ClientRunner>>,
    configs: HashMap<String, ClientConfig>,  // Store configs for persistence
}

impl ClientManager {
    pub fn new(runtime_dir: PathBuf) -> Result<Self> {
        let state_file = runtime_dir.join("clients.json");
        
        // Load existing state if available
        let mut manager = Self {
            state_file,
            runtime_dir,
            clients: HashMap::new(),
            configs: HashMap::new(),
        };
        
        manager.load_state()?;
        Ok(manager)
    }
    
    /// Load client state from disk
    fn load_state(&mut self) -> Result<()> {
        if self.state_file.exists() {
            let content = fs::read_to_string(&self.state_file)?;
            let infos: Vec<ClientInfo> = serde_json::from_str(&content)?;
            
            // Recreate client runners from saved state
            for info in infos {
                let config = ClientConfig {
                    client_type: info.client_type.clone(),
                    name: info.name.clone(),
                    config: info.config,
                };
                
                // Only recreate standalone clients (not geyser)
                if config.client_type != "geyser" {
                    match create_client(config.clone(), &self.runtime_dir) {
                        Ok(client) => {
                            self.clients.insert(info.name.clone(), client);
                            self.configs.insert(info.name.clone(), config);
                        }
                        Err(e) => {
                            eprintln!("Warning: Failed to recreate client '{}': {}", info.name, e);
                        }
                    }
                }
            }
        }
        Ok(())
    }
    
    /// Save client state to disk
    fn save_state(&self) -> Result<()> {
        let infos: Vec<ClientInfo> = self.clients.iter().map(|(name, client)| {
            let status = client.status();
            let config = self.configs.get(name)
                .map(|c| c.config.clone())
                .unwrap_or_else(|| toml::Value::Table(Default::default()));
            ClientInfo {
                name: name.clone(),
                client_type: status.client_type,
                pid: status.pid,
                config,
                log_file: Some(format!("{}.log", name)),
            }
        }).collect();
        
        let json = serde_json::to_string_pretty(&infos)?;
        fs::write(&self.state_file, json)?;
        Ok(())
    }
    
    /// Add and start a new client
    pub fn add_client(&mut self, config: ClientConfig) -> Result<()> {
        // Check if client already exists
        if self.clients.contains_key(&config.name) {
            return Err(anyhow::anyhow!("Client '{}' already exists", config.name));
        }
        
        // Don't allow adding geyser clients (must be configured at validator startup)
        if config.client_type == "geyser" {
            return Err(anyhow::anyhow!("Geyser clients must be configured at localnet startup"));
        }
        
        println!("Adding {} client '{}'", config.client_type, config.name);
        
        // Create and start the client
        let mut client = create_client(config.clone(), &self.runtime_dir)?;
        client.start()?;
        
        // Store in our maps
        self.clients.insert(config.name.clone(), client);
        self.configs.insert(config.name.clone(), config.clone());
        
        // Save state
        self.save_state()?;
        
        println!("✓ Client '{}' started successfully", config.name);
        Ok(())
    }
    
    /// Remove and stop a client
    pub fn remove_client(&mut self, name: &str) -> Result<()> {
        match self.clients.remove(name) {
            Some(mut client) => {
                println!("Stopping client '{}'", name);
                client.stop()?;
                self.configs.remove(name);
                self.save_state()?;
                println!("✓ Client '{}' stopped and removed", name);
                Ok(())
            }
            None => Err(anyhow::anyhow!("Client '{}' not found", name))
        }
    }
    
    /// List all clients
    pub fn list_clients(&self) -> Vec<ClientStatus> {
        self.clients.values().map(|c| c.status()).collect()
    }
    
    /// Stop all clients
    pub fn stop_all(&mut self) -> Result<()> {
        for (name, client) in self.clients.iter_mut() {
            println!("Stopping client '{}'", name);
            if let Err(e) = client.stop() {
                eprintln!("Warning: Failed to stop client '{}': {}", name, e);
            }
        }
        self.clients.clear();
        self.configs.clear();
        self.save_state()?;
        Ok(())
    }
}

/// Build a default carbon client configuration for localnet
pub fn default_carbon_config(name: String) -> ClientConfig {
    let mut config_table = toml::map::Map::new();
    
    // Default to RPC datasource connecting to local validator
    // Note: RpcProgramSubscribe needs WebSocket URL, not HTTP
    config_table.insert("datasource".to_string(), toml::Value::String("rpc".to_string()));
    config_table.insert("rpc_url".to_string(), toml::Value::String("ws://localhost:8900".to_string()));
    
    // Use test keypair from runtime dir
    let keypair_path = dirs_next::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".antegen")
        .join("localnet")
        .join("executor-keypair.json");
    config_table.insert("keypair_path".to_string(), 
        toml::Value::String(keypair_path.to_string_lossy().to_string()));
    
    // Thread program ID (from localnet deployment)
    config_table.insert("thread_program_id".to_string(), 
        toml::Value::String("AgThdyi1P5RkVeZD2rQahTvs8HePJoGFFxKtvok5s2J1".to_string()));
    
    ClientConfig {
        client_type: "carbon".to_string(),
        name,
        config: toml::Value::Table(config_table),
    }
}