//! Service management commands (init, start, stop, restart, uninstall, status)

use anyhow::{Context, Result};
use service_manager::{
    ServiceInstallCtx, ServiceLabel, ServiceLevel, ServiceManager, ServiceStartCtx, ServiceStatus,
    ServiceStatusCtx, ServiceStopCtx, ServiceUninstallCtx,
};
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;

/// Service label for antegen
const SERVICE_LABEL: &str = "antegen";

/// Get a user-level service manager
fn get_service_manager() -> Result<Box<dyn ServiceManager>> {
    let mut manager = <dyn ServiceManager>::native()
        .context("Failed to get native service manager for this platform")?;

    // Use user-level services (no sudo required)
    manager
        .set_level(ServiceLevel::User)
        .context("Service manager does not support user-level services")?;

    Ok(manager)
}

/// Get the service label
fn get_label() -> Result<ServiceLabel> {
    SERVICE_LABEL
        .parse()
        .context("Failed to parse service label")
}

/// Get the config directory path
fn config_dir() -> Result<PathBuf> {
    dirs::config_dir()
        .map(|p| p.join("antegen"))
        .context("Could not determine config directory")
}

/// Get the data directory path
fn data_dir() -> Result<PathBuf> {
    dirs::data_local_dir()
        .map(|p| p.join("antegen"))
        .context("Could not determine data directory")
}

