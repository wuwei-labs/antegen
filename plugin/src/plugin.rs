use {
    crate::{config::PluginConfig, events::AccountUpdateEvent, observers::Observers},
    agave_geyser_plugin_interface::geyser_plugin_interface::{
        GeyserPlugin, GeyserPluginError, ReplicaAccountInfo, ReplicaAccountInfoVersions,
        Result as PluginResult, SlotStatus,
    },
    log::{error, info},
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
    // pub executors: Arc<Executors>,
    pub observers: Arc<Observers>,
    pub runtime: Arc<Runtime>,
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

        // Initialize observers asynchronously
        let observers_result = runtime.block_on(async {
            match Observers::new().await {
                // Note the .await here
                Ok(observers) => Ok(observers),
                Err(e) => {
                    let error_msg = format!("Failed to create observers: {}", e);
                    error!("{}", error_msg);
                    Err(GeyserPluginError::Custom(error_msg.into()))
                }
            }
        })?;

        // let executors = Arc::new(Executors::new(config.clone()));

        self.inner = Some(Arc::new(Inner {
            config,
            // executors,
            observers: Arc::new(observers_result),
            runtime,
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
            // Send all account updates to the thread observer for account listeners.
            // Only process account updates if we're past the startup phase.
            if !is_startup {
                inner
                    .observers
                    .plugin
                    .clone()
                    .observe_account(account_pubkey, slot)
                    .await?;
            }

            // Parse and process specific update events.
            if let Ok(event) = event {
                match event {
                    AccountUpdateEvent::Clock { clock } => {
                        inner
                            .observers
                            .plugin
                            .clone()
                            .observe_clock(clock)
                            .await
                            .ok();
                    }
                    AccountUpdateEvent::Thread { thread } => {
                        inner
                            .observers
                            .plugin
                            .clone()
                            .observe_thread(thread, account_pubkey, slot)
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
                SlotStatus::Processed => {
                    inner.observers.plugin.clone().process_slot(slot).await?;
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
