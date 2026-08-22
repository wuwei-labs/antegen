//! Config file commands

use antegen_client::ClientConfig;
use anyhow::Result;
use clap::Subcommand;
use std::path::PathBuf;

/// Strip surrounding quotes from a string (handles user input with accidental quotes)
fn strip_quotes(s: String) -> String {
    s.trim_matches(|c| c == '"' || c == '\'').to_string()
}

/// Generate a default configuration file
pub(crate) fn init(
    output: PathBuf,
    rpc: Option<String>,
    keypair_path: Option<String>,
    storage_path: Option<String>,
    force: bool,
) -> Result<()> {
    if output.exists() && !force {
        anyhow::bail!(
            "Config file already exists: {}. Use --force to overwrite.",
            output.display()
        );
    }

    // Create parent directories if they don't exist
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut config = ClientConfig::default();

    // Apply overrides if provided (strip quotes for user-friendliness)
    if let Some(url) = rpc {
        config.rpc.endpoints[0].url = strip_quotes(url);
    }
    if let Some(path) = keypair_path {
        config.executor.keypair_path = strip_quotes(path);
    }
    if let Some(path) = storage_path {
        config.observability.storage_path = strip_quotes(path);
    }

    config.save(&output)?;

    // Set file permissions (640) and ownership (root:antegen) if possible
    #[cfg(unix)]
    {
        use file_owner::PathExt;
        use std::os::unix::fs::PermissionsExt;

        // Set permissions to 640 (owner rw, group r, others none)
        let _ = std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o640));

        // Set ownership to root:antegen (silently fails if not root or group doesn't exist)
        let _ = output.set_owner("root");
        let _ = output.set_group("antegen");
    }

    println!("✓ Generated config: {}", output.display());

    // Generate keypair if it doesn't exist
    let keypair_path = super::expand_tilde(&config.executor.keypair_path)?;
    let pubkey = super::ensure_keypair_exists(&keypair_path)?;
    println!("✓ Keypair: {} ({})", keypair_path.display(), pubkey);
    println!();
    println!("Next steps:");
    println!("  1. Fund address {} with SOL", pubkey);
    println!("  2. Run: antegen node start -c {}", output.display());

    Ok(())
}

/// Display the current configuration
pub(crate) fn get(config_path: PathBuf) -> Result<()> {
    let config = ClientConfig::load(&config_path)?;

    println!("Config: {}", config_path.display());
    println!();

    // Executor
    println!("[executor]");
    println!("  keypair_path       = {}", config.executor.keypair_path);
    println!(
        "  forgo_commission   = {}",
        config.executor.forgo_commission
    );
    println!();

    // RPC endpoints
    println!("[rpc]");
    for (i, ep) in config.rpc.endpoints.iter().enumerate() {
        println!("  [[endpoints]] #{}", i + 1);
        println!("    url      = {}", ep.url);
        if let Some(ws) = &ep.ws_url {
            println!("    ws_url   = {}", ws);
        }
        println!("    role     = {:?}", ep.role);
        println!("    priority = {}", ep.priority);
    }
    println!();

    // Datasources
    println!("[datasources]");
    println!("  commitment = {}", config.datasources.commitment);
    println!("  program_id = {}", config.datasources.program_id);
    println!();

    // Processor
    println!("[processor]");
    println!(
        "  max_concurrent_threads = {}",
        config.processor.max_concurrent_threads
    );
    println!();

    // Cache
    println!("[cache]");
    println!("  max_capacity = {}", config.cache.max_capacity);
    println!();

    // Load balancer
    println!("[load_balancer]");
    println!(
        "  grace_period         = {}s",
        config.load_balancer.grace_period
    );
    println!(
        "  eviction_buffer      = {}s",
        config.load_balancer.eviction_buffer
    );
    println!(
        "  thread_process_delay = {}s",
        config.load_balancer.thread_process_delay
    );
    println!();

    // Observability
    println!("[observability]");
    println!("  enabled      = {}", config.observability.enabled);
    println!("  storage_path = {}", config.observability.storage_path);
    println!();

    // TPU
    println!("[tpu]");
    println!("  enabled             = {}", config.tpu.enabled);
    println!("  num_connections     = {}", config.tpu.num_connections);
    println!("  leaders_fanout     = {}", config.tpu.leaders_fanout);
    println!("  worker_channel_size = {}", config.tpu.worker_channel_size);

    Ok(())
}

