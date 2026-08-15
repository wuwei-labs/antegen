//! Self-update command for the antegen CLI and node version management

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::fs::symlink;

/// GitHub repository owner
const REPO_OWNER: &str = "wuwei-labs";
/// GitHub repository name
const REPO_NAME: &str = "antegen";

/// Minimum supported node version — older releases were built from a separate
/// `antegen-node` crate that no longer exists.
const MIN_NODE_VERSION: &str = "v4.1.1";

/// Returns true if the given node version is >= MIN_NODE_VERSION.
fn is_node_version_supported(version: &str) -> bool {
    match (parse_version(version), parse_version(MIN_NODE_VERSION)) {
        (Some(v), Some(min)) => v >= min,
        _ => false,
    }
}

// =============================================================================
// Shared helpers
// =============================================================================

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
        _ => false,
    }
}

/// Get the platform target string for the current system
pub fn get_platform_target() -> &'static str {
    self_update::get_target()
}

/// Get the bin directory path
fn bin_dir() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|p| p.join(".local/bin"))
        .context("Could not determine home directory")
}

/// Normalize version string (ensure v prefix)
fn normalize_version(version: &str) -> String {
    if version.starts_with('v') {
        version.to_string()
    } else {
        format!("v{}", version)
    }
}

/// Download a binary to a temporary file
pub async fn download_binary(url: &str, temp_name: &str) -> Result<PathBuf> {
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

    let temp_dir = std::env::temp_dir();
    let temp_path = temp_dir.join(temp_name);

    let mut file = fs::File::create(&temp_path).context("Failed to create temp file")?;
    file.write_all(&bytes)
        .context("Failed to write temp file")?;

    println!("  Downloaded {} bytes", bytes.len());

    Ok(temp_path)
}

/// Install a downloaded binary to a versioned path with executable permissions
fn install_binary_to(temp_path: &PathBuf, dest_path: &PathBuf) -> Result<()> {
    let bin_dir = bin_dir()?;
    fs::create_dir_all(&bin_dir)?;

    println!("Installing {} ...", dest_path.display());
    fs::copy(temp_path, dest_path).context("Failed to copy binary")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dest_path, fs::Permissions::from_mode(0o755))?;
    }

    let _ = fs::remove_file(temp_path);
    Ok(())
}

/// Update a symlink to point to a new target
#[cfg(unix)]
fn update_symlink(symlink_path: &PathBuf, target: &PathBuf) -> Result<()> {
    if symlink_path.exists() || symlink_path.is_symlink() {
        fs::remove_file(symlink_path)
            .with_context(|| format!("Failed to remove old symlink: {}", symlink_path.display()))?;
    }
    symlink(target, symlink_path)
        .with_context(|| format!("Failed to create symlink: {}", symlink_path.display()))?;
    Ok(())
}

/// Fetch the latest CLI version from GitHub API using self_update
pub async fn fetch_latest_version() -> Result<String> {
    tokio::task::spawn_blocking(|| {
        let releases = self_update::backends::github::ReleaseList::configure()
            .repo_owner(REPO_OWNER)
            .repo_name(REPO_NAME)
            .build()
            .context("Failed to build release list")?
            .fetch()
            .context("Failed to fetch releases from GitHub")?;

        // CLI releases use tags like "v5.0.0" (no prefix)
        releases
            .iter()
            .find(|r| !r.version.starts_with("geyser-") && !r.version.starts_with("node-"))
            .map(|r| normalize_version(&r.version))
            .ok_or_else(|| anyhow::anyhow!("No CLI releases found"))
    })
    .await
    .context("Task failed")?
}

