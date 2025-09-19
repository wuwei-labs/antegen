use anyhow::{bail, Result};
use pmdaemon::{ProcessManager, ProcessState};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// PMDaemon wrapper for managing localnet services
pub struct LocalnetDaemon {
    config_file: PathBuf,
    process_manager: ProcessManager,
}

impl LocalnetDaemon {
    /// Create a new LocalnetDaemon instance
    pub async fn new(_is_dev: bool) -> Result<Self> {
        let runtime_dir = Self::get_runtime_dir();
        std::fs::create_dir_all(&runtime_dir)?;

        let config_file = runtime_dir.join("config.json");

        // Initialize PMDaemon process manager
        let process_manager = ProcessManager::new().await?;

        Ok(Self {
            config_file,
            process_manager,
        })
    }

    /// Start services defined in config.json
    pub async fn start(&mut self) -> Result<()> {
        // Load and start configuration if it exists
        if self.config_file.exists() {
            println!("  Loading config from: {:?}", self.config_file);
            let content = std::fs::read_to_string(&self.config_file)?;
            let config: ConfigFile = serde_json::from_str(&content)?;

            println!("  Found {} services to start", config.apps.len());
            for app in config.apps {
                println!("  Starting service: {}", app.name);

                // Stop existing service if it exists (following the example pattern)
                if let Ok(processes) = self.process_manager.list().await {
                    if processes.iter().any(|p| p.name == app.name) {
                        println!(
                            "    Service {} already exists, stopping it first...",
                            app.name
                        );
                        if let Err(e) = self.process_manager.stop(&app.name).await {
                            println!("    Warning: Failed to stop {}: {}", app.name, e);
                        }
                        // Wait for it to actually stop
                        let _ = self
                            .wait_for_process_stop(&app.name, Duration::from_secs(5))
                            .await;
                        // Delete from PMDaemon's tracking
                        let _ = self.process_manager.delete(&app.name).await;
                    }
                }

                // Start new service
                let config = self.process_config_from_app(&app)?;
                let process_id = self.process_manager.start(config).await?;
                println!("    ✓ {} started with process ID: {}", app.name, process_id);
            }
        } else {
            println!(
                "  Warning: No config.json found at {:?}",
                self.config_file
            );
        }

        // Wait for validator to be healthy
        println!("  Waiting for validator to become healthy...");
        self.wait_for_health("antegen-validator", Duration::from_secs(30))
            .await?;
        println!("  ✓ Validator is healthy");

        Ok(())
    }

    /// Stop all services
    pub async fn stop(&mut self) -> Result<()> {
        let processes = self.process_manager.list().await?;
        
        // Get the list of service names to stop
        let service_names: Vec<String> = processes.iter().map(|p| p.name.clone()).collect();

        for service_name in service_names {
            // Use the helper to stop and kill each service
            if let Err(e) = self.stop_service_with_kill(&service_name).await {
                println!("    Warning: Failed to stop {}: {}", service_name, e);
            }

            // Clean up from PMDaemon's tracking
            let _ = self.process_manager.delete(&service_name).await;
        }

        Ok(())
    }

