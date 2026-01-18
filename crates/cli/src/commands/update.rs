//! Self-update command for the antegen CLI

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::fs::symlink;

/// GitHub repository owner
const REPO_OWNER: &str = "wuwei-labs";
/// GitHub repository name
const REPO_NAME: &str = "antegen";

/// Get the current CLI version
pub fn current_version() -> &'static str {
    concat!("v", env!("CARGO_PKG_VERSION"))
}

/// Parse a version string like "v4.3.2" into (major, minor, patch)
pub fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let v = v.strip_prefix('v').unwrap_or(v);
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    Some((
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ))
}

/// Compare two version strings, returns true if v1 < v2
pub fn version_less_than(v1: &str, v2: &str) -> bool {
    match (parse_version(v1), parse_version(v2)) {
        (Some(a), Some(b)) => a < b,
        _ => false, // If parsing fails, don't update
    }
}

/// Get the platform target string for the current system
pub fn get_platform_target() -> &'static str {
    self_update::get_target()
}

/// Get the binary symlink path
pub fn binary_path() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|p| p.join(".local/bin/antegen"))
        .context("Could not determine home directory")
}

/// Get the versioned binary path (e.g., ~/.local/bin/antegen-v4.3.1)
fn versioned_binary_path(version: &str) -> Result<PathBuf> {
    dirs::home_dir()
        .map(|p| p.join(format!(".local/bin/antegen-{}", version)))
        .context("Could not determine home directory")
}

/// Get the bin directory path
fn bin_dir() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|p| p.join(".local/bin"))
        .context("Could not determine home directory")
}

/// Shell script for ~/.antegen/env (rustup-style PATH setup)
const ENV_SCRIPT: &str = r#"# Antegen PATH setup - sourced by shell rc files
# Add ~/.local/bin to PATH if not already present
case ":${PATH}:" in
    *:"$HOME/.local/bin":*)
        ;;
    *)
        export PATH="$HOME/.local/bin:$PATH"
        ;;
esac
"#;

/// Ensure ~/.local/bin is in PATH using rustup-style env file approach
/// Creates ~/.antegen/env and sources it from the user's shell rc file
fn ensure_path_configured() {
    use std::io::Write;

    let Some(home) = dirs::home_dir() else {
        return;
    };

    let antegen_dir = home.join(".antegen");
    let env_file = antegen_dir.join("env");

    // Create ~/.antegen/env if it doesn't exist
    if !env_file.exists() {
        if fs::create_dir_all(&antegen_dir).is_err() {
            return;
        }
        if fs::write(&env_file, ENV_SCRIPT).is_err() {
            return;
        }
    }

    // Check if already in PATH - skip rc file modification if so
    if std::env::var("PATH")
        .map(|p| p.contains(".local/bin"))
        .unwrap_or(false)
    {
        return;
    }

    // Shell rc files in priority order (zshenv runs for all zsh, including non-interactive)
    let rc_files = [
        ".zshenv",
        ".zshrc",
        ".bashrc",
        ".bash_profile",
        ".profile",
    ];

    for rc_name in rc_files {
        let rc_path = home.join(rc_name);
        if !rc_path.exists() {
            continue;
        }

        // Check if already sourcing our env file
        if let Ok(content) = fs::read_to_string(&rc_path) {
            if content.contains(".antegen/env") {
                return; // Already configured
            }
        }

        // Append source line to this rc file
        if let Ok(mut file) = fs::OpenOptions::new().append(true).open(&rc_path) {
            if writeln!(file, "\n# Added by antegen\n. \"$HOME/.antegen/env\"").is_ok() {
                println!("Added antegen to PATH in {}", rc_name);
                println!("Run 'source ~/{}' or restart your shell to apply.", rc_name);
                return; // Only update one file
            }
        }
    }
}

/// Fetch the latest version from GitHub API using self_update
pub async fn fetch_latest_version() -> Result<String> {
    // self_update's fetch is blocking, so run in spawn_blocking
    tokio::task::spawn_blocking(|| {
        let releases = self_update::backends::github::ReleaseList::configure()
            .repo_owner(REPO_OWNER)
            .repo_name(REPO_NAME)
            .build()
            .context("Failed to build release list")?
            .fetch()
            .context("Failed to fetch releases from GitHub")?;

        releases
            .first()
            .map(|r| normalize_version(&r.version)) // Ensure "v" prefix
            .ok_or_else(|| anyhow::anyhow!("No releases found"))
    })
    .await
    .context("Task failed")?
}

/// Build the download URL for the CLI binary
pub fn build_download_url(version: &str) -> String {
    let target = get_platform_target();
    format!(
        "https://github.com/{}/{}/releases/download/{}/antegen-{}-{}",
        REPO_OWNER, REPO_NAME, version, version, target
    )
}

