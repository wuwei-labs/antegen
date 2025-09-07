use std::collections::HashMap;
use std::path::PathBuf;
use super::daemon::AppConfig;

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
        args.push("AgThdyi1P5RkVeZD2rQahTvs8HePJoGFFxKtvok5s2J1".to_string());
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
         solana_runtime::bank=error".to_string()
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

/// Create carbon client service configuration
pub fn carbon_service(
    name: &str,
    rpc_url: &str,
    runtime_dir: &PathBuf,
    is_dev: bool,
) -> AppConfig {
    let keypair_path = runtime_dir.join("executor-keypair.json");
    
    let args = vec![
        "--datasource".to_string(),
        "rpc".to_string(),
        "--rpc-url".to_string(),
        rpc_url.to_string(),
        "--keypair".to_string(),
        keypair_path.to_string_lossy().to_string(),
        "--thread-program-id".to_string(),
        "AgThdyi1P5RkVeZD2rQahTvs8HePJoGFFxKtvok5s2J1".to_string(),
    ];
    
    // Get carbon binary path
    let script = get_binary_path("antegen-carbon", runtime_dir, is_dev);
    
    let mut env = HashMap::new();
    env.insert(
        "RUST_LOG".to_string(),
        "info,antegen_carbon=debug,antegen_processor=debug".to_string()
    );
    // Pass the instance name as an environment variable so carbon can use it for logging
    env.insert(
        "ANTEGEN_INSTANCE_NAME".to_string(),
        name.to_string()
    );
    
    AppConfig {
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
    }
}

/// Create RPC client service configuration
pub fn rpc_service(
    name: &str,
    rpc_url: &str,
    runtime_dir: &PathBuf,
    is_dev: bool,
) -> AppConfig {
    let keypair_path = runtime_dir.join("executor-keypair.json");
    
    let args = vec![
        "--rpc-url".to_string(),
        rpc_url.to_string(),
        "--keypair".to_string(),
        keypair_path.to_string_lossy().to_string(),
        "--thread-program-id".to_string(),
        "AgThdyi1P5RkVeZD2rQahTvs8HePJoGFFxKtvok5s2J1".to_string(),
        "--forgo-commission".to_string(),
    ];
    
    // Get RPC binary path
    let script = get_binary_path("antegen-rpc", runtime_dir, is_dev);
    
    let mut env = HashMap::new();
    env.insert(
        "RUST_LOG".to_string(),
        "info,antegen_client=debug,antegen_processor=debug".to_string()
    );
    env.insert(
        "ANTEGEN_INSTANCE_NAME".to_string(),
        name.to_string()
    );
    
    AppConfig {
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
    }
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

/// Get client template configuration
pub fn get_client_template(
    client_type: &str,
    name: &str,
    rpc_url: Option<String>,
    _keypair: Option<String>,
) -> anyhow::Result<AppConfig> {
    let runtime_dir = dirs_next::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".antegen")
        .join("localnet");
    
    let is_dev = std::path::Path::new("target").exists();
    
    match client_type {
        "rpc" => {
            let url = rpc_url.unwrap_or_else(|| "http://localhost:8899".to_string());
            Ok(rpc_service(name, &url, &runtime_dir, is_dev))
        }
        "carbon" => {
            let url = rpc_url.unwrap_or_else(|| "http://localhost:8899".to_string());
            Ok(carbon_service(name, &url, &runtime_dir, is_dev))
        }
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