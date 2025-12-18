//! Root Supervisor Actor
//!
//! The root supervisor manages the entire actor hierarchy and handles graceful shutdown
//! via SIGINT (Ctrl+C) and SIGTERM signals.

use crate::actors::messages::{RootMessage, StagingMessage};
use crate::actors::observability::{ObservabilityActor, ObservabilityMessage};
use crate::actors::{DatasourceSupervisor, ProcessorFactory, StagingActor};
use crate::config::ClientConfig;
use crate::executor::ExecutorLogic;
use crate::load_balancer::{LoadBalancer, LoadBalancerConfig};
use crate::resources::SharedResources;
use crate::types::AccountUpdate;
use ractor::{Actor, ActorProcessingErr, ActorRef};
use signal_hook::consts::signal::{SIGINT, SIGTERM};
use signal_hook_tokio::Signals;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::read_keypair_file;
use solana_sdk::signer::Signer;
use std::error::Error;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Default)]
pub struct RootSupervisor;

pub struct RootState {
    #[allow(dead_code)] // Kept for future observability control
    observability_ref: Option<ActorRef<ObservabilityMessage>>,
}

#[ractor::async_trait]
impl Actor for RootSupervisor {
    type Msg = RootMessage;
    type State = RootState;
    type Arguments = (
        ClientConfig,
        SharedResources,
        Option<mpsc::Receiver<AccountUpdate>>,
        mpsc::UnboundedReceiver<Pubkey>, // Cache eviction receiver for StagingActor
    );

    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        (config, resources, geyser_receiver, eviction_rx): Self::Arguments,
    ) -> Result<Self::State, Box<dyn Error + Send + Sync>> {
        log::debug!("RootSupervisor starting...");

        // Spawn signal handler task
        spawn_signal_handler(myself.clone());

        // Load executor keypair
        let keypair_path = shellexpand::tilde(&config.executor.keypair_path).to_string();
        log::debug!("Loading executor keypair from: {}", keypair_path);
        let keypair = read_keypair_file(&keypair_path)
            .map_err(|e| format!("Failed to load executor keypair from {}: {}", keypair_path, e))?;
        let executor_pubkey = keypair.pubkey();
        log::info!("Executor pubkey: {}", executor_pubkey);

        // Create ExecutorLogic
        let executor = ExecutorLogic::new(
            Arc::new(keypair),
            resources.clone(),
            config.executor.forgo_commission,
        );

        // Create LoadBalancer
        let load_balancer_config = LoadBalancerConfig::default();
        let load_balancer = Arc::new(LoadBalancer::new(executor_pubkey, load_balancer_config));

        // Spawn StagingActor first (others depend on it)
        log::debug!("Spawning StagingActor...");
        let (staging_ref, _staging_handle) = Actor::spawn(
            Some("staging-actor".to_string()),
            StagingActor,
            (config.clone(), resources.clone(), eviction_rx),
        )
        .await
        .map_err(|e| format!("Failed to spawn StagingActor: {}", e))?;

        // Spawn ProcessorFactory (depends on staging)
        log::debug!("Spawning ProcessorFactory...");
        let (processor_ref, _processor_handle) = Actor::spawn(
            Some("processor-factory".to_string()),
            ProcessorFactory,
            (
                config.clone(),
                resources.clone(),
                staging_ref.clone(),
                executor,
                load_balancer.clone(),
            ),
        )
        .await
        .map_err(|e| format!("Failed to spawn ProcessorFactory: {}", e))?;

        // Set processor ref in staging actor
        staging_ref
            .send_message(StagingMessage::SetProcessorRef(processor_ref.clone()))
            .map_err(|e| format!("Failed to set processor ref in staging: {}", e))?;

        // Spawn DatasourceSupervisor (depends on staging)
        // Pass optional geyser receiver for plugin mode
        log::debug!("Spawning DatasourceSupervisor...");
        let (_datasource_ref, _datasource_handle) = Actor::spawn(
            Some("datasource-supervisor".to_string()),
            DatasourceSupervisor,
            (
                config.clone(),
                resources.clone(),
                staging_ref.clone(),
                geyser_receiver,
            ),
        )
        .await
        .map_err(|e| format!("Failed to spawn DatasourceSupervisor: {}", e))?;

        log::debug!("All actors spawned successfully");

        // Spawn ObservabilityActor if enabled
        let observability_ref = if config.observability.enabled {
            log::debug!("Spawning ObservabilityActor...");
            let (obs_ref, _obs_handle) = Actor::spawn(
                Some("observability".to_string()),
                ObservabilityActor,
                config.observability.clone(),
            )
            .await
            .map_err(|e| format!("Failed to spawn ObservabilityActor: {}", e))?;
            Some(obs_ref)
        } else {
            log::debug!("Observability disabled, skipping loa-core agent");
            None
        };

        log::info!("System ready. Press Ctrl+C to shutdown.");

        Ok(RootState {
            observability_ref,
        })
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            RootMessage::Shutdown => {
                log::info!("RootSupervisor received shutdown signal");
                log::info!("Shutting down...");

                // Stop this actor (triggers post_stop)
                // Child actors will be automatically stopped by ractor's supervisor tree
                Err(From::from("Shutdown signal received"))
            }
        }
    }

    async fn post_stop(
        &self,
        _myself: ActorRef<Self::Msg>,
        _state: &mut Self::State,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        log::info!("RootSupervisor stopped. Graceful shutdown complete.");
        Ok(())
    }
}

/// Spawn a background task to listen for SIGINT and SIGTERM signals
fn spawn_signal_handler(root: ActorRef<RootMessage>) {
    tokio::spawn(async move {
        let mut signals = match Signals::new(&[SIGINT, SIGTERM]) {
            Ok(s) => s,
            Err(e) => {
                log::error!("Failed to create signal handler: {}", e);
                return;
            }
        };

        use futures::stream::StreamExt;
        if let Some(signal) = signals.next().await {
            let signal_name = match signal {
                SIGINT => "SIGINT (Ctrl+C)",
                SIGTERM => "SIGTERM",
                _ => "Unknown",
            };
            log::warn!("Received {} signal, initiating graceful shutdown...", signal_name);

            if let Err(e) = root.send_message(RootMessage::Shutdown) {
                log::error!("Failed to send shutdown message: {:?}", e);
            }
        }
    });
}