/// Fetch the latest node version from GitHub releases
pub async fn fetch_latest_node_version() -> Result<String> {
    tokio::task::spawn_blocking(|| {
        let releases = self_update::backends::github::ReleaseList::configure()
            .repo_owner(REPO_OWNER)
            .repo_name(REPO_NAME)
            .build()
            .context("Failed to build release list")?
            .fetch()
            .context("Failed to fetch releases from GitHub")?;

        // Node releases use tags like "node-v4.1.1"
        releases
            .iter()
            .find(|r| r.version.starts_with("node-v") || r.version.starts_with("node-"))
            .map(|r| {
                let v = r.version.strip_prefix("node-").unwrap_or(&r.version);
                normalize_version(v)
            })
            .ok_or_else(|| anyhow::anyhow!("No node releases found on GitHub yet"))
    })
    .await
    .context("Task failed")?
}

/// Fetch all available node versions from GitHub
pub async fn fetch_all_node_versions() -> Result<Vec<String>> {
    tokio::task::spawn_blocking(|| {
        let releases = self_update::backends::github::ReleaseList::configure()
            .repo_owner(REPO_OWNER)
            .repo_name(REPO_NAME)
            .build()
            .context("Failed to build release list")?
            .fetch()
            .context("Failed to fetch releases from GitHub")?;

        Ok(releases
            .iter()
            .filter(|r| r.version.starts_with("node-v") || r.version.starts_with("node-"))
            .map(|r| {
                let v = r.version.strip_prefix("node-").unwrap_or(&r.version);
                normalize_version(v)
            })
            .collect())
    })
    .await
    .context("Task failed")?
}

// =============================================================================
// Dev-build detection
// =============================================================================

/// Check if we're running from a cargo target directory (dev mode)
#[cfg(not(feature = "prod"))]
fn get_dev_binary() -> Option<PathBuf> {
    let current_exe = std::env::current_exe().ok()?;
    let path_str = current_exe.to_string_lossy();
    if path_str.contains("/target/debug/") || path_str.contains("/target/release/") {
        Some(current_exe)
    } else {
        None
    }
}

#[cfg(not(feature = "prod"))]
pub fn is_dev_build() -> bool {
    get_dev_binary().is_some()
}

// =============================================================================
// Node binary paths and management
// =============================================================================

/// Get the node binary symlink path (~/.local/bin/antegen-node)
pub fn node_binary_path() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|p| p.join(".local/bin/antegen-node"))
        .context("Could not determine home directory")
}

/// Get the versioned node binary path (e.g., ~/.local/bin/antegen-node-v4.1.1)
fn versioned_node_binary_path(version: &str) -> Result<PathBuf> {
    dirs::home_dir()
        .map(|p| p.join(format!(".local/bin/antegen-node-{}", version)))
        .context("Could not determine home directory")
}

/// Build the download URL for the node binary
pub fn build_node_download_url(version: &str) -> String {
    let target = get_platform_target();
    format!(
        "https://github.com/{}/{}/releases/download/node-{}/antegen-node-{}-{}",
        REPO_OWNER, REPO_NAME, version, version, target
    )
}

/// Path to the node version tracking file
fn node_version_path() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|p| p.join(".antegen/node-version"))
        .context("Could not determine home directory")
}

/// Write the active node version to the tracking file
pub fn write_node_version(version: &str) -> Result<()> {
    let path = node_version_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, version)?;
    Ok(())
}

