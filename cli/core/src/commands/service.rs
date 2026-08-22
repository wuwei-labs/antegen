//! Service management commands (init, start, stop, restart, uninstall, status)

use anyhow::{Context, Result};
use service_manager::{
    ServiceInstallCtx, ServiceLabel, ServiceManager, ServiceStartCtx, ServiceStatus,
    ServiceStatusCtx, ServiceStopCtx, ServiceUninstallCtx,
};
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

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
                     antegen node init --rpc <URL>\n  \
                     antegen node start --rpc <URL>"
                );
            }
        },
    };

    // Create directories
    std::fs::create_dir_all(&config_dir).with_context(|| {
        format!(
            "Failed to create config directory: {}",
            config_dir.display()
        )
    })?;
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

/// Whether `--version` output came from a pre-consolidation `antegen-node`.
fn is_legacy_daemon(version_output: &str) -> bool {
    version_output.starts_with("antegen-node")
}

/// Arguments the service should pass to a daemon binary.
///
/// The daemon used to be a separate `antegen-node` binary that took `--config`
/// directly; it is now `antegen node run`. Both can be installed at once — an
/// operator rolling back to a pre-consolidation version still has its binary on
/// disk — so ask the binary which it is rather than guessing from a version
/// number. `antegen-node` identifies itself in `--version`; anything else is
/// assumed to be the consolidated CLI.
fn daemon_args(binary: &Path, config_path: &Path) -> Vec<OsString> {
    let legacy = std::process::Command::new(binary)
        .arg("--version")
        .output()
        .map(|o| is_legacy_daemon(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or(false);

    let mut args = Vec::new();
    if !legacy {
        args.push(OsString::from("node"));
        args.push(OsString::from("run"));
    }
    args.push(OsString::from("--config"));
    args.push(OsString::from(config_path.as_os_str()));
    args
}

/// Install the service (helper for start command).
/// Runs the versioned daemon binary, which is independent of the CLI on PATH.
async fn install_service(config_path: &Path, version: Option<&str>) -> Result<()> {
    let manager = get_service_manager()?;
    let label = get_label()?;

    // Resolve the node binary to use for the service
    let node_version = match version {
        Some(v) => v.to_string(),
        None => match super::update::read_node_version() {
            Some(v) => v,
            None => {
                // No node version tracked — download latest
                println!("No node binary found. Downloading latest...");
                match super::update::download_latest_node().await {
                    Ok(()) => super::update::read_node_version()
                        .context("Failed to determine node version after download")?,
                    Err(e) => {
                        anyhow::bail!(
                            "No node binary available: {}\n  \
                             Run `antegen node install <version>` when a release is available.",
                            e
                        );
                    }
                }
            }
        },
    };

    let binary = super::update::ensure_node_downloaded(&node_version).await?;
    let binary = binary.canonicalize().unwrap_or(binary);

    // Create logs directory
    let log_dir = dirs::data_local_dir()
        .map(|p| p.join("antegen").join("logs"))
        .context("Could not determine log directory")?;
    std::fs::create_dir_all(&log_dir)?;

    let args = daemon_args(&binary, config_path);

    #[cfg(target_os = "macos")]
    let contents = Some(generate_launchd_plist(
        &binary,
        &args,
        &log_dir.join("antegen.out"),
        &log_dir.join("antegen.log"),
    ));

    #[cfg(not(target_os = "macos"))]
    let contents = None;

    manager
        .install(ServiceInstallCtx {
            label: label.clone(),
            program: binary.clone(),
            args,
            contents,
            username: None,
            working_directory: None,
            environment: None,
            autostart: true,
            // Always, not OnFailure. A node has no reason to exit on its own —
            // an operator stopping it goes through the service manager, which
            // systemd distinguishes from the process exiting — so any exit at
            // all means something went wrong and it should come back.
            //
            // OnFailure keys off the exit code, which made it one bug away from
            // useless: the node shut itself down after an actor failure, exited
            // 0 because the teardown was tidy, and systemd read that as a
            // successful run and left it down. That exit code is fixed too, but
            // the restart policy should not depend on getting it right.
            restart_policy: service_manager::RestartPolicy::Always {
                delay_secs: Some(5),
            },
        })
        .context("Failed to install service")?;

    // Track the node version from the binary filename (e.g., antegen-node-v4.1.1)
    if let Some(filename) = binary.file_name().and_then(|f| f.to_str()) {
        if let Some(ver) = filename.strip_prefix("antegen-node-") {
            let _ = super::update::write_node_version(ver);
        }
    }

    Ok(())
}

/// Generate launchd plist with log file paths
#[cfg(target_os = "macos")]
fn generate_launchd_plist(
    binary: &std::path::Path,
    args: &[OsString],
    stdout_log: &std::path::Path,
    stderr_log: &std::path::Path,
) -> String {
    let program_arguments = std::iter::once(binary.to_string_lossy().into_owned())
        .chain(args.iter().map(|a| a.to_string_lossy().into_owned()))
        .map(|a| format!("        <string>{}</string>", a))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{}</string>
    <key>ProgramArguments</key>
    <array>
{}
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
        program_arguments,
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
/// If the service is already installed, stops and uninstalls it first (clean reinstall).
pub async fn start(rpc: Option<String>, version: Option<String>) -> Result<()> {
    super::update::clean_legacy_layout();
    let config_path = do_init(rpc, false)?;

    // Clean reinstall: stop + uninstall existing service if present
    if is_installed() {
        let manager = get_service_manager()?;
        let label = get_label()?;
        let _ = manager.stop(ServiceStopCtx {
            label: label.clone(),
        });
        let _ = manager.uninstall(ServiceUninstallCtx { label });
    }

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
            println!("Use `antegen node stop` to stop or `antegen node restart` to restart.");

            // Check for updates
            print_update_notices().await;
        }
        // Report these as failures. `antegen node update` reinstalls the service
        // to move the daemon onto a new binary; exiting 0 here would report a
        // successful update while the node was not running at all.
        ServiceStatus::Stopped(reason) => {
            anyhow::bail!(
                "Service started but crashed immediately{}\n  \
                 Run `antegen node run` to see the error output, or \
                 `antegen node logs` for the service log.",
                reason
                    .map(|m| format!("\n  Reason: {}", m))
                    .unwrap_or_default()
            );
        }
        ServiceStatus::NotInstalled => {
            anyhow::bail!("Service failed to install");
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
            println!("  Run `antegenctl start` to install and start the service.");
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
            println!("Is the service running? Use 'antegen node start' to start it.");
            return Ok(());
        }

        let mut cmd = std::process::Command::new("tail");
        if follow {
            cmd.arg("-n").arg("50").arg("-f");
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

/// How to update the CLI itself. The CLI no longer manages its own versions —
/// the installer owns `~/.local/bin/antegen`, so re-running it is the update.
pub const INSTALL_HINT: &str =
    "Re-run: curl -sSfL https://raw.githubusercontent.com/wuwei-labs/antegen/main/scripts/install.sh | bash";

/// Print update notices for CLI and node if newer versions are available
async fn print_update_notices() {
    #[cfg(not(feature = "prod"))]
    if super::update::is_dev_build() {
        return;
    }

    let (cli_update, node_update) = tokio::join!(
        async {
            let installed = super::update::current_version();
            let latest = super::update::fetch_latest_version_cached().await.ok()?;
            if super::update::version_less_than(installed, &latest) {
                Some(latest)
            } else {
                None
            }
        },
        async {
            let installed = super::update::read_node_version()?;
            let latest = super::update::fetch_latest_version_cached().await.ok()?;
            if super::update::version_less_than(&installed, &latest) {
                Some(latest)
            } else {
                None
            }
        },
    );

    if cli_update.is_some() || node_update.is_some() {
        println!();
    }
    if let Some(latest) = cli_update {
        println!("CLI update available: {} -> {}", latest, INSTALL_HINT);
    }
    if let Some(latest) = node_update {
        println!(
            "Node update available: {} -> Run `antegen node update`",
            latest
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact `--version` output of both daemons, verified against a
    /// downloaded `antegen-node-v5.1.3` and a locally built `antegen`. The
    /// service unit's argv depends on telling them apart, and a wrong answer
    /// produces a unit that cannot start.
    #[test]
    fn distinguishes_legacy_daemon_from_consolidated_cli() {
        assert!(is_legacy_daemon("antegen-node 5.1.3\n"));
        assert!(!is_legacy_daemon("antegen 6.1.0 (client 5.2.0)\n"));
    }

    /// Exercises the real `daemon_args` against stub binaries that identify
    /// themselves the way each daemon does.
    #[test]
    fn argv_shape_follows_the_binary() {
        let dir = tempfile::tempdir().unwrap();
        let config = Path::new("/tmp/antegen.toml");

        let stub = |name: &str, version_line: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, format!("#!/bin/sh\necho '{}'\n", version_line)).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            path
        };

        let legacy = stub("antegen-node-v5.1.3", "antegen-node 5.1.3");
        assert_eq!(
            daemon_args(&legacy, config),
            vec![
                OsString::from("--config"),
                OsString::from(config.as_os_str())
            ]
        );

        let consolidated = stub("antegen-node-v7.0.0", "antegen 7.0.0 (client 6.0.0)");
        assert_eq!(
            daemon_args(&consolidated, config),
            vec![
                OsString::from("node"),
                OsString::from("run"),
                OsString::from("--config"),
                OsString::from(config.as_os_str())
            ]
        );
    }

    /// A binary we cannot execute must not be assumed legacy — new installs are
    /// the common case, and the old argv against a new binary fails to start.
    #[test]
    fn unreadable_binary_assumes_consolidated() {
        let args = daemon_args(Path::new("/nonexistent/antegen"), Path::new("/tmp/c.toml"));
        assert_eq!(args[0], OsString::from("node"));
    }
}
