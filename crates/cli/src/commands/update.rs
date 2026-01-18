//! Self-update command for the antegen CLI

use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::fs::symlink;

/// GitHub repository for releases
const GITHUB_REPO: &str = "wuwei-labs/antegen";

/// Get the current CLI version
pub fn current_version() -> &'static str {
    concat!("v", env!("CARGO_PKG_VERSION"))
}

/// Parse a version string like "v4.3.2" into (major, minor, patch)
fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
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
fn version_less_than(v1: &str, v2: &str) -> bool {
    match (parse_version(v1), parse_version(v2)) {
        (Some(a), Some(b)) => a < b,
        _ => false, // If parsing fails, don't update
    }
}

/// Get the platform target string for the current system
pub fn get_platform_target() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return "x86_64-unknown-linux-gnu";

    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return "aarch64-unknown-linux-gnu";

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return "x86_64-apple-darwin";

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return "aarch64-apple-darwin";

    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
    )))]
    compile_error!("Unsupported platform for update");
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

/// Fetch the latest version from GitHub API
pub async fn fetch_latest_version() -> Result<String> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        GITHUB_REPO
    );

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .header("User-Agent", "antegen-cli")
        .send()
        .await
        .context("Failed to connect to GitHub API")?;

    if !response.status().is_success() {
        anyhow::bail!("Failed to fetch latest version: HTTP {}", response.status());
    }

    let json: serde_json::Value = response.json().await?;
    let tag = json["tag_name"]
        .as_str()
        .context("No tag_name in release response")?;

    Ok(tag.to_string())
}

/// Build the download URL for the CLI binary
pub fn build_download_url(version: &str) -> String {
    let target = get_platform_target();
    format!(
        "https://github.com/{}/releases/download/{}/antegen-{}-{}",
        GITHUB_REPO, version, version, target
    )
}

/// Download the binary to a temporary file
pub async fn download_binary(url: &str) -> Result<PathBuf> {
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
                 You can download manually from: https://github.com/{}/releases",
                get_platform_target(),
                GITHUB_REPO
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

    Ok(symlink_path)
}
