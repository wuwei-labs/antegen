//! Download module for fetching the Geyser plugin from GitHub releases

use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::Path;

/// GitHub repository for releases
const GITHUB_REPO: &str = "wuwei-labs/antegen";

/// Get the current CLI version (used to match plugin version)
pub fn current_version() -> &'static str {
    concat!("v", env!("CARGO_PKG_VERSION"))
}

/// Get the platform target string for the current system
fn get_platform_target() -> &'static str {
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
    compile_error!("Unsupported platform for geyser plugin download");
}

/// Get the platform-specific library extension
pub fn get_library_extension() -> &'static str {
    #[cfg(target_os = "linux")]
    return "so";

    #[cfg(target_os = "macos")]
    return "dylib";

    #[cfg(target_os = "windows")]
    return "dll";

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    compile_error!("Unsupported platform");
}

/// Get the platform-specific library filename
pub fn get_library_filename() -> String {
    let ext = get_library_extension();
    format!("libantegen_geyser_plugin.{}", ext)
}

/// Build the download URL for the geyser plugin
fn build_download_url(version: &str) -> String {
    let target = get_platform_target();
    // Note: Release artifacts use .so extension even for macOS dylib
    format!(
        "https://github.com/{}/releases/download/{}/antegen-geyser-{}-{}.so",
        GITHUB_REPO, version, version, target
    )
}

/// Download the geyser plugin from GitHub releases
pub async fn download_geyser_plugin(version: &str, dest: &Path) -> Result<()> {
    let url = build_download_url(version);

    log::info!("Downloading geyser plugin from: {}", url);
    println!("Downloading geyser plugin...");
    println!("  URL: {}", url);

    // Create parent directory if needed
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    // Download the file
    let response = reqwest::get(&url)
        .await
        .context("Failed to connect to GitHub releases")?;

    if !response.status().is_success() {
        if response.status().as_u16() == 404 {
            return Err(anyhow::anyhow!(
                "Plugin not found for version {} on platform {}.\n\
                 This may mean:\n\
                 - The version hasn't been released yet\n\
                 - Pre-built binaries aren't available for your platform\n\
                 \n\
                 You can download manually from: https://github.com/{}/releases",
                version,
                get_platform_target(),
                GITHUB_REPO
            ));
        }
        return Err(anyhow::anyhow!(
            "Failed to download plugin: HTTP {}",
            response.status()
        ));
    }

    let bytes = response
        .bytes()
        .await
        .context("Failed to read response body")?;

    // Write to destination
    let mut file = fs::File::create(dest).context("Failed to create plugin file")?;
    file.write_all(&bytes)
        .context("Failed to write plugin file")?;

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(dest)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(dest, perms)?;
    }

    println!("  Downloaded {} bytes", bytes.len());

    Ok(())
}

/// Check if the plugin needs to be updated (version mismatch)
pub fn needs_update(plugin_path: &Path, expected_version: &str) -> Result<bool> {
    // For now, just check if the file exists
    // In the future, we could embed version info in the plugin or use a manifest
    if !plugin_path.exists() {
        return Ok(true);
    }

    // Check for version file alongside the plugin
    let version_file = plugin_path.with_extension("version");
    if version_file.exists() {
        let stored_version = fs::read_to_string(&version_file)?;
        if stored_version.trim() != expected_version {
            log::info!(
                "Plugin version mismatch: {} vs {}",
                stored_version.trim(),
                expected_version
            );
            return Ok(true);
        }
    } else {
        // No version file, assume update needed
        return Ok(true);
    }

    Ok(false)
}

/// Save the version information alongside the plugin
pub fn save_version_info(plugin_path: &Path, version: &str) -> Result<()> {
    let version_file = plugin_path.with_extension("version");
    fs::write(&version_file, version)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_current_version() {
        let version = current_version();
        assert!(version.starts_with('v'));
    }

    #[test]
    fn test_build_download_url() {
        let url = build_download_url("v3.0.0");
        assert!(url.contains("github.com"));
        assert!(url.contains("v3.0.0"));
        assert!(url.contains("antegen-geyser"));
    }

    #[test]
    fn test_get_library_filename() {
        let filename = get_library_filename();
        assert!(filename.starts_with("libantegen_geyser_plugin"));
    }
}