/// Prompt user for RPC endpoint
/// Returns None if stdin is not interactive (non-TTY mode)
fn prompt_for_rpc() -> Result<Option<String>> {
    use std::io::IsTerminal;

    // Check if stdin is interactive
    if !std::io::stdin().is_terminal() {
        return Ok(None);
    }

    print!("Enter RPC endpoint URL [http://localhost:8899]: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let trimmed = input.trim();
    if trimmed.is_empty() {
        Ok(Some("http://localhost:8899".to_string()))
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

/// Core init logic - creates config only, no service installation
/// Returns the config path
fn do_init(rpc: Option<String>, force: bool) -> Result<PathBuf> {
    let config_dir = config_dir()?;
    let config_path = config_dir.join("antegen.toml");
    let data_dir = data_dir()?;

    // Check if already configured
    if config_path.exists() && !force {
        return Ok(config_path);
    }

    // Prompt for RPC if not provided
    let rpc_url = match rpc {
        Some(url) => url,
        None => match prompt_for_rpc()? {
            Some(url) => url,
            None => {
                anyhow::bail!(
                    "RPC endpoint required. Use --rpc flag in non-interactive mode:\n  \
                     antegen init --rpc <URL>\n  \
                     antegen start --rpc <URL>"
                );
            }
        },
    };

    // Create directories
    std::fs::create_dir_all(&config_dir)
        .with_context(|| format!("Failed to create config directory: {}", config_dir.display()))?;
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("Failed to create data directory: {}", data_dir.display()))?;

    // Generate keypair path in data directory
    let keypair_path = data_dir.join("executor.json");

    // Generate config using existing config init logic
    super::config::init(
        config_path.clone(),
        Some(rpc_url),
        Some(keypair_path.to_string_lossy().to_string()),
        Some(data_dir.join("observability").to_string_lossy().to_string()),
        force,
    )?;

    Ok(config_path)
}

/// Install the service (helper for start command)
async fn install_service(config_path: &PathBuf) -> Result<()> {
    let manager = get_service_manager()?;
    let label = get_label()?;

    // Ensure binary is installed, download if missing
    let binary = super::update::ensure_binary_installed().await?;

    manager
        .install(ServiceInstallCtx {
            label: label.clone(),
            program: binary.clone(),
            args: vec![
                OsString::from("run"),
                OsString::from("-c"),
                OsString::from(config_path.as_os_str()),
            ],
            contents: None,
            username: None,
            working_directory: None,
            environment: None,
            autostart: true,
            restart_policy: service_manager::RestartPolicy::OnFailure { delay_secs: Some(5) },
        })
        .context("Failed to install service")?;

    Ok(())
}

/// Start the service (helper)
fn start_service() -> Result<()> {
    let manager = get_service_manager()?;
    let label = get_label()?;

    manager
        .start(ServiceStartCtx { label })
        .context("Failed to start service")?;

    Ok(())
}

/// Initialize config only (no service installation)
pub fn init(rpc: Option<String>, force: bool) -> Result<()> {
    let config_path = do_init(rpc, force)?;
    println!("✓ Config created: {}", config_path.display());
    Ok(())
}

/// Ensure config exists, init if needed. Returns config path.
/// Used by `run` command when no config is specified.
pub fn ensure_config() -> Result<PathBuf> {
    do_init(None, false)
}

/// Start the antegen service (init + install + start)
pub async fn start(rpc: Option<String>) -> Result<()> {
    let config_path = do_init(rpc, false)?;

    println!("Installing service...");
    install_service(&config_path).await?;
    println!("✓ Service installed");

    println!("Starting service...");
    start_service()?;

    // Give the service a moment to start (or crash)
    std::thread::sleep(std::time::Duration::from_secs(2));

    // Check if it's actually running
    let manager = get_service_manager()?;
    let label = get_label()?;
    match manager.status(ServiceStatusCtx { label })? {
        ServiceStatus::Running => {
            println!("✓ Service started");
            println!();
            println!("Antegen is now running as a user service.");
            println!("Use 'antegen stop' to stop or 'antegen restart' to restart.");
        }
        ServiceStatus::Stopped(reason) => {
            println!("✗ Service started but crashed immediately");
            if let Some(msg) = reason {
                println!("  Reason: {}", msg);
            }
            println!();
            println!("Check the configuration and try 'antegen run' to see error output.");
        }
        ServiceStatus::NotInstalled => {
            println!("✗ Service failed to install");
        }
    }

    Ok(())
}

/// Show service status
pub fn status() -> Result<()> {
    let manager = get_service_manager()?;
    let label = get_label()?;

    match manager.status(ServiceStatusCtx { label })? {
        ServiceStatus::Running => {
            println!("✓ Service is running");
        }
        ServiceStatus::Stopped(reason) => {
            println!("✗ Service is stopped");
            if let Some(msg) = reason {
                println!("  Reason: {}", msg);
            }
        }
        ServiceStatus::NotInstalled => {
            println!("✗ Service is not installed");
            println!("  Run 'antegen start' to install and start the service.");
            return Ok(());
        }
    }

    // Print platform-specific command for detailed status
    #[cfg(target_os = "macos")]
    println!("\nFor detailed status: launchctl print gui/$(id -u)/{}", SERVICE_LABEL);

    #[cfg(target_os = "linux")]
    println!("\nFor logs: journalctl --user -u {} -f", SERVICE_LABEL);

    Ok(())
}

/// Stop the antegen service
pub fn stop() -> Result<()> {
    let manager = get_service_manager()?;
    let label = get_label()?;

    println!("Stopping antegen service...");

    manager
        .stop(ServiceStopCtx { label })
        .context("Failed to stop service")?;

    println!("✓ Service stopped");
    Ok(())
}

/// Restart the antegen service
pub fn restart() -> Result<()> {
    let manager = get_service_manager()?;
    let label = get_label()?;

    println!("Restarting antegen service...");

    // Stop first (ignore errors if not running)
    let _ = manager.stop(ServiceStopCtx {
        label: label.clone(),
    });

    // Start
    manager
        .start(ServiceStartCtx { label })
        .context("Failed to start service")?;

    println!("✓ Service restarted");
    Ok(())
}

/// Uninstall the antegen service
pub fn uninstall() -> Result<()> {
    let manager = get_service_manager()?;
    let label = get_label()?;

    println!("Uninstalling antegen service...");

    // Stop first (ignore errors if not running)
    let _ = manager.stop(ServiceStopCtx {
        label: label.clone(),
    });

    // Uninstall
    manager
        .uninstall(ServiceUninstallCtx { label })
        .context("Failed to uninstall service")?;

    println!("✓ Service uninstalled");
    println!();
    println!("Note: Config and data files are preserved in:");
    println!("  Config: {}", config_dir()?.display());
    println!("  Data: {}", data_dir()?.display());

    Ok(())
}

/// Check if the service is installed (for update command)
pub fn is_installed() -> bool {
    let Ok(manager) = get_service_manager() else {
        return false;
    };
    let Ok(label) = get_label() else {
        return false;
    };

    matches!(
        manager.status(ServiceStatusCtx { label }),
        Ok(ServiceStatus::Running) | Ok(ServiceStatus::Stopped(_))
    )
}
