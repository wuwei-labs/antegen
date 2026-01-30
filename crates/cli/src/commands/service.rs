//! Service management commands (init, start, stop, restart, uninstall, status)

use anyhow::{Context, Result};
use service_manager::{
    ServiceInstallCtx, ServiceLabel, ServiceManager, ServiceStartCtx, ServiceStatus,
    ServiceStatusCtx, ServiceStopCtx, ServiceUninstallCtx,
};
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;

/// Service label for antegen
const SERVICE_LABEL: &str = "antegen";

/// Check if running as root user (Linux only)
#[cfg(target_os = "linux")]
fn is_root() -> bool {
    unsafe { libc::getuid() == 0 }
}

/// Get a service manager (user-level by default, system-level for root on Linux)
fn get_service_manager() -> Result<Box<dyn ServiceManager>> {
    #[cfg(target_os = "macos")]
    {
        use service_manager::LaunchdServiceManager;
        Ok(Box::new(LaunchdServiceManager::user()))
    }

    #[cfg(target_os = "linux")]
    {
        use service_manager::SystemdServiceManager;
        if is_root() {
            Ok(Box::new(SystemdServiceManager::system()))
        } else {
            Ok(Box::new(SystemdServiceManager::user()))
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        anyhow::bail!("Service management is not supported on this platform")
    }
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
async fn install_service(config_path: &PathBuf, version: Option<&str>) -> Result<()> {
    let manager = get_service_manager()?;
    let label = get_label()?;

    // Ensure binary is installed, download if missing
    let binary = super::update::ensure_binary_installed(version).await?;

    // Create logs directory
    let log_dir = dirs::data_local_dir()
        .map(|p| p.join("antegen").join("logs"))
        .context("Could not determine log directory")?;
    std::fs::create_dir_all(&log_dir)?;

    // Generate platform-specific service config with log paths
    // Note: Rust logger writes to stderr, so stderr gets the main log file
    #[cfg(target_os = "macos")]
    let contents = Some(generate_launchd_plist(
        &binary,
        config_path,
        &log_dir.join("antegen.out"),
        &log_dir.join("antegen.log"),
    ));

    #[cfg(not(target_os = "macos"))]
    let contents = None;

    manager
        .install(ServiceInstallCtx {
            label: label.clone(),
            program: binary.clone(),
            args: vec![
                OsString::from("run"),
                OsString::from("-c"),
                OsString::from(config_path.as_os_str()),
            ],
            contents,
            username: None,
            working_directory: None,
            environment: None,
            autostart: true,
            restart_policy: service_manager::RestartPolicy::OnFailure { delay_secs: Some(5) },
        })
        .context("Failed to install service")?;

    Ok(())
}

/// Generate launchd plist with log file paths
#[cfg(target_os = "macos")]
fn generate_launchd_plist(
    binary: &std::path::Path,
    config_path: &std::path::Path,
    stdout_log: &std::path::Path,
    stderr_log: &std::path::Path,
) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>run</string>
        <string>-c</string>
        <string>{}</string>
    </array>
    <key>StandardOutPath</key>
    <string>{}</string>
    <key>StandardErrorPath</key>
    <string>{}</string>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>RunAtLoad</key>
    <true/>
    <key>ThrottleInterval</key>
    <integer>5</integer>
</dict>
</plist>"#,
        SERVICE_LABEL,
        binary.display(),
        config_path.display(),
        stdout_log.display(),
        stderr_log.display(),
    )
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
pub async fn start(rpc: Option<String>, version: Option<String>) -> Result<()> {
    let config_path = do_init(rpc, false)?;

    println!("Installing service...");
    install_service(&config_path, version.as_deref()).await?;
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

            // Check for updates
            if let Some(latest) = check_update_available().await {
                println!();
                println!("Update available: {} -> Run `antegen update`", latest);
            }
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

    // Print platform-specific log location
    #[cfg(target_os = "macos")]
    {
        if let Some(log_dir) = dirs::data_local_dir().map(|p| p.join("antegen").join("logs")) {
            println!("\nLogs: tail -f \"{}/antegen.log\"", log_dir.display());
        }
    }

    #[cfg(target_os = "linux")]
    println!("\nLogs: journalctl --user -u {} -f", SERVICE_LABEL);

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

/// Get the log file path (macOS only)
#[cfg(target_os = "macos")]
fn get_log_path() -> Result<PathBuf> {
    dirs::data_local_dir()
        .map(|p| p.join("antegen").join("logs").join("antegen.log"))
        .context("Could not determine log directory")
}

/// View service logs
pub fn logs(follow: bool) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let log_file = get_log_path()?;
        if !log_file.exists() {
            println!("No log file found at: {}", log_file.display());
            println!("Is the service running? Use 'antegen start' to start it.");
            return Ok(());
        }

        let mut cmd = std::process::Command::new("tail");
        if follow {
            cmd.arg("-f");
        } else {
            cmd.arg("-n").arg("100");
        }
        cmd.arg(&log_file);
        cmd.status().context("Failed to run tail")?;
    }

    #[cfg(target_os = "linux")]
    {
        // Linux uses journalctl for systemd user services - no log file check needed
        let mut cmd = std::process::Command::new("journalctl");
        if is_root() {
            // System service (root)
            cmd.arg("-u").arg(SERVICE_LABEL);
        } else {
            // User service
            cmd.arg("--user").arg("-u").arg(SERVICE_LABEL);
        }
        if follow {
            cmd.arg("-f");
        } else {
            cmd.arg("-n").arg("100");
        }
        cmd.status().context("Failed to run journalctl")?;
    }

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

/// Check if an update is available (compares installed binary to latest release)
async fn check_update_available() -> Option<String> {
    // Skip in dev mode - dev always uses local build
    #[cfg(not(feature = "prod"))]
    if super::update::is_dev_build() {
        return None;
    }

    let binary_path = super::update::binary_path().ok()?;
    if !binary_path.is_symlink() {
        return None;
    }

    let target = std::fs::read_link(&binary_path).ok()?;
    let filename = target.file_name()?.to_str()?;
    let installed = filename.strip_prefix("antegen-")?;

    let latest = super::update::fetch_latest_version().await.ok()?;

    if super::update::version_less_than(installed, &latest) {
        Some(latest)
    } else {
        None
    }
}
