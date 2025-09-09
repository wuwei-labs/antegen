use {
    crate::{events::replica_account_to_update, utils::PluginConfig},
    agave_geyser_plugin_interface::geyser_plugin_interface::{
        GeyserPlugin, GeyserPluginError, ReplicaAccountInfo, ReplicaAccountInfoVersions,
        Result as PluginResult, SlotStatus,
    },
    antegen_client::{AntegenClientBuilder, GeyserDatasource},
    antegen_processor::{builder::ProcessorBuilder, types::AccountUpdate},
    antegen_submitter::builder::SubmitterBuilder,
    log::{debug, error, info},
    solana_sdk::signature::read_keypair_file,
    std::{fmt::Debug, sync::Arc},
    tokio::{
        runtime::{Builder, Runtime},
        sync::mpsc,
        task::JoinHandle,
    },
};

pub struct AntegenPlugin {
    pub inner: Option<Arc<Inner>>,
}

impl Debug for AntegenPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.inner {
            Some(inner) => write!(f, "inner: {:?}", inner),
            None => write!(f, "inner: None"),
        }
    }
}

pub struct Inner {
    pub config: PluginConfig,
    pub runtime: Arc<Runtime>,
    pub account_sender: mpsc::Sender<AccountUpdate>,
    pub client_handle: JoinHandle<anyhow::Result<()>>,
    pub block_height: Arc<std::sync::atomic::AtomicU64>,
}

impl Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inner")
            .field("config", &self.config)
            .field("runtime", &"Arc<Runtime>")
            .field("account_sender", &"Sender<AccountUpdate>")
            .field("client_handle", &"JoinHandle")
            .field("block_height", &self.block_height)
            .finish()
    }
}