/// Read the active node version from the tracking file
pub fn read_node_version() -> Option<String> {
    node_version_path()
        .ok()
        .and_then(|p| fs::read_to_string(&p).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Get the installed node version from the symlink target
fn get_installed_node_version() -> Option<String> {
    let symlink_path = node_binary_path().ok()?;
    if !symlink_path.is_symlink() {
        return None;
    }
    let target = std::fs::read_link(&symlink_path).ok()?;
    let filename = target.file_name()?.to_str()?;
    filename
        .strip_prefix("antegen-node-")
        .map(|v| v.to_string())
}

/// Ensure a specific node version is downloaded. Returns the versioned path.
pub async fn ensure_node_downloaded(version: &str) -> Result<PathBuf> {
    let version = normalize_version(version);
    let versioned_path = versioned_node_binary_path(&version)?;

    // Dev mode: look for antegen-node in target directory
    #[cfg(not(feature = "prod"))]
    {
        if let Ok(current_exe) = std::env::current_exe() {
            let path_str = current_exe.to_string_lossy();
            if path_str.contains("/target/debug/") || path_str.contains("/target/release/") {
                let dev_node = current_exe.with_file_name("antegen-node");
                if dev_node.exists() {
                    println!("Dev mode: using {} ...", dev_node.display());
                    let bin_dir = bin_dir()?;
                    fs::create_dir_all(&bin_dir)?;
                    fs::copy(&dev_node, &versioned_path)
                        .context("Failed to copy dev node binary")?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(&versioned_path, fs::Permissions::from_mode(0o755))?;
                    }
                    return Ok(versioned_path);
                }
            }
        }
    }

    if !versioned_path.exists() {
        println!("Node version {} not installed, downloading...", version);
        let url = build_node_download_url(&version);
        let temp_path = download_binary(&url, "antegen-node-update").await?;
        install_binary_to(&temp_path, &versioned_path)?;
        println!("Downloaded node {}", version);
    }

    Ok(versioned_path)
}

/// Detect a locally-built antegen-node binary in the cargo workspace.
///
/// Walks up from CWD to find the workspace root, checks for
/// `target/release/antegen-node`, and extracts its version.
///
/// Returns `Some((binary_path, version))` if found, `None` otherwise.
fn detect_local_node_build() -> Option<(PathBuf, String)> {
    let workspace_root = find_workspace_root().ok()?;
    let built_binary = workspace_root.join("target/release/antegen-node");
    if !built_binary.exists() {
        return None;
    }

    let output = std::process::Command::new(&built_binary)
        .arg("--version")
        .output()
        .ok()?;
    let version_output = String::from_utf8_lossy(&output.stdout);
    let version = version_output
        .split_whitespace()
        .nth(1)
        .map(normalize_version)?;

    Some((built_binary, version))
}

/// Build antegen-node from the local workspace and install to ~/.local/bin/
///
/// Returns the version string of the built binary.
pub fn cargo_build_and_install_node() -> Result<String> {
    let workspace_root = find_workspace_root()?;

    println!("Building antegen-node from {}...", workspace_root.display());

    let status = std::process::Command::new("cargo")
        .args([
            "build",
            "--release",
            "-p",
            "antegen-client",
            "--features",
            "node",
        ])
        .current_dir(&workspace_root)
        .status()
        .context("Failed to run cargo build")?;

    if !status.success() {
        anyhow::bail!("cargo build failed with exit code: {}", status);
    }

    let built_binary = workspace_root.join("target/release/antegen-node");
    if !built_binary.exists() {
        anyhow::bail!("Expected binary not found at {}.", built_binary.display());
    }

    let output = std::process::Command::new(&built_binary)
        .arg("--version")
        .output()
        .context("Failed to run built binary with --version")?;

    let version_output = String::from_utf8_lossy(&output.stdout);
    let version = version_output
        .split_whitespace()
        .nth(1)
        .map(normalize_version)
        .context("Failed to parse version from built binary")?;

    // Copy to versioned path (don't use install_binary_to — it deletes the source)
    let versioned_path = versioned_node_binary_path(&version)?;
    let dest_dir = bin_dir()?;
    fs::create_dir_all(&dest_dir)?;
    fs::copy(&built_binary, &versioned_path)
        .context("Failed to copy built binary to install directory")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&versioned_path, fs::Permissions::from_mode(0o755))?;
    }

    println!("Built and installed node {} from local source.", version);
    Ok(version)
}

/// Find the cargo workspace root by walking up from CWD
fn find_workspace_root() -> Result<PathBuf> {
    let mut dir = std::env::current_dir().context("Failed to get current directory")?;
    loop {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.exists() {
            let contents = fs::read_to_string(&cargo_toml)?;
            if contents.contains("[workspace]") {
                return Ok(dir);
            }
        }
        if !dir.pop() {
            anyhow::bail!(
                "Could not find workspace Cargo.toml. Run this command from within the antegen repository."
            );
        }
    }
}

