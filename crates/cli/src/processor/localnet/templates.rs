use std::collections::HashMap;
use std::path::PathBuf;
use super::daemon::AppConfig;
use antegen_sdk::ID as THREAD_PROGRAM_ID;
use solana_sdk::signature::{read_keypair_file, Signer};

/// Create validator service configuration
///
/// To enable Geyser plugin integration:
/// 1. Pass a geyser_config pointing to the Antegen Geyser plugin configuration
/// 2. The plugin will automatically start processing events through the Geyser interface
/// 3. The Geyser client uses pre-built datasources from antegen_client crate
pub fn validator_service(
    geyser_config: Option<PathBuf>,
    runtime_dir: &PathBuf,
    is_dev: bool,
) -> AppConfig {
    let mut args = vec![
        "--reset".to_string(),
        "--ledger".to_string(),
        runtime_dir.join("test-ledger").to_string_lossy().to_string(),
        "--rpc-port".to_string(),
        "8899".to_string(),
        "--faucet-port".to_string(),
        "9900".to_string(),
        "--log".to_string(),
    ];

    // Add thread program if available
    let program_path = if is_dev {
        // Get absolute path for dev mode
        std::env::current_dir()
            .unwrap_or_default()
            .join("target/deploy/antegen_thread_program.so")
    } else {
        runtime_dir.join("antegen_thread_program.so")
    };

    if program_path.exists() {
        args.push("--bpf-program".to_string());
        args.push(THREAD_PROGRAM_ID.to_string());
        args.push(program_path.to_string_lossy().to_string());
    }

    // Add Geyser plugin if configured
    if let Some(config) = geyser_config {
        args.push("--geyser-plugin-config".to_string());
        args.push(config.to_string_lossy().to_string());
    }

    // Get validator binary path
    let script = get_binary_path("solana-test-validator", runtime_dir, is_dev);

    let mut env = HashMap::new();
    env.insert(
        "RUST_LOG".to_string(),
        "solana_runtime::system_instruction_processor=error,\
         solana_runtime::bank=error,\
         antegen_client_geyser=info,\
         antegen_client=info,\
         antegen_processor=info,\
         antegen_submitter=info".to_string()
    );

    AppConfig {
        name: "antegen-validator".to_string(),
        script: script.to_string_lossy().to_string(),
        args: Some(args),
        cwd: Some(runtime_dir.to_string_lossy().to_string()),
        env: Some(env),
        auto_restart: Some(true),
        max_restarts: Some(3),
        restart_delay: Some(2000),
        depends_on: None,
        log_file: None,
        error_file: None,
    }
}

// TODO: Add custom data source service configuration when implemented
// pub fn custom_service(
//     name: &str,
//     rpc_url: &str,
//     runtime_dir: &PathBuf,
//     is_dev: bool,
//     verbose: bool,
// ) -> AppConfig {
//     // Implementation for custom data source client would go here
//     // This would create configuration for a custom datasource like Carbon, Yellowstone, etc.
//     todo!("Custom data source service not yet implemented")
// }

/// Create RPC client service configuration
pub fn rpc_service(
    name: &str,
    rpc_url: &str,
    runtime_dir: &PathBuf,
    is_dev: bool,
    verbose: bool,
) -> Result<AppConfig, anyhow::Error> {
    // Just construct the path where keypair should be stored
    // The client will handle creation/funding
    let keypair_path = runtime_dir
        .join("keypairs")
        .join(format!("{}-keypair.json", name));

    let mut args = vec![
        "--rpc-url".to_string(),
        rpc_url.to_string(),
        "--keypair".to_string(),
        keypair_path.to_string_lossy().to_string(),
        "--thread-program-id".to_string(),
        THREAD_PROGRAM_ID.to_string(),
        "--forgo-commission".to_string(),
    ];

    if verbose {
        args.push("--verbose".to_string());
    }

    // Get RPC binary path
    let script = get_binary_path("antegen-rpc", runtime_dir, is_dev);

    let mut env = HashMap::new();
    let log_level = if verbose {
        "debug,antegen_client=debug,antegen_processor=debug,antegen_submitter=debug"
    } else {
        "info,antegen_client=info,antegen_processor=info,antegen_submitter=info"
    };
    env.insert("RUST_LOG".to_string(), log_level.to_string());
    env.insert(
        "ANTEGEN_INSTANCE_NAME".to_string(),
        name.to_string()
    );

    Ok(AppConfig {
        name: name.to_string(),
        script: script.to_string_lossy().to_string(),
        args: Some(args),
        cwd: Some(runtime_dir.to_string_lossy().to_string()),
        env: Some(env),
        auto_restart: Some(true),
        max_restarts: Some(5),
        restart_delay: Some(3000),
        depends_on: Some(vec!["antegen-validator".to_string()]),
        log_file: None,
        error_file: None,
    })
}