    /// Wait for a process to stop
    async fn wait_for_process_stop(&self, name: &str, timeout: Duration) -> Result<()> {
        let start = std::time::Instant::now();

        while start.elapsed() < timeout {
            // Check if process still exists in PMDaemon's list
            let processes = self.process_manager.list().await?;

            // Check the specific process
            if let Some(process) = processes.iter().find(|p| p.name == name) {
                // Check if PMDaemon considers it stopped
                if process.state == ProcessState::Stopped {
                    // Don't trust PMDaemon's state, verify the actual process
                }

                // Also check if the actual OS process is gone
                if let Some(pid) = process.pid {
                    // Check if process exists using kill -0 (doesn't actually kill, just checks)
                    let output = std::process::Command::new("kill")
                        .arg("-0")
                        .arg(pid.to_string())
                        .output();

                    if let Ok(output) = output {
                        if !output.status.success() {
                            // Process doesn't exist anymore
                            return Ok(());
                        }
                    }
                }
            } else {
                // Process not found in list - it's gone
                return Ok(());
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        bail!("Process {} did not stop within timeout", name)
    }

    /// Stop a service and ensure its process is killed
    async fn stop_service_with_kill(&mut self, service_name: &str) -> Result<()> {
        // Get the process info before stopping
        let processes = self.process_manager.list().await?;
        let process = processes
            .iter()
            .find(|p| p.name == service_name);
        
        // If process not found, it might already be stopped
        let process = match process {
            Some(p) => p,
            None => {
                println!("  Service {} not found in process list", service_name);
                return Ok(());
            }
        };
        
        println!("  Stopping {} (PID: {:?})", service_name, process.pid);
        
        // Store the PID before calling stop (in case PMDaemon clears it)
        let original_pid = process.pid;
        
        // Call PMDaemon's stop - this might just mark it as stopped without killing
        if let Err(e) = self.process_manager.stop(service_name).await {
            println!("    Warning: PMDaemon stop failed: {}", e);
        }
        
        // PMDaemon's stop doesn't reliably kill processes, so we need to ensure it's dead
        if let Some(pid) = original_pid {
            // Give it a moment to stop gracefully
            tokio::time::sleep(Duration::from_millis(500)).await;
            
            // Check if it's actually stopped
            let check = std::process::Command::new("kill")
                .arg("-0")
                .arg(pid.to_string())
                .output();
            
            if let Ok(output) = check {
                if output.status.success() {
                    // Process still exists, need to kill it
                    println!("    Process {} still running, sending SIGTERM...", pid);
                    let _ = std::process::Command::new("kill")
                        .arg(pid.to_string())
                        .output();
                    
                    // Wait a bit for graceful shutdown
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    
                    // Check again
                    let check = std::process::Command::new("kill")
                        .arg("-0")
                        .arg(pid.to_string())
                        .output();
                    
                    if let Ok(output) = check {
                        if output.status.success() {
                            // Still running, force kill
                            println!("    Process {} still running, sending SIGKILL...", pid);
                            let _ = std::process::Command::new("kill")
                                .arg("-9")
                                .arg(pid.to_string())
                                .output();
                        }
                    }
                }
            }
        }
        
        println!("    ✓ {} stopped", service_name);
        Ok(())
    }

    /// Add a new service dynamically
    pub async fn add_service(&mut self, app: AppConfig) -> Result<()> {
        // Start the service
        let config = self.process_config_from_app(&app)?;
        self.process_manager.start(config).await?;

        // Update config.json for persistence
        self.update_config_add(app).await?;

        Ok(())
    }

    /// Remove a service
    pub async fn remove_service(&mut self, service_name: &str) -> Result<()> {
        // Stop the service and ensure it's killed
        self.stop_service_with_kill(service_name).await?;
        
        // Clean up from PMDaemon's tracking
        self.process_manager.delete(service_name).await?;

        // Update config.json
        self.update_config_remove(service_name).await?;

        Ok(())
    }

    /// Get status of all services
    pub async fn status(&self) -> Result<Vec<ServiceStatus>> {
        let processes = self.process_manager.list().await?;

        Ok(processes
            .into_iter()
            .map(|p| ServiceStatus {
                name: p.name,
                status: format!("{:?}", p.state),
                pid: p.pid,
                started_at: p
                    .uptime
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string()),
            })
            .collect())
    }

    /// Wait for a service to become healthy
    async fn wait_for_health(&self, service_name: &str, timeout: Duration) -> Result<()> {
        let start = std::time::Instant::now();

        while start.elapsed() < timeout {
            // Check if service is running
            let processes = self.process_manager.list().await?;

            if let Some(process) = processes.iter().find(|p| p.name == service_name) {
                if matches!(process.state, ProcessState::Online) {
                    // For validator, also check RPC endpoint
                    if service_name == "antegen-validator" {
                        if self.check_validator_health().await {
                            return Ok(());
                        }
                    } else {
                        return Ok(());
                    }
                }
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        bail!(
            "Service {} failed to become healthy within timeout",
            service_name
        )
    }

    /// Check if validator is healthy by testing RPC and WebSocket endpoints
    async fn check_validator_health(&self) -> bool {
        use solana_client::rpc_client::RpcClient;

        // Check RPC endpoint
        let client = RpcClient::new("http://localhost:8899");
        if !client.get_version().is_ok() {
            return false;
        }

        // Check WebSocket endpoint by trying to connect
        // We use a simple TCP check since full WebSocket handshake is complex
        use std::net::TcpStream;
        use std::time::Duration;
        
        match TcpStream::connect_timeout(
            &"127.0.0.1:8900".parse().unwrap(),
            Duration::from_secs(2)
        ) {
            Ok(_) => {
                // Both RPC and WebSocket ports are accessible
                true
            }
            Err(_) => {
                // WebSocket not ready yet
                false
            }
        }
    }

    /// Update config.json to add a service
    async fn update_config_add(&self, app: AppConfig) -> Result<()> {
        let mut config = if self.config_file.exists() {
            let content = std::fs::read_to_string(&self.config_file)?;
            serde_json::from_str(&content)?
        } else {
            ConfigFile { apps: Vec::new() }
        };

        // Remove existing app with same name if it exists
        config.apps.retain(|a| a.name != app.name);
        config.apps.push(app);

        std::fs::write(
            &self.config_file,
            serde_json::to_string_pretty(&config)?,
        )?;
        Ok(())
    }

    /// Update config.json to remove a service
    async fn update_config_remove(&self, service_name: &str) -> Result<()> {
        if self.config_file.exists() {
            let content = std::fs::read_to_string(&self.config_file)?;
            let mut config: ConfigFile = serde_json::from_str(&content)?;

            config.apps.retain(|a| a.name != service_name);

            std::fs::write(
                &self.config_file,
                serde_json::to_string_pretty(&config)?,
            )?;
        }
        Ok(())
    }

    /// Convert AppConfig to PMDaemon ProcessConfig
    fn process_config_from_app(&self, app: &AppConfig) -> Result<pmdaemon::ProcessConfig> {
        use pmdaemon::ProcessConfigBuilder;

        let mut builder = ProcessConfigBuilder::new()
            .name(&app.name)
            .script(&app.script);

        if let Some(args) = &app.args {
            builder = builder.args(args.clone());
        }

        if let Some(cwd) = &app.cwd {
            builder = builder.cwd(cwd);
        }

        if let Some(env) = &app.env {
            for (key, value) in env {
                builder = builder.env(key, value);
            }
        }

        // Note: PMDaemon 0.1.4 doesn't expose kill_timeout, auto_restart, or max_restarts
        // in the builder API. These features may need to be handled differently
        // or requested as enhancements to the library.

        Ok(builder.build()?)
    }

    /// Get runtime directory
    fn get_runtime_dir() -> PathBuf {
        dirs_next::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".antegen")
            .join("localnet")
    }
}

/// Service status information
#[derive(Debug, Clone)]
pub struct ServiceStatus {
    pub name: String,
    pub status: String,
    pub pid: Option<u32>,
    pub started_at: Option<String>,
}

/// Configuration file structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigFile {
    pub apps: Vec<AppConfig>,
}

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub name: String,
    pub script: String,
    pub args: Option<Vec<String>>,
    pub cwd: Option<String>,
    pub env: Option<std::collections::HashMap<String, String>>,
    pub auto_restart: Option<bool>,
    pub max_restarts: Option<u32>,
    pub restart_delay: Option<u64>,
    pub depends_on: Option<Vec<String>>,
    pub log_file: Option<String>,
    pub error_file: Option<String>,
}