/// Update node to latest or a specific version (for `antegen node update`).
/// Downloads the node binary, updates the `antegen-node` symlink, writes node-version.
/// Does NOT touch the interactive CLI at ~/.local/bin/antegen.
pub async fn update_node(version: Option<String>, local: bool) -> Result<()> {
    if local {
        let version = cargo_build_and_install_node()?;
        let versioned_path = versioned_node_binary_path(&version)?;

        #[cfg(unix)]
        {
            let symlink_path = node_binary_path()?;
            update_symlink(&symlink_path, &versioned_path)?;
        }

        write_node_version(&version)?;

        if super::service::is_installed() {
            println!("Restarting service with local node {}...", version);
            super::service::start(None, Some(version)).await?;
        } else {
            println!("Updated node to {} (local build)", version);
        }

        return Ok(());
    }

    let installed = get_installed_node_version()
        .or_else(read_node_version)
        .unwrap_or_else(|| "none".to_string());
    println!("Installed node version: {}", installed);

    let latest = match &version {
        Some(v) => normalize_version(v),
        None => {
            println!("Checking for node updates...");
            match fetch_latest_node_version().await {
                Ok(v) => v,
                Err(_) => {
                    println!("No node-specific releases found, checking CLI releases...");
                    fetch_latest_version().await?
                }
            }
        }
    };

    if !is_node_version_supported(&latest) {
        anyhow::bail!(
            "Node {} is not supported. Minimum version is {}.",
            latest,
            MIN_NODE_VERSION
        );
    }

    if version.is_none() && installed != "none" && !version_less_than(&installed, &latest) {
        println!("Already up to date ({})", installed);
        return Ok(());
    }

    if version.is_some() {
        println!("Switching node to version: {}", latest);
    } else {
        println!("New node version available: {} -> {}", installed, latest);
    }

    let versioned_path = ensure_node_downloaded(&latest).await?;

    #[cfg(unix)]
    {
        let symlink_path = node_binary_path()?;
        update_symlink(&symlink_path, &versioned_path)?;
    }

    write_node_version(&latest)?;

    if super::service::is_installed() {
        println!("Restarting service with node {}...", latest);
        super::service::start(None, Some(latest.clone())).await?;
    } else {
        println!("Updated node to {}", latest);
    }

    Ok(())
}

/// Switch node to a specific version (for `antegen node use <version>`).
/// Downloads if needed, updates symlink, writes node-version, reinstalls service.
/// Does NOT touch CLI symlinks.
pub async fn use_node_version(version: String) -> Result<()> {
    // Handle "local" keyword — copy workspace build into version manager
    if version == "local" {
        let (built_binary, ver) = detect_local_node_build()
            .context("No local build found. Run `cargo build -p antegen-client --release --features node` first.")?;

        let versioned_path = versioned_node_binary_path(&ver)?;
        let dest_dir = bin_dir()?;
        fs::create_dir_all(&dest_dir)?;
        fs::copy(&built_binary, &versioned_path)
            .context("Failed to copy local build to install directory")?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&versioned_path, fs::Permissions::from_mode(0o755))?;
        }

        #[cfg(unix)]
        {
            let symlink_path = node_binary_path()?;
            update_symlink(&symlink_path, &versioned_path)?;
        }

        write_node_version(&ver)?;

        if super::service::is_installed() {
            println!("Switching service to local node {}...", ver);
            super::service::start(None, Some(ver)).await?;
        } else {
            println!("Node switched to {} (local build)", ver);
        }

        return Ok(());
    }

    let version = normalize_version(&version);

    if !is_node_version_supported(&version) {
        anyhow::bail!(
            "Node {} is not supported. Minimum version is {}.",
            version,
            MIN_NODE_VERSION
        );
    }

    let versioned_path = ensure_node_downloaded(&version).await?;

    // Update antegen-node symlink
    #[cfg(unix)]
    {
        let symlink_path = node_binary_path()?;
        update_symlink(&symlink_path, &versioned_path)?;
    }

    write_node_version(&version)?;

    if super::service::is_installed() {
        println!("Switching service to node {}...", version);
        super::service::start(None, Some(version.clone())).await?;
    } else {
        println!("Node switched to {}", version);
        println!("Run `antegen node start` to start the service.");
    }

    Ok(())
}

