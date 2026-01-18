//! Info command - Show antegen configuration and status

use antegen_client::ClientConfig;
use anyhow::Result;
use serde::Serialize;
use solana_sdk::native_token::LAMPORTS_PER_SOL;
use solana_sdk::signature::{read_keypair_file, Signer};
use std::path::PathBuf;

/// Info output structure for JSON serialization
#[derive(Serialize)]
pub struct InfoOutput {
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance_sol: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpc: Option<String>,
    pub service: String,
    pub observability: ObservabilityInfo,
    pub data: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_available: Option<String>,
}

/// Observability info
#[derive(Serialize)]
pub struct ObservabilityInfo {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_page: Option<String>,
}

/// Get the default config path
fn default_config_path() -> Result<PathBuf> {
    dirs::config_dir()
        .map(|p| p.join("antegen").join("antegen.toml"))
        .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))
}

/// Get the default data directory path
fn default_data_dir() -> Result<PathBuf> {
    dirs::data_local_dir()
        .map(|p| p.join("antegen"))
        .ok_or_else(|| anyhow::anyhow!("Could not determine data directory"))
}

/// Shorten path by replacing home directory with ~
fn shorten_path(path: &std::path::Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(relative) = path.strip_prefix(&home) {
            return format!("~/{}", relative.display());
        }
    }
    path.display().to_string()
}

/// Get the installed binary version from the symlink target
fn get_installed_binary_version() -> Option<String> {
    let binary_path = super::update::binary_path().ok()?;

    // Check if symlink exists
    if !binary_path.is_symlink() {
        return None;
    }

    // Read the symlink target (e.g., antegen-v4.4.0)
    let target = std::fs::read_link(&binary_path).ok()?;
    let filename = target.file_name()?.to_str()?;

    // Extract version from filename like "antegen-v4.4.0"
    filename.strip_prefix("antegen-").map(|v| v.to_string())
}

/// Gather all info
async fn gather_info() -> Result<InfoOutput> {
    let version = env!("CARGO_PKG_VERSION").to_string();
    let data_dir = default_data_dir()?;
    let config_path = default_config_path()?;

    // Service status and version (only get version if running)
    let (service, service_version) = if super::service::is_installed() {
        use service_manager::{ServiceManager, ServiceStatus, ServiceStatusCtx};
        let manager = <dyn ServiceManager>::native()?;
        let label = "antegen".parse()?;
        match manager.status(ServiceStatusCtx { label })? {
            ServiceStatus::Running => {
                let ver = get_installed_binary_version();
                ("running".to_string(), ver)
            }
            ServiceStatus::Stopped(_) => ("stopped".to_string(), None),
            ServiceStatus::NotInstalled => ("not installed".to_string(), None),
        }
    } else {
        ("not installed".to_string(), None)
    };

    // Check for updates (non-blocking, fail silently)
    let update_available = check_update_available(&service_version).await;

    // If config doesn't exist, return minimal info
    if !config_path.exists() {
        return Ok(InfoOutput {
            version,
            service_version,
            executor: None,
            balance_sol: None,
            rpc: None,
            service,
            observability: ObservabilityInfo {
                enabled: false,
                name: None,
                status_page: None,
            },
            data: shorten_path(&data_dir),
            update_available,
        });
    }

    // Load config
    let config = ClientConfig::load(&config_path)?;

    // Get executor pubkey
    let keypair_path = super::expand_tilde(&config.executor.keypair_path)?;
    let executor = if keypair_path.exists() {
        read_keypair_file(&keypair_path)
            .ok()
            .map(|kp| kp.pubkey().to_string())
    } else {
        None
    };

    // Get balance
    let balance_sol = if let Some(ref _executor) = executor {
        get_balance(&config).await.ok()
    } else {
        None
    };

    // Get RPC URL
    let rpc = config.rpc.endpoints.first().map(|e| e.url.clone());

    // Get observability info
    let observability = get_observability_info(&config);

    Ok(InfoOutput {
        version,
        service_version,
        executor,
        balance_sol,
        rpc,
        service,
        observability,
        data: shorten_path(&data_dir),
        update_available,
    })
}