impl GeyserPlugin for AntegenPlugin {
    fn name(&self) -> &'static str {
        "antegen-plugin"
    }

    fn on_load(&mut self, _config_file: &str, _is_reload: bool) -> PluginResult<()> {
        solana_logger::setup_with_default("info");

        info!("ANTEGEN PLUGIN: on_load called");

        // Read config from known location
        info!("ANTEGEN PLUGIN: Reading config from geyser-plugin-config.json");

        let config_content = std::fs::read_to_string("geyser-plugin-config.json")
            .or_else(|_| std::fs::read_to_string("./geyser-plugin-config.json"))
            .map_err(|e| GeyserPluginError::ConfigFileReadError {
                msg: format!("Failed to read config file: {}", e),
            })?;

        info!("ANTEGEN PLUGIN: Config file read successfully");

        let mut config: PluginConfig = serde_json::from_str(&config_content).map_err(|e| {
            GeyserPluginError::ConfigFileReadError {
                msg: format!("Failed to parse config JSON: {}", e),
            }
        })?;

        // Apply environment variable overrides
        config.apply_env_overrides();

        info!("ANTEGEN PLUGIN: Config parsed successfully");

        // Create runtime
        info!(
            "ANTEGEN PLUGIN: Creating runtime with {} threads",
            config.thread_count
        );
        let runtime = build_runtime(config.clone());
        info!("ANTEGEN PLUGIN: Runtime created successfully");

        // Initialize metrics if configured
        if let Some(ref metrics_config) = config.metrics {
            if metrics_config.enabled {
                info!("=== Initializing Metrics ===");
                info!("Metrics backend: {:?}", metrics_config.backend);

                // Initialize basic meter provider for metrics collection
                let registry = runtime.block_on(async {
                    crate::metrics::init_basic_meter_provider().map_err(|e| {
                        GeyserPluginError::Custom(
                            format!("Failed to initialize meter provider: {}", e).into(),
                        )
                    })
                })?;
                debug!("Basic meter provider initialized with registry");

                // Initialize metrics HTTP server
                let handle = runtime.handle().clone();
                runtime.block_on(async {
                    crate::metrics::init_metrics(metrics_config, registry, handle)
                        .await
                        .map_err(|e| {
                            GeyserPluginError::Custom(
                                format!("Failed to initialize metrics HTTP server: {}", e).into(),
                            )
                        })
                })?;

                info!("=== Metrics Service Started ===");
            } else {
                info!("Metrics disabled in configuration");
            }
        } else {
            debug!("No metrics configuration provided - skipping metrics initialization");
        }

        // Create unified Antegen client
        info!("=== Creating Antegen Client ===");

        // Get configuration values
        let rpc_url = config
            .rpc_url
            .clone()
            .unwrap_or_else(|| "http://localhost:8899".to_string());
        let keypair_path = config.keypath.clone().unwrap_or_else(|| {
            format!("{}/.config/solana/id.json", std::env::var("HOME").unwrap())
        });
        let forgo_executor_commission = config.forgo_executor_commission.unwrap_or(false);
        let enable_replay = config.enable_replay.unwrap_or(false);
        let nats_url = config.nats_url.clone();

        info!("Configuration: RPC={}, keypair={}, replay={}", rpc_url, keypair_path, enable_replay);

        // Create Geyser datasource with channel
        let mut geyser_datasource = GeyserDatasource::new();
        let account_tx = geyser_datasource.get_plugin_sender();
        info!("Created Geyser datasource with channel");

        // Build the client using AntegenClientBuilder
        let mut client_builder = AntegenClientBuilder::default()
            .rpc_url(rpc_url.clone())
            .datasource(Box::new(geyser_datasource))
            .processor(
                ProcessorBuilder::new()
                    .keypair(keypair_path.clone())
                    .rpc_url(rpc_url.clone())
                    .forgo_commission(forgo_executor_commission),
            );

        // Add submitter if replay is enabled
        if enable_replay {
            info!("Enabling replay with NATS URL: {:?}", nats_url);
            let mut replay_config = antegen_submitter::ReplayConfig::default();
            replay_config.enable_replay = true;
            replay_config.nats_url = nats_url;

            client_builder = client_builder.submitter(
                SubmitterBuilder::new()
                    .rpc_url(rpc_url)
                    .executor_keypair(Arc::new(read_keypair_file(&keypair_path).map_err(|e| {
                        GeyserPluginError::Custom(format!("Failed to read keypair: {}", e).into())
                    })?))
                    .replay_config(replay_config)
                    .tpu_enabled(),
            );
        }

        // Build the client
        let client = runtime.block_on(async {
            client_builder.build().await.map_err(|e| {
                GeyserPluginError::Custom(format!("Failed to build AntegenClient: {}", e).into())
            })
        })?;

        info!("AntegenClient built successfully");

        // Start the client in the background
        let client_handle = runtime.spawn(async move {
            info!("AntegenClient starting...");
            match client.run().await {
                Ok(_) => {
                    info!("AntegenClient completed normally");
                    Ok(())
                }
                Err(e) => {
                    error!("AntegenClient error: {}", e);
                    Err(e)
                }
            }
        });

        info!("AntegenClient task started");

        self.inner = Some(Arc::new(Inner {
            config,
            runtime,
            account_sender: account_tx,
            client_handle,
            block_height: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }));

        info!("ANTEGEN PLUGIN: on_load completed successfully");
        Ok(())
    }

    fn on_unload(&mut self) {
        // Shutdown metrics if enabled
        if self
            .inner
            .as_ref()
            .and_then(|i| i.config.metrics.as_ref())
            .is_some()
        {
            info!("=== Shutting Down Metrics Service ===");
            crate::metrics::shutdown();
        }

        // Clean up resources if needed
        self.inner = None;
    }

    fn update_account(
        &self,
        account: ReplicaAccountInfoVersions,
        slot: u64,
        is_startup: bool,
    ) -> PluginResult<()> {
        // Skip startup accounts
        if is_startup {
            debug!("update_account called during startup for slot {}", slot);
            return Ok(());
        }

        let inner = match &self.inner {
            Some(inner) => inner.clone(),
            None => {
                debug!("update_account called but inner not initialized");
                return Ok(());
            }
        };

        // Parse account info
        let account_info = &mut match account {
            ReplicaAccountInfoVersions::V0_0_1(account_info) => ReplicaAccountInfo {
                pubkey: account_info.pubkey,
                lamports: account_info.lamports,
                owner: account_info.owner,
                executable: account_info.executable,
                rent_epoch: account_info.rent_epoch,
                data: account_info.data,
                write_version: account_info.write_version,
            },
            ReplicaAccountInfoVersions::V0_0_2(account_info) => ReplicaAccountInfo {
                pubkey: account_info.pubkey,
                lamports: account_info.lamports,
                owner: account_info.owner,
                executable: account_info.executable,
                rent_epoch: account_info.rent_epoch,
                data: account_info.data,
                write_version: account_info.write_version,
            },
            ReplicaAccountInfoVersions::V0_0_3(account_info) => ReplicaAccountInfo {
                pubkey: account_info.pubkey,
                lamports: account_info.lamports,
                owner: account_info.owner,
                executable: account_info.executable,
                rent_epoch: account_info.rent_epoch,
                data: account_info.data,
                write_version: account_info.write_version,
            },
        };

        // Convert to AccountUpdate
        let account_update = replica_account_to_update(account_info)?;

        // Send to processor using try_send (non-blocking)
        // Since we're not in an async context, use try_send
        if let Err(e) = inner.account_sender.try_send(account_update) {
            debug!("Failed to send account update: {}", e);
        }
        Ok(())
    }

    fn notify_end_of_startup(&self) -> PluginResult<()> {
        info!("Snapshot loaded");
        Ok(())
    }

    fn update_slot_status(
        &self,
        slot: u64,
        _parent: Option<u64>,
        status: &SlotStatus,
    ) -> PluginResult<()> {
        let inner = match &self.inner {
            Some(inner) => inner.clone(),
            None => return Ok(()), // No-op if not initialized
        };

        let status = status.clone();
        inner.clone().spawn(|inner| async move {
            match status {
                SlotStatus::Confirmed | SlotStatus::Rooted => {
                    // Increment block height for confirmed/finalized slots
                    inner
                        .block_height
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                SlotStatus::Processed => {
                    debug!("Slot {} processed", slot);
                }
                _ => (),
            }
            Ok(())
        });
        Ok(())
    }

    fn notify_transaction(
        &self,
        _transaction: agave_geyser_plugin_interface::geyser_plugin_interface::ReplicaTransactionInfoVersions,
        _slot: u64,
    ) -> PluginResult<()> {
        Ok(())
    }

    fn notify_block_metadata(
        &self,
        _blockinfo: agave_geyser_plugin_interface::geyser_plugin_interface::ReplicaBlockInfoVersions,
    ) -> PluginResult<()> {
        Ok(())
    }

    fn account_data_notifications_enabled(&self) -> bool {
        true
    }

    fn transaction_notifications_enabled(&self) -> bool {
        false
    }
}

impl AntegenPlugin {
    pub fn new() -> Self {
        Self { inner: None }
    }
}

impl Default for AntegenPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Inner {
    fn spawn<F: std::future::Future<Output = PluginResult<()>> + Send + 'static>(
        self: Arc<Self>,
        f: impl FnOnce(Arc<Self>) -> F,
    ) {
        self.runtime.spawn(f(self.clone()));
    }
}

fn build_runtime(config: PluginConfig) -> Arc<Runtime> {
    Arc::new(
        Builder::new_multi_thread()
            .enable_all()
            .thread_name("antegen-plugin")
            .worker_threads(config.thread_count)
            .max_blocking_threads(config.thread_count)
            .build()
            .unwrap(),
    )
}