/// Update configuration values
#[allow(clippy::too_many_arguments)]
pub(crate) fn set(
    config_path: PathBuf,
    rpc: Option<String>,
    keypair_path: Option<String>,
    forgo_commission: Option<bool>,
    commitment: Option<String>,
    max_threads: Option<usize>,
    cache_max_capacity: Option<u64>,
    grace_period: Option<u64>,
    eviction_buffer: Option<u64>,
    thread_process_delay: Option<u64>,
    observability_enabled: Option<bool>,
    observability_storage_path: Option<String>,
    tpu_enabled: Option<bool>,
    tpu_num_connections: Option<usize>,
    tpu_leaders_fanout: Option<usize>,
) -> Result<()> {
    let mut config = ClientConfig::load(&config_path)?;
    let mut changes: Vec<String> = Vec::new();

    if let Some(v) = rpc {
        let v = strip_quotes(v);
        config.rpc.endpoints[0].url = v.clone();
        changes.push(format!("rpc.endpoints[0].url = {}", v));
    }
    if let Some(v) = keypair_path {
        let v = strip_quotes(v);
        config.executor.keypair_path = v.clone();
        changes.push(format!("executor.keypair_path = {}", v));
    }
    if let Some(v) = forgo_commission {
        config.executor.forgo_commission = v;
        changes.push(format!("executor.forgo_commission = {}", v));
    }
    if let Some(v) = commitment {
        let v = strip_quotes(v);
        config.datasources.commitment = v.clone();
        changes.push(format!("datasources.commitment = {}", v));
    }
    if let Some(v) = max_threads {
        config.processor.max_concurrent_threads = v;
        changes.push(format!("processor.max_concurrent_threads = {}", v));
    }
    if let Some(v) = cache_max_capacity {
        config.cache.max_capacity = v;
        changes.push(format!("cache.max_capacity = {}", v));
    }
    if let Some(v) = grace_period {
        config.load_balancer.grace_period = v;
        changes.push(format!("load_balancer.grace_period = {}", v));
    }
    if let Some(v) = eviction_buffer {
        config.load_balancer.eviction_buffer = v;
        changes.push(format!("load_balancer.eviction_buffer = {}", v));
    }
    if let Some(v) = thread_process_delay {
        config.load_balancer.thread_process_delay = v;
        changes.push(format!("load_balancer.thread_process_delay = {}", v));
    }
    if let Some(v) = observability_enabled {
        config.observability.enabled = v;
        changes.push(format!("observability.enabled = {}", v));
    }
    if let Some(v) = observability_storage_path {
        let v = strip_quotes(v);
        config.observability.storage_path = v.clone();
        changes.push(format!("observability.storage_path = {}", v));
    }
    if let Some(v) = tpu_enabled {
        config.tpu.enabled = v;
        changes.push(format!("tpu.enabled = {}", v));
    }
    if let Some(v) = tpu_num_connections {
        config.tpu.num_connections = v;
        changes.push(format!("tpu.num_connections = {}", v));
    }
    if let Some(v) = tpu_leaders_fanout {
        config.tpu.leaders_fanout = v;
        changes.push(format!("tpu.leaders_fanout = {}", v));
    }

    if changes.is_empty() {
        anyhow::bail!("No changes specified. Use --help to see available options.");
    }

    // Validate before saving
    config.validate()?;
    config.save(&config_path)?;

    println!("Updated {}:", config_path.display());
    for change in &changes {
        println!("  {}", change);
    }

    Ok(())
}

/// Validate a configuration file
pub(crate) fn validate(config_path: PathBuf) -> Result<()> {
    println!("Validating config: {}", config_path.display());

    let config = ClientConfig::load(&config_path)?;

    println!("✓ Config is valid");
    println!();
    println!("Configuration summary:");
    println!("  Executor keypair: {}", config.executor.keypair_path);
    println!("  Thread program: {}", config.datasources.program_id);
    println!(
        "  Max concurrent threads: {}",
        config.processor.max_concurrent_threads
    );
    println!("  RPC endpoints: {}", config.rpc.endpoints.len());

    let datasource_count = config
        .rpc
        .endpoints
        .iter()
        .filter(|e| {
            matches!(
                e.role,
                antegen_client::config::EndpointRole::Datasource
                    | antegen_client::config::EndpointRole::Both
            )
        })
        .count();
    let submission_count = config
        .rpc
        .endpoints
        .iter()
        .filter(|e| {
            matches!(
                e.role,
                antegen_client::config::EndpointRole::Submission
                    | antegen_client::config::EndpointRole::Both
            )
        })
        .count();

    println!("    - Datasource endpoints: {}", datasource_count);
    println!("    - Submission endpoints: {}", submission_count);

    if config.observability.enabled {
        println!(
            "  Observability: enabled (storage: {})",
            config.observability.storage_path
        );
    } else {
        println!("  Observability: disabled");
    }

    Ok(())
}