/// Get executor balance
async fn get_balance(config: &ClientConfig) -> Result<f64> {
    let keypair_path = super::expand_tilde(&config.executor.keypair_path)?;
    let keypair = read_keypair_file(&keypair_path)
        .map_err(|e| anyhow::anyhow!("Failed to read keypair: {}", e))?;
    let pubkey = keypair.pubkey();

    let rpc_url = config
        .rpc
        .endpoints
        .first()
        .ok_or_else(|| anyhow::anyhow!("No RPC endpoints configured"))?;

    let client = antegen_client::rpc::RpcPool::with_url(&rpc_url.url)?;
    let balance = client.get_balance(&pubkey).await?;

    Ok(balance as f64 / LAMPORTS_PER_SOL as f64)
}

/// Get observability info from LOA
fn get_observability_info(config: &ClientConfig) -> ObservabilityInfo {
    if !config.observability.enabled {
        return ObservabilityInfo {
            enabled: false,
            name: None,
            status_page: None,
        };
    }

    // Try to read LOA agent info
    let storage_path = match super::expand_tilde(&config.observability.storage_path) {
        Ok(p) => p,
        Err(_) => {
            return ObservabilityInfo {
                enabled: true,
                name: None,
                status_page: None,
            }
        }
    };

    // Use loa-core AgentInfo API
    match loa_core::AgentInfo::read(&storage_path) {
        Ok(info) => ObservabilityInfo {
            enabled: true,
            name: info.name.clone(),
            status_page: Some(info.dashboard_url.clone()),
        },
        Err(_) => ObservabilityInfo {
            enabled: true,
            name: None,
            status_page: None,
        },
    }
}

/// Check if update is available (compares service/installed version to latest)
async fn check_update_available(service_version: &Option<String>) -> Option<String> {
    // Skip in dev mode - dev always uses local build
    #[cfg(not(feature = "prod"))]
    if super::update::is_dev_build() {
        return None;
    }

    // Get the version that's running (service version if available, otherwise installed binary)
    let running_version = match service_version {
        Some(v) => v.clone(),
        None => get_installed_binary_version()?,
    };

    let latest = super::update::fetch_latest_version().await.ok()?;

    // Only show update if latest is actually newer than running version
    if version_less_than(&running_version, &latest) {
        Some(latest)
    } else {
        None
    }
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
        _ => false,
    }
}

/// Print info in human-readable format
fn print_info(info: &InfoOutput) {
    // Show version header: "Antegen -- 4.5.0 (service 4.4.0)" or "Antegen -- 4.5.0"
    if let Some(service_ver) = &info.service_version {
        // Strip 'v' prefix from service version if present
        let svc_ver = service_ver.strip_prefix('v').unwrap_or(service_ver);
        println!("Antegen -- {} (service {})", info.version, svc_ver);
    } else {
        println!("Antegen -- {}", info.version);
    }
    println!();

    // Check if we have config
    if info.executor.is_none() && info.rpc.is_none() {
        println!("Config not found. Run `antegen init` to get started.");
        return;
    }

    if let Some(executor) = &info.executor {
        println!("{:14} {}", "executor:", executor);
    }
    if let Some(balance) = info.balance_sol {
        println!("{:14} {:.4} SOL", "balance:", balance);
    }
    if let Some(rpc) = &info.rpc {
        println!("{:14} {}", "rpc:", rpc);
    }
    println!("{:14} {}", "service:", info.service);

    if info.observability.enabled {
        println!("{:14} enabled", "observability:");
        if let Some(status_page) = &info.observability.status_page {
            println!("{:14} {}", "status page:", status_page);
        }
    } else {
        println!("{:14} disabled", "observability:");
    }

    println!("{:14} {}", "data:", info.data);

    if let Some(update) = &info.update_available {
        println!();
        println!("Update available: {} -> Run `antegen update`", update);
    }
}

/// Execute the info command
pub async fn info(json: bool) -> Result<()> {
    let info = gather_info().await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&info)?);
    } else {
        print_info(&info);
    }

    Ok(())
}
