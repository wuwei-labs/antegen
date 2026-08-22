//! Node version management: which daemon build is installed, and switching
//! between them.
//!
//! The CLI does not update itself — re-running the install script does that.
//! What lives here is the versioned daemon binaries under `~/.local/bin`, the
//! `antegen-node` symlink the service points at, and the release lookup that
//! decides what is available.

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
pub(crate) fn current_version() -> &'static str {
    concat!("v", env!("CARGO_PKG_VERSION"))
}

/// Parse a version string like "v4.3.2" into (major, minor, patch)
pub(crate) fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
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
pub(crate) fn version_less_than(v1: &str, v2: &str) -> bool {
    match (parse_version(v1), parse_version(v2)) {
        (Some(a), Some(b)) => a < b,
        _ => false,
    }
}

/// Get the platform target string for the current system
pub(crate) fn get_platform_target() -> &'static str {
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
pub(crate) async fn download_binary(url: &str, temp_name: &str) -> Result<PathBuf> {
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

// =============================================================================
// Release lookup
//
// The CLI and the daemon are the same artifact, published on the
// `antegen-cli-v<x>` tag, so one lookup answers both "is my CLI current" and
// "is my node current". Results are cached on disk: the version check runs on
// `antegen node status` and `antegen info`, and GitHub allows 60 unauthenticated
// requests an hour.
// =============================================================================

/// Release tag prefix carrying the `antegen` binary.
const RELEASE_TAG_PREFIX: &str = "antegen-cli-v";

/// How long a fetched release list stays fresh.
const RELEASE_CACHE_TTL_SECS: u64 = 24 * 60 * 60;

#[derive(serde::Serialize, serde::Deserialize)]
struct ReleaseCache {
    fetched_at: u64,
    versions: Vec<String>,
}

fn release_cache_path() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|p| p.join(".antegen/release-cache.json"))
        .context("Could not determine home directory")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn read_release_cache() -> Option<Vec<String>> {
    let cache: ReleaseCache =
        serde_json::from_str(&fs::read_to_string(release_cache_path().ok()?).ok()?).ok()?;
    (now_secs().saturating_sub(cache.fetched_at) < RELEASE_CACHE_TTL_SECS).then_some(cache.versions)
}

fn write_release_cache(versions: &[String]) {
    let Ok(path) = release_cache_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(&ReleaseCache {
        fetched_at: now_secs(),
        versions: versions.to_vec(),
    }) {
        let _ = fs::write(path, json);
    }
}

/// Released `antegen` versions, newest first, always fetched live.
///
/// Used by anything the operator asked for directly — listing, updating,
/// installing. Those must be authoritative: answering `antegen node update`
/// from a cache is how the CLI goes blind to a release that already exists,
/// which is the exact failure the `node-v*` tag lookup used to produce.
///
/// One page of the releases API is enough — a version old enough to have fallen
/// off it is far below `MIN_NODE_VERSION` anyway.
pub(crate) async fn fetch_all_versions() -> Result<Vec<String>> {
    fetch_all_versions_inner(false).await
}

/// Released versions, allowed to come from a cache up to `RELEASE_CACHE_TTL_SECS`
/// old.
///
/// Only for the passive "an update is available" notice, which runs on every
/// `antegen node status` and `antegen info`. That check hitting the GitHub API
/// each time is what the cache exists to prevent; being a few hours stale costs
/// nothing, because nothing acts on it.
pub(crate) async fn fetch_all_versions_cached() -> Result<Vec<String>> {
    fetch_all_versions_inner(true).await
}

async fn fetch_all_versions_inner(use_cache: bool) -> Result<Vec<String>> {
    if use_cache {
        if let Some(cached) = read_release_cache() {
            return Ok(cached);
        }
    }

    let url = format!(
        "https://api.github.com/repos/{}/{}/releases?per_page=100",
        REPO_OWNER, REPO_NAME
    );

    let releases: Vec<serde_json::Value> = reqwest::Client::new()
        .get(&url)
        .header("User-Agent", "antegen-cli")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("Failed to reach the GitHub releases API")?
        .error_for_status()
        .context("GitHub releases API returned an error")?
        .json()
        .await
        .context("Failed to parse the GitHub releases response")?;

    let versions: Vec<String> = releases
        .iter()
        .filter_map(|r| r.get("tag_name")?.as_str())
        .filter_map(|tag| tag.strip_prefix(RELEASE_TAG_PREFIX))
        .map(normalize_version)
        .collect();

    write_release_cache(&versions);
    Ok(versions)
}

/// Latest released `antegen` version, fetched live.
pub(crate) async fn fetch_latest_version() -> Result<String> {
    fetch_all_versions()
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("No {}* releases found on GitHub yet", RELEASE_TAG_PREFIX))
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
pub(crate) fn is_dev_build() -> bool {
    get_dev_binary().is_some()
}

/// Latest released version for the passive update notice.
pub(crate) async fn fetch_latest_version_cached() -> Result<String> {
    fetch_all_versions_cached()
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("No {}* releases found on GitHub yet", RELEASE_TAG_PREFIX))
}

// =============================================================================
// Node binary paths and management
// =============================================================================

