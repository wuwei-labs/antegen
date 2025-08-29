use {
    crate::{
        builder::PluginWorkerBuilder, config::PluginConfig, events::replica_account_to_account,
    },
    agave_geyser_plugin_interface::geyser_plugin_interface::{
        GeyserPlugin, GeyserPluginError, ReplicaAccountInfo, ReplicaAccountInfoVersions,
        Result as PluginResult, SlotStatus,
    },
    log::{debug, error, info},
    std::{fmt::Debug, sync::Arc},
    tokio::runtime::{Builder, Runtime},
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

#[derive(Debug)]
pub struct Inner {
    pub config: PluginConfig,
    pub worker: Arc<PluginWorkerBuilder>,
    pub runtime: Arc<Runtime>,
    pub block_height: Arc<std::sync::atomic::AtomicU64>,
}

impl GeyserPlugin for AntegenPlugin {
    fn name(&self) -> &'static str {
        "antegen-plugin"
    }

    fn on_load(&mut self, config_file: &str, _is_reload: bool) -> PluginResult<()> {
        solana_logger::setup_with_default("info");
        // Plugin version info

        let config = PluginConfig::read_from(config_file)?;

        // Create runtime here
        let runtime = build_runtime(config.clone());

        // Always initialize a basic meter provider for metrics collection
        // This ensures metrics are collected even if not exposed via HTTP
        let registry = runtime.block_on(async {
            crate::metrics::init_basic_meter_provider().map_err(|e| {
                GeyserPluginError::Custom(
                    format!("Failed to initialize meter provider: {}", e).into(),
                )
            })
        })?;
        debug!("Basic meter provider initialized with registry");

        // Initialize metrics HTTP server if configured
        if let Some(ref metrics_config) = config.metrics {
            if metrics_config.enabled {
                info!("=== Initializing Metrics HTTP Service ===");
                info!("Metrics backend: {:?}", metrics_config.backend);

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

                info!("=== Metrics HTTP Service Started ===");
            } else {
                info!("Metrics HTTP server disabled in configuration");
            }
        } else {
            debug!("No metrics HTTP configuration provided");
        }

        // Initialize worker using builder pattern
        info!("Initializing worker with builder pattern");

        let mut worker = runtime.block_on(async {
            let rpc_url = config
                .rpc_url
                .clone()
                .unwrap_or_else(|| "http://localhost:8899".to_string());
            let ws_url = config
                .ws_url
                .clone()
                .unwrap_or_else(|| "ws://localhost:8900".to_string());
            let keypair_path = config.keypath.clone().unwrap_or_else(|| {
                format!("{}/.config/solana/id.json", std::env::var("HOME").unwrap())
            });

            let forgo_executor_commission = config.forgo_executor_commission.unwrap_or(false);
            let enable_replay = config.enable_replay.unwrap_or(false);
            let nats_url = config.nats_url.clone();

            match PluginWorkerBuilder::new(
                rpc_url,
                ws_url,
                keypair_path,
                forgo_executor_commission,
                enable_replay,
                nats_url,
            )
            .await
            {
                Ok(worker) => Ok(worker),
                Err(e) => {
                    let error_msg = format!("Failed to create worker: {}", e);
                    error!("{}", error_msg);
                    Err(GeyserPluginError::Custom(error_msg.into()))
                }
            }
        })?;

        // Start the worker services
        worker.start(runtime.handle().clone()).map_err(|e| {
            GeyserPluginError::Custom(format!("Failed to start worker services: {}", e).into())
        })?;

        let worker = Arc::new(worker);

        self.inner = Some(Arc::new(Inner {
            config,
            worker,
            runtime,
            block_height: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }));

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
        let inner = match &self.inner {
            Some(inner) => inner.clone(),
            None => return Ok(()), // No-op if not initialized
        };

        // Parse account info.
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

        // Convert to standard account
        let account_result = replica_account_to_account(account_info);

        // Process event on tokio task.
        inner.clone().spawn(|inner| async move {
            // Only process account updates if we're past the startup phase.
            if is_startup {
                // Skip startup accounts
                return Ok(());
            }

            // Forward all accounts to worker
            if let Ok((pubkey, account)) = account_result {
                inner
                    .worker
                    .send_account_event(pubkey, account, slot)
                    .await
                    .ok();
            }
            Ok(())
        });
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
                    let new_height = inner
                        .block_height
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                        + 1;
                    info!("Block {} confirmed (slot {})", new_height, slot);

                    // Send block height update to worker
                    // This will be included with the next clock event
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
