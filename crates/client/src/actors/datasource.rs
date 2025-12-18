//! Datasource Supervisor and Source Actors
//!
//! The DatasourceSupervisor dynamically spawns source actors based on configuration:
//! - RpcSourceActor: Listens to WebSocket streams for account updates
//! - GeyserSourceActor: Consumes mpsc channel from Geyser plugin
//!
//! All source actors push updates through the shared cache for deduplication
//! before forwarding to StagingActor.

use crate::actors::messages::{DatasourceMessage, GeyserSourceMessage, RpcSourceMessage, StagingMessage};
use crate::config::{ClientConfig, EndpointRole, RpcEndpoint};
use crate::datasources::RpcSubscription;
use crate::resources::SharedResources;
use crate::types::AccountUpdate;
use ractor::{Actor, ActorProcessingErr, ActorRef};
use std::collections::HashMap;
use std::error::Error;
use tokio::sync::mpsc;

// ============================================================================
// Datasource Supervisor
// ============================================================================

#[derive(Default)]
pub struct DatasourceSupervisor;

pub struct DatasourceState {
    #[allow(dead_code)] // Kept for future supervisor functionality (stop/restart children)
    rpc_sources: HashMap<String, ActorRef<RpcSourceMessage>>,
    #[allow(dead_code)] // Kept for future supervisor functionality (stop/restart children)
    geyser_source: Option<ActorRef<GeyserSourceMessage>>,
}

#[ractor::async_trait]
impl Actor for DatasourceSupervisor {
    type Msg = DatasourceMessage;
    type State = DatasourceState;
    type Arguments = (
        ClientConfig,
        SharedResources,
        ActorRef<StagingMessage>,
        Option<mpsc::Receiver<AccountUpdate>>,
    );

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        (config, resources, staging_ref, geyser_receiver): Self::Arguments,
    ) -> Result<Self::State, Box<dyn Error + Send + Sync>> {
        log::debug!("DatasourceSupervisor starting...");

        let mut rpc_sources = HashMap::new();
        let mut datasource_count = 0;

        // Spawn RpcSourceActor for each datasource endpoint
        for endpoint in &config.rpc.endpoints {
            if matches!(endpoint.role, EndpointRole::Datasource | EndpointRole::Both) {
                let actor_name = format!("rpc-source-{}", endpoint.url);

                log::debug!("Spawning RpcSourceActor for: {}", endpoint.url);

                let (rpc_ref, _handle) = Actor::spawn(
                    Some(actor_name.clone()),
                    RpcSourceActor,
                    (endpoint.clone(), resources.clone(), staging_ref.clone()),
                )
                .await
                .map_err(|e| format!("Failed to spawn RpcSourceActor: {}", e))?;

                rpc_sources.insert(actor_name, rpc_ref);
                datasource_count += 1;
            }
        }

        log::debug!("Spawned {} RPC datasource actors", datasource_count);

        // Optionally spawn GeyserSourceActor if we have a channel from the plugin
        let geyser_source = if let Some(receiver) = geyser_receiver {
            log::info!("Spawning GeyserSourceActor for plugin mode");

            let (geyser_ref, _handle) = Actor::spawn(
                Some("geyser-source".to_string()),
                GeyserSourceActor,
                (receiver, resources.clone(), staging_ref.clone()),
            )
            .await
            .map_err(|e| format!("Failed to spawn GeyserSourceActor: {}", e))?;

            Some(geyser_ref)
        } else {
            None
        };

        Ok(DatasourceState {
            rpc_sources,
            geyser_source,
        })
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            DatasourceMessage::AccountUpdate(_update) => {
                // Datasource supervisor doesn't need to handle updates directly
                // RpcSourceActors send directly to StagingActor
                Ok(())
            }
            DatasourceMessage::Shutdown => {
                log::info!("DatasourceSupervisor shutting down...");
                Err(From::from("Shutdown signal received"))
            }
        }
    }

    async fn post_stop(
        &self,
        _myself: ActorRef<Self::Msg>,
        state: &mut Self::State,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        log::info!(
            "DatasourceSupervisor stopped. {} RPC sources cleaned up",
            state.rpc_sources.len()
        );
        Ok(())
    }
}

// ============================================================================
// RPC Source Actor
// ============================================================================

#[derive(Default)]
pub struct RpcSourceActor;

pub struct RpcSourceState {
    ws_url: String,
    staging_ref: ActorRef<StagingMessage>,
    resources: SharedResources,
}

