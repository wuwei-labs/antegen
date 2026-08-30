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
use ractor::{Actor, ActorProcessingErr, ActorRef, SupervisionEvent};
use signal_hook::consts::signal::{SIGINT, SIGTERM};
use signal_hook_tokio::Signals;
use solana_keypair::read_keypair_file;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Name of the observability child, used both to spawn it and to recognise it
/// in supervision events.
const OBSERVABILITY_ACTOR: &str = "observability";

/// Children whose death must not take the node down.
///
/// Every other child is load-bearing: losing staging, the processor or the
/// datasources means threads stop executing, and stopping is the honest
/// response. Telemetry is not in that category — the node's job is executing
/// threads, and it can do that with the agent dead.
fn is_non_essential(name: &str) -> bool {
    name == OBSERVABILITY_ACTOR
}

#[derive(Default)]
pub struct RootSupervisor;

pub struct RootState {
    #[allow(dead_code)] // Kept for future observability control
    observability_ref: Option<ActorRef<ObservabilityMessage>>,
    /// Raised before stopping on a child failure, so the caller can exit
    /// non-zero. A shutdown requested by a signal leaves it clear.
    fatal: Arc<AtomicBool>,
}

impl Actor for RootSupervisor {
    type Msg = RootMessage;
    type State = RootState;
    type Arguments = (
        ClientConfig,
        SharedResources,
        Option<mpsc::Receiver<AccountUpdate>>,
        mpsc::UnboundedReceiver<Pubkey>, // Cache eviction receiver for StagingActor
        // Set when the supervisor stops because something failed rather than
        // because it was asked to. The caller turns it into a non-zero exit,
        // which is what a process supervisor needs to see.
        Arc<AtomicBool>,
    );

    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        (config, resources, geyser_receiver, eviction_rx, fatal): Self::Arguments,
    ) -> Result<Self::State, Box<dyn Error + Send + Sync>> {
        log::debug!("RootSupervisor starting...");

        // Spawn signal handler task
        spawn_signal_handler(myself.clone());

        // Load executor keypair
        let keypair_path = shellexpand::tilde(&config.executor.keypair_path).to_string();
        log::debug!("Loading executor keypair from: {}", keypair_path);
        let keypair = read_keypair_file(&keypair_path).map_err(|e| {
            format!(
                "Failed to load executor keypair from {}: {}",
                keypair_path, e
            )
        })?;
        let executor_pubkey = keypair.pubkey();
        log::info!("Executor pubkey: {}", executor_pubkey);

        // Create ExecutorLogic
        let executor = ExecutorLogic::new(
            Arc::new(keypair),
            resources.clone(),
            config.executor.forgo_commission,
        )
        .with_tx_version(config.transaction.version);

        // Create LoadBalancer with config values
        let load_balancer_config = LoadBalancerConfig {
            enabled: true,
            capacity_threshold: 5,
            thread_takeover_delay: config.load_balancer.grace_period as i64,
            thread_process_delay: config.load_balancer.thread_process_delay,
        };
        let load_balancer = Arc::new(LoadBalancer::new(executor_pubkey, load_balancer_config));

        let supervisor = myself.get_cell();

        // Spawn StagingActor first (others depend on it)
        log::debug!("Spawning StagingActor...");
        let (staging_ref, _staging_handle) = Actor::spawn_linked(
            Some("staging-actor".to_string()),
            StagingActor,
            (
                config.clone(),
                resources.clone(),
                load_balancer.clone(),
                eviction_rx,
            ),
            supervisor.clone(),
        )
        .await
        .map_err(|e| format!("Failed to spawn StagingActor: {}", e))?;

        // Spawn ProcessorFactory (depends on staging)
        log::debug!("Spawning ProcessorFactory...");
        let (processor_ref, _processor_handle) = Actor::spawn_linked(
            Some("processor-factory".to_string()),
            ProcessorFactory,
            (
                config.clone(),
                resources.clone(),
                staging_ref.clone(),
                executor,
                load_balancer.clone(),
            ),
            supervisor.clone(),
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
        let (_datasource_ref, _datasource_handle) = Actor::spawn_linked(
            Some("datasource-supervisor".to_string()),
            DatasourceSupervisor,
            (
                config.clone(),
                resources.clone(),
                staging_ref.clone(),
                geyser_receiver,
                executor_pubkey,
            ),
            supervisor.clone(),
        )
        .await
        .map_err(|e| format!("Failed to spawn DatasourceSupervisor: {}", e))?;

        log::debug!("All actors spawned successfully");

        // Spawn ObservabilityActor if enabled.
        //
        // A failure here is logged and stepped over rather than propagated.
        // Telemetry is not part of executing threads, and loa-core reaches the
        // network during startup — it registers with a backend and opens a
        // tunnel — so anything from restricted egress to the backend being down
        // can stop it. A mainnet node that will not start because a metrics
        // agent could not phone home is strictly worse than one running blind:
        // loa-core 3.0 panicked in `Runtime::start()` on a mainnet host and
        // crash-looped the executor thirteen times before the operator disabled
        // observability by hand.
        let observability_ref = if config.observability.enabled {
            log::debug!("Spawning ObservabilityActor...");
            match Actor::spawn_linked(
                Some(OBSERVABILITY_ACTOR.to_string()),
                ObservabilityActor,
                config.observability.clone(),
                supervisor.clone(),
            )
            .await
            {
                Ok((obs_ref, _obs_handle)) => Some(obs_ref),
                Err(e) => {
                    log::warn!(
                        "Observability agent failed to start ({}). Continuing without telemetry; \
                         set observability.enabled = false to silence this.",
                        e
                    );
                    None
                }
            }
        } else {
            log::debug!("Observability disabled, skipping loa-core agent");
            None
        };

        log::info!("System ready. Press Ctrl+C to shutdown.");

        Ok(RootState {
            observability_ref,
            fatal,
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

    async fn handle_supervisor_evt(
        &self,
        myself: ActorRef<Self::Msg>,
        message: SupervisionEvent,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            SupervisionEvent::ActorTerminated(who, _, reason) => {
                let name = who.get_name().unwrap_or_default();
                if is_non_essential(&name) {
                    log::warn!(
                        "Child actor {} terminated (reason: {:?}). Continuing without it.",
                        name,
                        reason
                    );
                    return Ok(());
                }
                log::error!(
                    "Child actor {} terminated (reason: {:?}). Shutting down system.",
                    name,
                    reason
                );
                state.fatal.store(true, Ordering::SeqCst);
                myself.stop(None);
            }
            SupervisionEvent::ActorFailed(who, error) => {
                let name = who.get_name().unwrap_or_default();
                if is_non_essential(&name) {
                    log::warn!(
                        "Child actor {} failed: {}. Continuing without it.",
                        name,
                        error
                    );
                    return Ok(());
                }
                log::error!(
                    "Child actor {} failed: {}. Shutting down system.",
                    name,
                    error
                );
                state.fatal.store(true, Ordering::SeqCst);
                myself.stop(None);
            }
            _ => {}
        }
        Ok(())
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
        let mut signals = match Signals::new([SIGINT, SIGTERM]) {
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
            log::warn!(
                "Received {} signal, initiating graceful shutdown...",
                signal_name
            );

            if let Err(e) = root.send_message(RootMessage::Shutdown) {
                log::error!("Failed to send shutdown message: {:?}", e);
            }
        }
    });
}