/// Download the binary to a temporary file
pub async fn download_binary(url: &str) -> Result<PathBuf> {
    use std::io::Write;

    println!("Downloading from: {}", url);

    let response = reqwest::get(url)
        .await
        .context("Failed to connect to GitHub releases")?;

    if !response.status().is_success() {
        if response.status().as_u16() == 404 {
            anyhow::bail!(
                "Binary not found. This may mean:\n\
                 - The version hasn't been released yet\n\
                 - Pre-built binaries aren't available for your platform ({})\n\
                 \n\
                 You can download manually from: https://github.com/{}/{}/releases",
                get_platform_target(),
                REPO_OWNER,
                REPO_NAME
            );
        }
        anyhow::bail!("Failed to download: HTTP {}", response.status());
    }

    let bytes = response
        .bytes()
        .await
        .context("Failed to read response body")?;

    // Write to temp file
    let temp_dir = std::env::temp_dir();
    let temp_path = temp_dir.join("antegen-update");

    let mut file = fs::File::create(&temp_path).context("Failed to create temp file")?;
    file.write_all(&bytes)
        .context("Failed to write temp file")?;

    println!("  Downloaded {} bytes", bytes.len());

    Ok(temp_path)
}

/// Get the installed binary version from the symlink target
fn get_installed_version() -> Option<String> {
    let symlink_path = binary_path().ok()?;
    if !symlink_path.is_symlink() {
        return None;
    }
    let target = std::fs::read_link(&symlink_path).ok()?;
    let filename = target.file_name()?.to_str()?;
    filename.strip_prefix("antegen-").map(|v| v.to_string())
}