/// Get the path to a binary based on dev/release mode
fn get_binary_path(binary_name: &str, runtime_dir: &PathBuf, is_dev: bool) -> PathBuf {
    if is_dev {
        // Get absolute path to current directory
        let current_dir = std::env::current_dir().unwrap_or_default();

        // Check for debug build first
        let debug_path = current_dir.join("target/debug").join(binary_name);
        if debug_path.exists() {
            return debug_path;
        }

        // Then check release build
        let release_path = current_dir.join("target/release").join(binary_name);
        if release_path.exists() {
            return release_path;
        }

        // Fallback to just the binary name (PATH resolution)
        PathBuf::from(binary_name)
    } else {
        // In release mode, use runtime directory
        runtime_dir.join(binary_name)
    }
}

/// Check and fund executor keypair if needed (localnet only)
pub fn check_and_fund_executor(keypair_path: &PathBuf, rpc_url: &str) -> anyhow::Result<()> {
    // Only for localnet
    if !rpc_url.contains("localhost") && !rpc_url.contains("127.0.0.1") {
        return Ok(());
    }

    // Check if keypair exists
    if keypair_path.exists() {
        println!("    Found keypair at {:?}", keypair_path);
        // Read keypair
        match read_keypair_file(keypair_path) {
            Ok(keypair) => {
                let pubkey = keypair.pubkey();
                println!("    Pubkey: {}", pubkey);

                // Check balance using solana CLI
                let output = std::process::Command::new("solana")
                    .args(&["balance", &pubkey.to_string(), "--url", rpc_url])
                    .output()?;

                if output.status.success() {
                    let balance_str = String::from_utf8_lossy(&output.stdout);
                    // Parse balance (format: "0.5 SOL" or "0 SOL")
                    if let Some(balance_part) = balance_str.split_whitespace().next() {
                        if let Ok(balance) = balance_part.parse::<f64>() {
                            // If balance < 0.1 SOL, airdrop
                            if balance < 0.1 {
                                println!("    Balance: {} SOL (insufficient), airdropping...", balance);
                                airdrop_to_executor(&pubkey, rpc_url)?;
                            } else {
                                println!("    Balance: {} SOL (sufficient)", balance);
                            }
                        }
                    }
                } else {
                    println!("    Could not check balance: {}", String::from_utf8_lossy(&output.stderr));
                }
            }
            Err(e) => {
                println!("    Warning: Could not read keypair: {}", e);
            }
        }
    } else {
        println!("    Keypair not found (client will create it)");
    }

    Ok(())
}

/// Airdrop SOL to an executor account
fn airdrop_to_executor(pubkey: &solana_sdk::pubkey::Pubkey, rpc_url: &str) -> anyhow::Result<()> {
    let output = std::process::Command::new("solana")
        .args(&[
            "airdrop",
            "1", // 1 SOL
            &pubkey.to_string(),
            "--url", rpc_url
        ])
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to execute airdrop command: {}", e))?;

    if output.status.success() {
        println!("  ✓ Successfully airdropped 1 SOL to {}", pubkey);
        Ok(())
    } else {
        // Don't fail if airdrop fails, client will wait for funding
        println!("  Warning: Airdrop failed, client will wait for manual funding");
        Ok(())
    }
}

/// Get client template configuration
pub fn get_client_template(
    client_type: &str,
    name: &str,
    rpc_url: Option<String>,
    keypair: Option<String>,
    verbose: bool,
) -> anyhow::Result<AppConfig> {
    let runtime_dir = dirs_next::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".antegen")
        .join("localnet");

    let is_dev = std::path::Path::new("target").exists();

    // Handle custom keypair if provided
    if let Some(_custom_keypair) = keypair {
        // TODO: Copy custom keypair to service-specific location
        // For now, we'll just use the service-specific generation
        eprintln!("Note: Custom keypair support coming soon. Using auto-generated keypair.");
    }

    match client_type {
        "rpc" => {
            let url = rpc_url.unwrap_or_else(|| "http://localhost:8899".to_string());
            rpc_service(name, &url, &runtime_dir, is_dev, verbose)
        }
        // TODO: Add custom data source when implemented
        // "custom" => {
        //     let url = rpc_url.unwrap_or_else(|| "http://localhost:8899".to_string());
        //     Ok(custom_service(name, &url, &runtime_dir, is_dev, verbose))
        // }
        "geyser" => {
            // Geyser requires special handling as it needs to be loaded as a validator plugin
            // To use Geyser:
            // 1. Start a test-validator with --geyser-plugin-config pointing to the Geyser plugin config
            // 2. The plugin will automatically create the Antegen Geyser client
            anyhow::bail!("Geyser client requires validator plugin integration. Use 'antegen localnet start --with-geyser' instead")
        }
        _ => anyhow::bail!("Unsupported client type: {}", client_type),
    }
}