/// Remove artefacts of the pre-consolidation layout.
///
/// `antegenctl` was never a real binary on a scripted install — it was a
/// symlink into `~/.local/bin` pointing at the `antegen` binary — so leaving it
/// behind means `antegenctl <cmd>` keeps resolving to whatever version it was
/// last pointed at, silently running an old CLI. Only symlinks we created are
/// removed; a real file there belongs to someone else.
///
/// Idempotent, and never fatal: failing to tidy up must not stop an update.
pub(crate) fn clean_legacy_layout() {
    #[cfg(unix)]
    {
        let Some(home) = dirs::home_dir() else {
            return;
        };
        let stale = home.join(".local/bin/antegenctl");
        if !stale.is_symlink() {
            return;
        }
        let points_into_bin = fs::read_link(&stale)
            .map(|t| t.starts_with(home.join(".local/bin")))
            .unwrap_or(false);
        if points_into_bin && fs::remove_file(&stale).is_ok() {
            println!("Removed the deprecated antegenctl symlink; use `antegen node` instead.");
        }
    }
}

/// Get the node binary symlink path (~/.local/bin/antegen-node)
pub(crate) fn node_binary_path() -> Result<PathBuf> {
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
pub(crate) fn build_node_download_url(version: &str) -> String {
    let target = get_platform_target();
    format!(
        "https://github.com/{}/{}/releases/download/{}{}/antegen-{}-{}",
        REPO_OWNER,
        REPO_NAME,
        RELEASE_TAG_PREFIX,
        version.trim_start_matches('v'),
        version,
        target
    )
}

/// Download URL for a pre-consolidation daemon, published as its own binary on
/// a `node-v*` tag. Kept so an operator can still roll back to a version that
/// predates the merge into `antegen`.
fn build_legacy_node_download_url(version: &str) -> String {
    let target = get_platform_target();
    format!(
        "https://github.com/{}/{}/releases/download/node-{}/antegen-node-{}-{}",
        REPO_OWNER, REPO_NAME, version, version, target
    )
}

/// Download a daemon binary, falling back to the pre-consolidation layout.
async fn download_node_binary(version: &str) -> Result<PathBuf> {
    let url = build_node_download_url(version);
    match download_binary(&url, "antegen-node-update").await {
        Ok(path) => Ok(path),
        Err(err) => {
            let legacy = build_legacy_node_download_url(version);
            println!("Not published as {}; trying {}", url, legacy);
            download_binary(&legacy, "antegen-node-update")
                .await
                .map_err(|legacy_err| {
                    anyhow::anyhow!(
                        "Could not download node {version}.\n  \
                         {url}\n    {err}\n  \
                         {legacy}\n    {legacy_err}"
                    )
                })
        }
    }
}

/// Path to the node version tracking file
fn node_version_path() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|p| p.join(".antegen/node-version"))
        .context("Could not determine home directory")
}

/// Write the active node version to the tracking file
pub(crate) fn write_node_version(version: &str) -> Result<()> {
    let path = node_version_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, version)?;
    Ok(())
}

/// Read the active node version from the tracking file
pub(crate) fn read_node_version() -> Option<String> {
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
pub(crate) async fn ensure_node_downloaded(version: &str) -> Result<PathBuf> {
    let version = normalize_version(version);
    let versioned_path = versioned_node_binary_path(&version)?;

    // Dev mode: the daemon is this binary, so a cargo build is the node build
    #[cfg(not(feature = "prod"))]
    {
        if let Ok(current_exe) = std::env::current_exe() {
            let path_str = current_exe.to_string_lossy();
            if path_str.contains("/target/debug/") || path_str.contains("/target/release/") {
                let dev_node = current_exe.clone();
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
        let temp_path = download_node_binary(&version).await?;
        install_binary_to(&temp_path, &versioned_path)?;
        println!("Downloaded node {}", version);
    }

    Ok(versioned_path)
}

/// Detect a locally-built `antegen` binary in the cargo workspace.
///
/// Walks up from CWD to find the workspace root, checks for
/// `target/release/antegen`, and extracts its version.
///
/// Returns `Some((binary_path, version))` if found, `None` otherwise.
fn detect_local_node_build() -> Option<(PathBuf, String)> {
    let workspace_root = find_workspace_root().ok()?;
    let built_binary = workspace_root.join("target/release/antegen");
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

/// Build `antegen` from the local workspace and install it as a node version
///
/// Returns the version string of the built binary.
pub(crate) fn cargo_build_and_install_node() -> Result<String> {
    let workspace_root = find_workspace_root()?;

    println!("Building antegen from {}...", workspace_root.display());

    let status = std::process::Command::new("cargo")
        .args(["build", "--release", "-p", "antegen-cli"])
        .current_dir(&workspace_root)
        .status()
        .context("Failed to run cargo build")?;

    if !status.success() {
        anyhow::bail!("cargo build failed with exit code: {}", status);
    }

    let built_binary = workspace_root.join("target/release/antegen");
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
pub(crate) async fn update_node(version: Option<String>, local: bool) -> Result<()> {
    clean_legacy_layout();
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
            match fetch_latest_version().await {
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
pub(crate) async fn use_node_version(version: String) -> Result<()> {
    clean_legacy_layout();
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
pub(crate) async fn install_node_version(version: Option<String>, local: bool) -> Result<()> {
    clean_legacy_layout();
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

    let temp_path = download_node_binary(&version).await?;
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
pub(crate) async fn download_latest_node() -> Result<()> {
    let latest = fetch_latest_version().await?;

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
pub(crate) async fn list_node() -> Result<()> {
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
    let remote: Vec<String> = match fetch_all_versions().await {
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