#[derive(Subcommand)]
pub(crate) enum NodeConfigCommands {
    /// Display the current executor node configuration
    Get {
        /// Path to config file (defaults to ~/.config/antegen/antegen.toml)
        #[arg(short, long)]
        config: Option<PathBuf>,
    },

    /// Update configuration values
    #[command(after_long_help = "\
EXAMPLES:
    antegen config set --max-threads 20
    antegen config set --commitment finalized --tpu-enabled false
    antegen config set --keypair-path ~/.antegen/my-keypair.json
    antegen config set --grace-period 15 --eviction-buffer 30
")]
    Set {
        /// Path to config file (defaults to ~/.config/antegen/antegen.toml)
        #[arg(short, long)]
        config: Option<PathBuf>,

        // -- executor --
        /// Path to executor keypair file
        #[arg(long)]
        keypair_path: Option<String>,

        /// Skip taking executor commission fee
        #[arg(long)]
        forgo_commission: Option<bool>,

        // -- datasources --
        /// Commitment level (processed, confirmed, finalized)
        #[arg(long)]
        commitment: Option<String>,

        // -- processor --
        /// Maximum number of concurrent threads to process
        #[arg(long)]
        max_threads: Option<usize>,

        // -- cache --
        /// Maximum number of accounts to cache
        #[arg(long)]
        cache_max_capacity: Option<u64>,

        // -- load_balancer --
        /// Grace period in seconds for fee decay calculations
        #[arg(long)]
        grace_period: Option<u64>,

        /// Eviction buffer in seconds (extra cache TTL after grace period)
        #[arg(long)]
        eviction_buffer: Option<u64>,

        /// Delay in seconds before claiming new threads
        #[arg(long)]
        thread_process_delay: Option<u64>,

        // -- observability --
        /// Enable/disable observability agent
        #[arg(long)]
        observability_enabled: Option<bool>,

        /// Storage path for observability data
        #[arg(long)]
        observability_storage_path: Option<String>,

        // -- tpu --
        /// Enable/disable TPU client for transaction submission
        #[arg(long)]
        tpu_enabled: Option<bool>,

        /// Number of QUIC connections per leader
        #[arg(long)]
        tpu_num_connections: Option<usize>,

        /// Number of leaders to fan out transactions to
        #[arg(long)]
        tpu_leaders_fanout: Option<usize>,
    },

    /// Generate default config file
    Init {
        /// Output path for config file (defaults to ~/.config/antegen/antegen.toml)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// RPC endpoint URL
        #[arg(long)]
        rpc: Option<String>,

        /// Path to executor keypair file
        #[arg(long)]
        keypair_path: Option<String>,

        /// Path to observability storage
        #[arg(long)]
        storage_path: Option<String>,

        /// Overwrite existing config file
        #[arg(long)]
        force: bool,
    },

    /// Validate config file
    Validate {
        /// Path to config file
        #[arg(short, long, default_value = "antegen.toml")]
        config: PathBuf,
    },
}

/// Dispatch a NodeConfigCommands variant to the appropriate handler
pub(crate) fn dispatch_config(
    config_cmd: NodeConfigCommands,
    global_rpc: Option<String>,
) -> anyhow::Result<()> {
    match config_cmd {
        NodeConfigCommands::Get { config } => {
            let path = config.map(Ok).unwrap_or_else(super::default_config_path)?;
            get(path)
        }
        NodeConfigCommands::Set {
            config,
            keypair_path,
            forgo_commission,
            commitment,
            max_threads,
            cache_max_capacity,
            grace_period,
            eviction_buffer,
            thread_process_delay,
            observability_enabled,
            observability_storage_path,
            tpu_enabled,
            tpu_num_connections,
            tpu_leaders_fanout,
        } => {
            let path = config.map(Ok).unwrap_or_else(super::default_config_path)?;
            set(
                path,
                global_rpc,
                keypair_path,
                forgo_commission,
                commitment,
                max_threads,
                cache_max_capacity,
                grace_period,
                eviction_buffer,
                thread_process_delay,
                observability_enabled,
                observability_storage_path,
                tpu_enabled,
                tpu_num_connections,
                tpu_leaders_fanout,
            )
        }
        NodeConfigCommands::Init {
            output,
            rpc,
            keypair_path,
            storage_path,
            force,
        } => {
            let path = output.map(Ok).unwrap_or_else(super::default_config_path)?;
            init(path, rpc, keypair_path, storage_path, force)
        }
        NodeConfigCommands::Validate { config } => validate(config),
    }
}
