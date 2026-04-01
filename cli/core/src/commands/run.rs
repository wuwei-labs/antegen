//! Run command — delegates to the `antegen-node` binary
//!
//! This preserves backward compatibility: `antegen run` and service plists
//! using `antegen run` still work by exec'ing into `antegen-node`.

use anyhow::Result;
use std::path::PathBuf;

/// Execute the run command by delegating to `antegen-node`
pub async fn execute(
    config_path: PathBuf,
    rpc_override: Option<String>,
    log_level: Option<crate::LogLevel>,
    _version: Option<String>,
) -> Result<()> {
    let node_binary = match find_node_binary() {
        Ok(path) => path,
        Err(_) => {
            println!("No node binary found. Downloading latest...");
            match super::update::download_latest_node().await {
                Ok(()) => find_node_binary()?,
                Err(e) => {
                    anyhow::bail!(
                        "No node binary available: {}\n  \
                         Run `antegenctl install <version>` when a release is available.",
                        e
                    );
                }
            }
        }
    };

    let mut cmd = std::process::Command::new(&node_binary);
    cmd.arg("--config").arg(&config_path);

    if let Some(rpc) = rpc_override {
        cmd.arg("--rpc").arg(rpc);
    }

    if let Some(level) = log_level {
        let level_str = match level {
            crate::LogLevel::Trace => "trace",
            crate::LogLevel::Debug => "debug",
            crate::LogLevel::Info => "info",
            crate::LogLevel::Warn => "warn",
            crate::LogLevel::Error => "error",
            crate::LogLevel::Off => "off",
        };
        cmd.arg("--log-level").arg(level_str);
    }

    // On Unix, exec replaces the current process
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = cmd.exec();
        // exec() only returns on error
        anyhow::bail!("Failed to exec antegen-node: {}", err);
    }

    // On non-Unix, spawn and wait
    #[cfg(not(unix))]
    {
        let status = cmd.status().context("Failed to run antegen-node")?;
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }
        Ok(())
    }
}

/// Find the `antegen-node` binary.
/// Searches:
/// 1. Same directory as the current executable
/// 2. ~/.local/bin/
/// 3. PATH
fn find_node_binary() -> Result<PathBuf> {
    // Check same directory as current exe
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            let candidate = dir.join("antegen-node");
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }

    // Check ~/.local/bin/
    if let Some(home) = dirs::home_dir() {
        let candidate = home.join(".local/bin/antegen-node");
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    // Check cargo target directories (dev mode)
    #[cfg(not(feature = "prod"))]
    if let Ok(current_exe) = std::env::current_exe() {
        let path_str = current_exe.to_string_lossy();
        if path_str.contains("/target/debug/") {
            let candidate = current_exe.with_file_name("antegen-node");
            if candidate.exists() {
                return Ok(candidate);
            }
        } else if path_str.contains("/target/release/") {
            let candidate = current_exe.with_file_name("antegen-node");
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }

    anyhow::bail!(
        "antegen-node binary not found.\n\
         Install it with: antegenctl install <version>\n\
         Or build it with: cargo build -p antegen-client --features node"
    )
}