/// Update the CLI binary to latest or a specific version
/// By default, restarts the service if running. Use --manual-restart to skip.
pub async fn update(version: Option<String>, manual_restart: bool) -> Result<()> {
    // Get installed version (from symlink), fall back to CLI version
    let installed = get_installed_version().unwrap_or_else(|| current_version().to_string());
    println!("Installed version: {}", installed);

    // Resolve target version
    let latest = match &version {
        Some(v) => normalize_version(v),
        None => {
            println!("Checking for updates...");
            fetch_latest_version().await?
        }
    };

    // Skip if already on this version (unless explicitly requested)
    if version.is_none() && !version_less_than(&installed, &latest) {
        println!("✓ Already up to date ({})", installed);
        return Ok(());
    }

    if version.is_some() {
        println!("Switching to version: {}", latest);
    } else {
        println!("New version available: {} -> {}", installed, latest);
    }

    // Download new binary to versioned path
    let url = build_download_url(&latest);
    let temp_path = download_binary(&url).await?;

    // Get paths
    let bin_dir = bin_dir()?;
    let symlink_path = binary_path()?;
    let new_versioned_path = versioned_binary_path(&latest)?;
    let old_versioned_path = versioned_binary_path(&installed)?;

    // Create bin directory if needed
    fs::create_dir_all(&bin_dir)?;

    // Install new versioned binary
    println!("Installing {} ...", new_versioned_path.display());
    fs::copy(&temp_path, &new_versioned_path).context("Failed to copy binary")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&new_versioned_path, fs::Permissions::from_mode(0o755))?;
    }

    // Clean up temp file
    let _ = fs::remove_file(&temp_path);

    // If current binary is a regular file (not symlink), migrate it to versioned path
    if symlink_path.exists() && !symlink_path.is_symlink() {
        println!("Migrating existing binary to versioned path...");
        fs::rename(&symlink_path, &old_versioned_path)
            .context("Failed to migrate existing binary")?;
    }

    // Atomically swap symlink to new version
    // Remove old symlink first (atomic_symlink not available in std)
    if symlink_path.exists() || symlink_path.is_symlink() {
        fs::remove_file(&symlink_path).context("Failed to remove old symlink")?;
    }

    #[cfg(unix)]
    symlink(&new_versioned_path, &symlink_path).context("Failed to create symlink")?;

    println!("✓ Updated to {}", latest);

    // Ensure PATH is configured for the user
    ensure_path_configured();

    // Check if service is running and handle restart
    if super::service::is_installed() {
        use service_manager::{ServiceManager, ServiceStatus, ServiceStatusCtx};
        if let Ok(manager) = <dyn ServiceManager>::native() {
            if let Ok(label) = "antegen".parse() {
                if let Ok(ServiceStatus::Running) = manager.status(ServiceStatusCtx { label }) {
                    if manual_restart {
                        // User opted out of auto-restart
                        println!();
                        println!("Note: Service is still running {}.", installed);
                        println!("Run `antegen restart` to update the service to {}.", latest);
                    } else {
                        // Auto-restart service with new version
                        println!();
                        println!("Restarting service...");
                        if let Err(e) = super::service::restart() {
                            println!("✗ Failed to restart service: {}", e);
                            println!("Run `antegen restart` manually to update the service.");
                        } else {
                            println!("✓ Service restarted with {}", latest);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Install the binary (called by install script)
/// Downloads if needed, sets up symlink, configures PATH
pub async fn install(version: Option<String>) -> Result<()> {
    let target_version = match &version {
        Some(v) => normalize_version(v),
        None => {
            println!("Fetching latest version...");
            fetch_latest_version().await?
        }
    };

    println!("Installing antegen {}...", target_version);
    ensure_binary_installed(Some(&target_version)).await?;

    println!("✓ Installed antegen {}", target_version);
    Ok(())
}

/// Check if we're running from a cargo target directory (dev mode)
/// Returns the path to the dev binary if in dev mode
#[cfg(not(feature = "prod"))]
fn get_dev_binary() -> Option<PathBuf> {
    let current_exe = std::env::current_exe().ok()?;
    let path_str = current_exe.to_string_lossy();

    // Check if running from cargo target directory
    if path_str.contains("/target/debug/") || path_str.contains("/target/release/") {
        Some(current_exe)
    } else {
        None
    }
}

/// Check if we're running in dev mode (from cargo target directory)
#[cfg(not(feature = "prod"))]
pub fn is_dev_build() -> bool {
    get_dev_binary().is_some()
}

/// Normalize version string (ensure v prefix)
fn normalize_version(version: &str) -> String {
    if version.starts_with('v') {
        version.to_string()
    } else {
        format!("v{}", version)
    }
}

/// Ensure the binary is installed at ~/.local/bin/antegen (as symlink to versioned binary)
/// - version: None = use dev build (if applicable) or latest release
/// - version: Some(v) = use that specific version (overrides dev mode)
pub async fn ensure_binary_installed(version: Option<&str>) -> Result<PathBuf> {
    let symlink_path = binary_path()?;
    let bin_dir = bin_dir()?;

    // Create bin directory if needed
    fs::create_dir_all(&bin_dir)?;

    // Dev mode: only if no version specified (not compiled in prod builds)
    #[cfg(not(feature = "prod"))]
    if version.is_none() {
        if let Some(dev_binary) = get_dev_binary() {
            let version = current_version();
            let versioned_path = versioned_binary_path(version)?;

            // Check if dev binary needs updating (version changed or symlink missing/wrong)
            let needs_update = !versioned_path.exists()
                || symlink_path
                    .read_link()
                    .ok()
                    .map(|target| target != versioned_path)
                    .unwrap_or(true);

            if needs_update {
                println!("Dev mode: installing {} ...", versioned_path.display());
                fs::copy(&dev_binary, &versioned_path).context("Failed to copy dev binary")?;

                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&versioned_path, fs::Permissions::from_mode(0o755))?;
                }

                // Update symlink
                if symlink_path.exists() || symlink_path.is_symlink() {
                    fs::remove_file(&symlink_path)?;
                }
                #[cfg(unix)]
                symlink(&versioned_path, &symlink_path).context("Failed to create symlink")?;

                println!("✓ Installed dev binary {}", version);
            }

            return Ok(symlink_path);
        }
    }

    // Resolve version: use specified version or fetch latest
    let version = match version {
        Some(v) => normalize_version(v),
        None => {
            // Production: use existing binary if installed and no version specified
            if symlink_path.exists() {
                return Ok(symlink_path);
            }
            println!("Binary not found at {}", symlink_path.display());
            println!("Downloading latest release...");
            fetch_latest_version().await?
        }
    };

    let versioned_path = versioned_binary_path(&version)?;

    // Download if not installed
    if !versioned_path.exists() {
        println!("Version {} not installed, downloading...", version);
        let url = build_download_url(&version);
        let temp_path = download_binary(&url).await?;

        println!("Installing {} ...", versioned_path.display());
        fs::copy(&temp_path, &versioned_path).context("Failed to copy binary")?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&versioned_path, fs::Permissions::from_mode(0o755))?;
        }

        let _ = fs::remove_file(&temp_path);
        println!("✓ Downloaded {}", version);
    }

    // Update symlink
    if symlink_path.exists() || symlink_path.is_symlink() {
        fs::remove_file(&symlink_path)?;
    }
    #[cfg(unix)]
    symlink(&versioned_path, &symlink_path).context("Failed to create symlink")?;

    println!("✓ Switched to {}", version);

    // Ensure PATH is configured for the user
    ensure_path_configured();

    Ok(symlink_path)
}
