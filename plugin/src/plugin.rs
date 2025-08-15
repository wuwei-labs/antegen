use {
    crate::{config::PluginConfig, events::AccountUpdateEvent, worker::PluginWorker},
    agave_geyser_plugin_interface::geyser_plugin_interface::{
        GeyserPlugin, GeyserPluginError, ReplicaAccountInfo, ReplicaAccountInfoVersions,
        Result as PluginResult, SlotStatus,
    },
    log::{debug, error, info},
    solana_program::pubkey::Pubkey,
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
    pub worker: Arc<PluginWorker>,
    pub runtime: Arc<Runtime>,
    pub block_height: Arc<std::sync::atomic::AtomicU64>,
}

impl GeyserPlugin for AntegenPlugin {
    fn name(&self) -> &'static str {
        "antegen-plugin"
    }

    fn on_load(&mut self, config_file: &str, is_reload: bool) -> PluginResult<()> {
        solana_logger::setup_with_default("info");
        info!(
            "antegen-plugin v{} - geyser_interface_version: {}, rustc: {}",
            env!("CARGO_PKG_VERSION"),
            env!("GEYSER_INTERFACE_VERSION"),
            env!("RUSTC_VERSION")
        );

        info!("Loading snapshot..., isReload: {}", is_reload);
        let config = PluginConfig::read_from(config_file)?;
        println!("config_file: {:?}", config_file);

        // Create runtime here
        let runtime = build_runtime(config.clone());

        // Initialize worker mode (builder + submitter)
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

            match PluginWorker::new(config.builder_id, rpc_url, ws_url, keypair_path).await {
                Ok(worker) => Ok(worker),
                Err(e) => {
                    let error_msg = format!("Failed to create worker: {}", e);
                    error!("{}", error_msg);
                    Err(GeyserPluginError::Custom(error_msg.into()))
                }
            }
        })?;

        // Start the worker services (spawns builder and submitter)
        worker.start(runtime.handle().clone())
            .map_err(|e| GeyserPluginError::Custom(format!("Failed to start worker services: {}", e).into()))?;
        
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
        let account_pubkey = Pubkey::try_from(account_info.pubkey).unwrap();
        let event = AccountUpdateEvent::try_from(account_info);

        // Process event on tokio task.
        inner.clone().spawn(|inner| async move {
            // Only process account updates if we're past the startup phase.
            if !is_startup {
                // Account updates could be sent to worker if needed
                // For now, we only care about specific events below
            }

            // Parse and process specific update events.
            if let Ok(event) = event {
                match event {
                    AccountUpdateEvent::Clock { clock } => {
                        // Get current block height
                        let block_height = inner.block_height.load(std::sync::atomic::Ordering::SeqCst);
                        // Send to worker for processing with block height
                        inner.worker.send_clock_event(clock, slot, block_height).await.ok();
                    }
                    AccountUpdateEvent::Thread { thread } => {
                        // Send to worker for processing
                        inner
                            .worker
                            .send_thread_event(thread, account_pubkey, slot)
                            .await
                            .ok();
                    }
                }
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
                    let new_height = inner.block_height.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
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
