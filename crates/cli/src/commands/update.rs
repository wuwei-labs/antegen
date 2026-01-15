//! Self-update command for the antegen CLI

use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

/// GitHub repository for releases
const GITHUB_REPO: &str = "wuwei-labs/antegen";

/// Get the current CLI version
pub fn current_version() -> &'static str {
    concat!("v", env!("CARGO_PKG_VERSION"))
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

/// Get the binary install path
pub fn binary_path() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|p| p.join(".local/bin/antegen"))
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

/// Update the CLI binary
pub async fn update(no_restart: bool) -> Result<()> {
    let current = current_version();
    println!("Current version: {}", current);

    println!("Checking for updates...");
    let latest = fetch_latest_version().await?;

    if current == latest {
        println!("✓ Already up to date ({})", current);
        return Ok(());
    }

    println!("New version available: {} -> {}", current, latest);

    // Download new binary
    let url = build_download_url(&latest);
    let temp_path = download_binary(&url).await?;

    // Get install path
    let bin_path = binary_path()?;

    // Create parent directory if needed
    if let Some(parent) = bin_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Replace binary
    println!("Installing to: {}", bin_path.display());
    fs::copy(&temp_path, &bin_path).context("Failed to copy binary")?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bin_path, fs::Permissions::from_mode(0o755))?;
    }

    // Clean up temp file
    let _ = fs::remove_file(&temp_path);

    println!("✓ Updated to {}", latest);

    // Auto-restart service if running
    if !no_restart && super::service::is_installed() {
        println!("Restarting service...");
        if let Err(e) = super::service::restart() {
            println!("Warning: Failed to restart service: {}", e);
            println!("You may need to run 'antegen restart' manually.");
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

/// Ensure the binary is installed at ~/.local/bin/antegen
/// In dev mode, copies the current binary. Otherwise downloads from GitHub.
pub async fn ensure_binary_installed() -> Result<PathBuf> {
    let bin_path = binary_path()?;

    if bin_path.exists() {
        return Ok(bin_path);
    }

    // Create parent directory if needed
    if let Some(parent) = bin_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Check if we're running from cargo (dev mode)
    if let Some(dev_binary) = is_dev_build() {
        println!("Dev mode: copying current binary to {}", bin_path.display());
        fs::copy(&dev_binary, &bin_path).context("Failed to copy dev binary")?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&bin_path, fs::Permissions::from_mode(0o755))?;
        }

        println!("✓ Installed dev binary");
        return Ok(bin_path);
    }

    // Production mode: download from GitHub
    println!("Binary not found at {}", bin_path.display());
    println!("Downloading latest release...");

    let version = fetch_latest_version().await?;
    println!("Latest version: {}", version);

    let url = build_download_url(&version);
    let temp_path = download_binary(&url).await?;

    println!("Installing to: {}", bin_path.display());
    fs::copy(&temp_path, &bin_path).context("Failed to copy binary")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bin_path, fs::Permissions::from_mode(0o755))?;
    }

    let _ = fs::remove_file(&temp_path);

    println!("✓ Installed antegen {}", version);

    Ok(bin_path)
}