/// Download a specific node version without switching (for `antegen node install <version>`)
pub async fn install_node_version(version: Option<String>, local: bool) -> Result<()> {
    if local {
        let version = cargo_build_and_install_node()?;
        println!("Use `antegen node use {}` to switch.", version);
        return Ok(());
    }

    let version = normalize_version(&version.context("version required (or use --local)")?);

    if !is_node_version_supported(&version) {
        anyhow::bail!(
            "Node {} is not supported. Minimum version is {}.",
            version,
            MIN_NODE_VERSION
        );
    }

    let versioned_path = versioned_node_binary_path(&version)?;

    if versioned_path.exists() {
        println!("Node {} is already installed.", version);
        return Ok(());
    }

    let url = build_node_download_url(&version);
    let temp_path = download_binary(&url, "antegen-node-update").await?;
    install_binary_to(&temp_path, &versioned_path)?;

    println!(
        "Downloaded node {}. Use `antegen node use {}` to switch.",
        version, version
    );
    Ok(())
}

// =============================================================================
// List (shows both CLI and node versions)
// =============================================================================

/// Download the latest supported node binary and set it as active.
/// Used by `antegen init` for out-of-box readiness.
pub async fn download_latest_node() -> Result<()> {
    let latest = fetch_latest_node_version().await?;

    if !is_node_version_supported(&latest) {
        anyhow::bail!(
            "No supported node version available yet (minimum {})",
            MIN_NODE_VERSION
        );
    }

    let versioned_path = ensure_node_downloaded(&latest).await?;

    #[cfg(unix)]
    {
        let symlink_path = node_binary_path()?;
        update_symlink(&symlink_path, &versioned_path)?;
    }

    write_node_version(&latest)?;
    println!("Installed node {}", latest);
    Ok(())
}

/// List node versions (for `antegen node list`)
/// Shows installed versions, local cargo build (if detected), and available remote versions.
pub async fn list_node() -> Result<()> {
    let bin_dir = bin_dir()?;
    let active_version = get_installed_node_version().or_else(read_node_version);

    // Detect local cargo build
    let local_build = detect_local_node_build();

    // Collect locally installed versions (>= MIN_NODE_VERSION only)
    let mut installed: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(&bin_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(ver) = name.strip_prefix("antegen-node-") {
                if ver.starts_with('v') && is_node_version_supported(ver) {
                    installed.push(ver.to_string());
                }
            }
        }
    }

    installed.sort_by(|a, b| match (parse_version(a), parse_version(b)) {
        (Some(va), Some(vb)) => vb.cmp(&va),
        _ => b.cmp(a),
    });

    // Fetch remote versions (>= MIN_NODE_VERSION only)
    let remote: Vec<String> = match fetch_all_node_versions().await {
        Ok(versions) => versions
            .into_iter()
            .filter(|v| is_node_version_supported(v))
            .collect(),
        Err(_) => Vec::new(),
    };

    println!("Installed:");
    if installed.is_empty() && local_build.is_none() {
        println!("  (none)");
    } else {
        for ver in &installed {
            if active_version.as_deref() == Some(ver.as_str()) {
                println!("  {} (active)", ver);
            } else {
                println!("  {}", ver);
            }
        }
    }

    // Show local cargo build if detected
    if let Some((path, ver)) = &local_build {
        let already_installed =
            installed.contains(ver) && active_version.as_deref() == Some(ver.as_str());
        if !already_installed {
            println!();
            println!("Local build:");
            println!("  {} ({})", ver, path.display());
            println!("  Use `antegen node use local` to switch.");
        }
    }

    println!();
    println!("Available:");
    let available: Vec<&String> = remote.iter().filter(|v| !installed.contains(v)).collect();
    if available.is_empty() {
        println!("  (all versions installed)");
    } else {
        for ver in available {
            println!("  {}", ver);
        }
    }

    Ok(())
}