#[ractor::async_trait]
impl Actor for RpcSourceActor {
    type Msg = RpcSourceMessage;
    type State = RpcSourceState;
    type Arguments = (RpcEndpoint, SharedResources, ActorRef<StagingMessage>);

    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        (endpoint, resources, staging_ref): Self::Arguments,
    ) -> Result<Self::State, Box<dyn Error + Send + Sync>> {
        let ws_url = endpoint.get_ws_url();
        log::debug!("RpcSourceActor starting for: {} (ws: {})", endpoint.url, ws_url);
        log::debug!("  - Thread program: {}", antegen_thread_program::ID);
        log::debug!("  - Clock sysvar: {}", solana_sdk::sysvar::clock::ID);

        // Spawn program subscription task (pws handles reconnection automatically)
        // Note: Backfill is triggered by ConnectionOpened event (handles both initial and reconnection)
        let program_ws_url = ws_url.clone();
        let program_id = antegen_thread_program::ID;
        let program_rpc_client = resources.rpc_client.clone();
        let program_actor_ref = myself.clone();
        tokio::spawn(async move {
            let subscription = RpcSubscription::new(program_ws_url, program_id, program_rpc_client);
            subscription
                .subscribe_to_program_accounts(program_actor_ref)
                .await;
        });

        // Spawn clock subscription task (pws handles reconnection automatically)
        let clock_ws_url = ws_url.clone();
        let clock_rpc_client = resources.rpc_client.clone();
        let clock_actor_ref = myself.clone();
        tokio::spawn(async move {
            let subscription = RpcSubscription::new(clock_ws_url, antegen_thread_program::ID, clock_rpc_client);
            subscription
                .subscribe_to_clock(clock_actor_ref)
                .await;
        });

        Ok(RpcSourceState {
            ws_url,
            staging_ref,
            resources,
        })
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            RpcSourceMessage::UpdateReceived(update) => {
                log::trace!(
                    "[{}] Received account update: pubkey={}, slot={}, data_len={}",
                    state.ws_url,
                    update.pubkey,
                    update.slot,
                    update.data.len()
                );

                // Push to cache first - this deduplicates and stores the data
                // Returns true only if this is new/newer data
                let is_new = state
                    .resources
                    .cache
                    .put_if_newer(update.pubkey, update.data.clone(), update.slot)
                    .await;

                if is_new {
                    log::debug!(
                        "[{}] New/updated account: pubkey={}, slot={}",
                        state.ws_url,
                        update.pubkey,
                        update.slot
                    );

                    // Forward to StagingActor only if data was actually new/updated
                    state
                        .staging_ref
                        .send_message(StagingMessage::AccountUpdate(update))
                        .map_err(|e| format!("Failed to send to staging: {}", e))?;
                } else {
                    log::trace!(
                        "[{}] Duplicate/stale account update ignored: pubkey={}",
                        state.ws_url,
                        update.pubkey
                    );
                }

                Ok(())
            }
            RpcSourceMessage::ClockReceived(clock) => {
                log::trace!(
                    "[{}] Received clock update: slot={}, timestamp={}",
                    state.ws_url,
                    clock.slot,
                    clock.unix_timestamp
                );

                // Clock is NOT cached - always forward fresh to StagingActor
                state
                    .staging_ref
                    .send_message(StagingMessage::ClockTick(clock))
                    .map_err(|e| format!("Failed to send clock to staging: {}", e))?;

                Ok(())
            }
            RpcSourceMessage::Reconnected => {
                // WebSocket connected - perform backfill to load/refresh threads
                let subscription = RpcSubscription::new(
                    state.ws_url.clone(),
                    antegen_thread_program::ID,
                    state.resources.rpc_client.clone(),
                );
                if let Err(e) = subscription.perform_backfill(myself.clone()).await {
                    log::error!("[{}] Backfill failed: {}", state.ws_url, e);
                }

                Ok(())
            }
        }
    }

    async fn post_stop(
        &self,
        _myself: ActorRef<Self::Msg>,
        state: &mut Self::State,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        log::info!("RpcSourceActor for {} stopped", state.ws_url);
        Ok(())
    }
}

// ============================================================================
// Geyser Source Actor
// ============================================================================

/// Actor that consumes account updates from the Geyser plugin channel
#[derive(Default)]
pub struct GeyserSourceActor;

pub struct GeyserSourceState {
    #[allow(dead_code)] // Kept for future message handling (supervisor commands)
    staging_ref: ActorRef<StagingMessage>,
    #[allow(dead_code)] // Kept for future message handling (supervisor commands)
    resources: SharedResources,
}

#[ractor::async_trait]
impl Actor for GeyserSourceActor {
    type Msg = GeyserSourceMessage;
    type State = GeyserSourceState;
    type Arguments = (
        mpsc::Receiver<AccountUpdate>,
        SharedResources,
        ActorRef<StagingMessage>,
    );

    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        (mut receiver, resources, staging_ref): Self::Arguments,
    ) -> Result<Self::State, Box<dyn Error + Send + Sync>> {
        log::debug!("GeyserSourceActor starting...");

        // Spawn task to consume the channel
        let cache = resources.cache.clone();
        let staging = staging_ref.clone();
        let actor_ref = myself.clone();

        tokio::spawn(async move {
            log::info!("GeyserSourceActor channel consumer started");

            while let Some(update) = receiver.recv().await {
                log::trace!(
                    "[Geyser] Received account update: pubkey={}, slot={}, data_len={}",
                    update.pubkey,
                    update.slot,
                    update.data.len()
                );

                // Push to cache first - this deduplicates and stores the data
                let is_new = cache
                    .put_if_newer(update.pubkey, update.data.clone(), update.slot)
                    .await;

                if is_new {
                    log::debug!(
                        "[Geyser] New/updated account: pubkey={}, slot={}",
                        update.pubkey,
                        update.slot
                    );

                    // Forward to StagingActor only if data was actually new/updated
                    if let Err(e) = staging.send_message(StagingMessage::AccountUpdate(update)) {
                        log::error!("[Geyser] Failed to send to staging: {}", e);
                        break;
                    }
                } else {
                    log::trace!(
                        "[Geyser] Duplicate/stale account update ignored: pubkey={}",
                        update.pubkey
                    );
                }
            }

            log::info!("GeyserSourceActor channel consumer stopped (channel closed)");

            // Signal actor to stop when channel closes
            let _ = actor_ref.send_message(GeyserSourceMessage::Shutdown);
        });

        Ok(GeyserSourceState {
            staging_ref,
            resources,
        })
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            GeyserSourceMessage::Shutdown => {
                log::info!("GeyserSourceActor received shutdown signal");
                myself.stop(Some("Channel closed".to_string()));
                Ok(())
            }
        }
    }

    async fn post_stop(
        &self,
        _myself: ActorRef<Self::Msg>,
        _state: &mut Self::State,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        log::info!("GeyserSourceActor stopped");
        Ok(())
    }
}

