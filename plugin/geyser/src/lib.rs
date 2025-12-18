//! Antegen Geyser Plugin - Thin wrapper for validator integration
//!
//! This plugin is loaded by the Solana validator and forwards account updates
//! to the Antegen client via PluginHandle.

use agave_geyser_plugin_interface::geyser_plugin_interface::{
    GeyserPlugin, GeyserPluginError, ReplicaAccountInfo, ReplicaAccountInfoVersions,
    Result as PluginResult,
};
use antegen_client::{AccountUpdate, ClientConfig, PluginHandle};
use solana_program::pubkey::Pubkey;
use std::sync::Arc;
use tokio::runtime::Runtime;

#[derive(Debug)]
pub struct AntegenPlugin {
    inner: Option<Arc<Inner>>,
}

struct Inner {
    _runtime: Arc<Runtime>, // Kept alive to prevent runtime drop while plugin is active
    handle: PluginHandle,
    program_id: Pubkey,
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inner")
            .field("_runtime", &"Arc<Runtime>")
            .field("handle", &"PluginHandle")
            .field("program_id", &self.program_id)
            .finish()
    }
}

impl GeyserPlugin for AntegenPlugin {
    fn name(&self) -> &'static str {
        "antegen-plugin"
    }

    fn on_load(&mut self, config_file: &str, _is_reload: bool) -> PluginResult<()> {
        // Setup logging
        solana_logger::setup_with_default(
            "info,antegen_client=info,antegen_client_geyser=info"
        );

        log::info!("=== Antegen Plugin Loading ===");
        log::info!("Reading configuration from: {}", config_file);

        // Load configuration from the path specified by the validator
        let config = ClientConfig::load(config_file)
            .map_err(|e| GeyserPluginError::ConfigFileReadError {
                msg: format!("Failed to load config file '{}': {}", config_file, e),
            })?;

        let program_id = config.datasources.program_id();
        log::info!("Thread program: {}", program_id);
        log::info!("Max concurrent threads: {}", config.processor.max_concurrent_threads);

        // Create tokio runtime
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("antegen-plugin")
                .worker_threads(4) // Adjust as needed
                .build()
                .map_err(|e| GeyserPluginError::ConfigFileReadError {
                    msg: format!("Failed to create runtime: {}", e),
                })?
        );

        log::info!("Created tokio runtime");

        // Spawn client in plugin mode
        let handle = runtime.block_on(async {
            PluginHandle::spawn(config).await
        }).map_err(|e| GeyserPluginError::ConfigFileReadError {
            msg: format!("Failed to spawn client: {}", e),
        })?;

        log::info!("Spawned Antegen client in plugin mode");

        self.inner = Some(Arc::new(Inner {
            _runtime: runtime,
            handle,
            program_id,
        }));

        log::info!("=== Antegen Plugin Loaded Successfully ===");
        Ok(())
    }

    fn on_unload(&mut self) {
        log::info!("=== Antegen Plugin Unloading ===");
        self.inner = None;
    }

    fn update_account(
        &self,
        account: ReplicaAccountInfoVersions,
        slot: u64,
        _is_startup: bool,
    ) -> PluginResult<()> {
        let inner = match &self.inner {
            Some(inner) => inner.clone(),
            None => return Ok(()), // Not initialized yet
        };

        // Parse account info
        let account_info: ReplicaAccountInfo = match account {
            ReplicaAccountInfoVersions::V0_0_1(info) => ReplicaAccountInfo {
                pubkey: info.pubkey,
                lamports: info.lamports,
                owner: info.owner,
                executable: info.executable,
                rent_epoch: info.rent_epoch,
                data: info.data,
                write_version: info.write_version,
            },
            ReplicaAccountInfoVersions::V0_0_2(info) => ReplicaAccountInfo {
                pubkey: info.pubkey,
                lamports: info.lamports,
                owner: info.owner,
                executable: info.executable,
                rent_epoch: info.rent_epoch,
                data: info.data,
                write_version: info.write_version,
            },
            ReplicaAccountInfoVersions::V0_0_3(info) => ReplicaAccountInfo {
                pubkey: info.pubkey,
                lamports: info.lamports,
                owner: info.owner,
                executable: info.executable,
                rent_epoch: info.rent_epoch,
                data: info.data,
                write_version: info.write_version,
            },
        };

        // Parse pubkeys
        let pubkey = Pubkey::try_from(account_info.pubkey)
            .map_err(|e| GeyserPluginError::AccountsUpdateError {
                msg: format!("Failed to parse account pubkey: {}", e),
            })?;

        let owner = Pubkey::try_from(account_info.owner)
            .map_err(|e| GeyserPluginError::AccountsUpdateError {
                msg: format!("Failed to parse owner pubkey: {}", e),
            })?;

        // Filter: only thread program accounts or clock sysvar
        let is_clock = pubkey == solana_program::sysvar::clock::ID;
        let is_thread_account = owner == inner.program_id;

        if !is_clock && !is_thread_account {
            return Ok(());
        }

        // Create account update
        let update = AccountUpdate::new(
            pubkey,
            account_info.data.to_vec(),
            slot,
        );

        // Send to client (non-blocking)
        if let Err(e) = inner.handle.try_send_update(update) {
            log::warn!("Failed to send account update: {}", e);
        }

        Ok(())
    }

    fn account_data_notifications_enabled(&self) -> bool {
        true
    }

    fn transaction_notifications_enabled(&self) -> bool {
        false
    }
}

impl Default for AntegenPlugin {
    fn default() -> Self {
        Self { inner: None }
    }
}

#[no_mangle]
#[allow(improper_ctypes_definitions)]
pub unsafe extern "C" fn _create_plugin() -> *mut dyn GeyserPlugin {
    Box::into_raw(Box::new(AntegenPlugin::default()))
}

