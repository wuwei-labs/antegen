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

/// Update the CLI binary using symlink-based atomic updates with rollback
pub async fn update(no_restart: bool) -> Result<()> {
    let current = current_version();
    println!("Current version: {}", current);

    println!("Checking for updates...");
    let latest = fetch_latest_version().await?;

    if !version_less_than(current, &latest) {
        println!("✓ Already up to date ({})", current);
        return Ok(());
    }

    println!("New version available: {} -> {}", current, latest);

    // Download new binary to versioned path
    let url = build_download_url(&latest);
    let temp_path = download_binary(&url).await?;

    // Get paths
    let bin_dir = bin_dir()?;
    let symlink_path = binary_path()?;
    let new_versioned_path = versioned_binary_path(&latest)?;
    let old_versioned_path = versioned_binary_path(current)?;

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

    // Check if current binary is a symlink or regular file (migration case)
    let was_symlink = symlink_path.is_symlink();
    let old_target = if was_symlink {
        fs::read_link(&symlink_path).ok()
    } else {
        None
    };

    // If current binary is a regular file (not symlink), migrate it to versioned path
    if symlink_path.exists() && !was_symlink {
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

    // Auto-restart service if running and verify it works
    if !no_restart && super::service::is_installed() {
        println!("Restarting service...");

        if let Err(e) = super::service::restart() {
            println!("✗ Failed to restart service: {}", e);
            println!("Rolling back to previous version...");

            // Rollback: restore old symlink
            let _ = fs::remove_file(&symlink_path);

            #[cfg(unix)]
            if let Some(ref old_target) = old_target {
                symlink(old_target, &symlink_path)?;
            } else if old_versioned_path.exists() {
                symlink(&old_versioned_path, &symlink_path)?;
            }

            // Clean up failed new version
            let _ = fs::remove_file(&new_versioned_path);

            anyhow::bail!(
                "Update failed: service did not start with new version.\n\
                 Rolled back to {}",
                current
            );
        }

        // Give service time to stabilize
        std::thread::sleep(std::time::Duration::from_secs(2));

        // Verify service is still running
        use service_manager::{ServiceManager, ServiceStatus, ServiceStatusCtx};
        let manager = <dyn ServiceManager>::native()?;
        let label = "antegen".parse()?;
        let status = manager.status(ServiceStatusCtx { label })?;

        match status {
            ServiceStatus::Running => {
                println!("✓ Service running with new version");

                // Clean up old versioned binary
                if old_versioned_path.exists() && old_versioned_path != new_versioned_path {
                    let _ = fs::remove_file(&old_versioned_path);
                    println!("✓ Cleaned up old version");
                }
            }
            ServiceStatus::Stopped(reason) => {
                println!("✗ Service stopped after update");
                if let Some(msg) = reason {
                    println!("  Reason: {}", msg);
                }

                println!("Rolling back to previous version...");

                // Rollback
                let _ = fs::remove_file(&symlink_path);

                #[cfg(unix)]
                if let Some(ref old_target) = old_target {
                    symlink(old_target, &symlink_path)?;
                } else if old_versioned_path.exists() {
                    symlink(&old_versioned_path, &symlink_path)?;
                }

                // Restart with old version
                let _ = super::service::restart();

                // Clean up failed new version
                let _ = fs::remove_file(&new_versioned_path);

                anyhow::bail!(
                    "Update failed: service crashed with new version.\n\
                     Rolled back to {}",
                    current
                );
            }
            _ => {}
        }
    } else {
        // No service or --no-restart, just clean up old version
        if old_versioned_path.exists() && old_versioned_path != new_versioned_path {
            let _ = fs::remove_file(&old_versioned_path);
        }
    }

    Ok(())
}

/// Check if we're running from a cargo target directory (dev mode)
fn is_dev_build() -> Option<PathBuf> {
    let current_exe = std::env::current_exe().ok()?;
    let path_str = current_exe.to_string_lossy();

    // Check if running from cargo target directory
    if path_str.contains("/target/debug/") || path_str.contains("/target/release/") {
        Some(current_exe)
    } else {
        None
    }
}

/// Ensure the binary is installed at ~/.local/bin/antegen (as symlink to versioned binary)
/// In dev mode, copies the current binary. Otherwise downloads from GitHub.
pub async fn ensure_binary_installed() -> Result<PathBuf> {
    let symlink_path = binary_path()?;
    let bin_dir = bin_dir()?;

    // If symlink exists and points to valid binary, we're good
    if symlink_path.exists() {
        return Ok(symlink_path);
    }

    // Create bin directory if needed
    fs::create_dir_all(&bin_dir)?;

    // Check if we're running from cargo (dev mode)
    if let Some(dev_binary) = is_dev_build() {
        let version = current_version();
        let versioned_path = versioned_binary_path(version)?;

        println!("Dev mode: installing {} ...", versioned_path.display());
        fs::copy(&dev_binary, &versioned_path).context("Failed to copy dev binary")?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&versioned_path, fs::Permissions::from_mode(0o755))?;
        }

        // Create symlink
        #[cfg(unix)]
        symlink(&versioned_path, &symlink_path).context("Failed to create symlink")?;

        println!("✓ Installed dev binary {}", version);
        return Ok(symlink_path);
    }

    // Production mode: download from GitHub
    println!("Binary not found at {}", symlink_path.display());
    println!("Downloading latest release...");

    let version = fetch_latest_version().await?;
    println!("Latest version: {}", version);

    let versioned_path = versioned_binary_path(&version)?;

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

    // Create symlink
    #[cfg(unix)]
    symlink(&versioned_path, &symlink_path).context("Failed to create symlink")?;

    println!("✓ Installed antegen {}", version);

    Ok(symlink_path)
}
