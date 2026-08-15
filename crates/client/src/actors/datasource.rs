//! Datasource Supervisor and Source Actors
//!
//! The DatasourceSupervisor dynamically spawns source actors based on configuration:
//! - RpcSourceActor: Listens to WebSocket streams for account updates
//! - GeyserSourceActor: Consumes mpsc channel from Geyser plugin
//!
//! All source actors push updates through the shared cache for deduplication
//! before forwarding to StagingActor.

use crate::actors::messages::{
    DatasourceMessage, GeyserSourceMessage, RpcSourceMessage, StagingMessage,
};
use crate::config::{ClientConfig, EndpointRole, RpcEndpoint};
use crate::datasources::RpcSubscription;
use crate::resources::SharedResources;
use crate::types::AccountUpdate;
use ractor::{Actor, ActorProcessingErr, ActorRef, SupervisionEvent};
use std::collections::HashMap;
use std::error::Error;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

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
        myself: ActorRef<Self::Msg>,
        (config, resources, staging_ref, geyser_receiver): Self::Arguments,
    ) -> Result<Self::State, Box<dyn Error + Send + Sync>> {
        log::debug!("DatasourceSupervisor starting...");

        let supervisor = myself.get_cell();
        let mut rpc_sources = HashMap::new();
        let mut datasource_count = 0;

        // Spawn RpcSourceActor for each datasource endpoint (linked to this supervisor)
        for endpoint in &config.rpc.endpoints {
            if matches!(endpoint.role, EndpointRole::Datasource | EndpointRole::Both) {
                let actor_name = format!("rpc-source-{}", endpoint.url);

                log::debug!("Spawning RpcSourceActor for: {}", endpoint.url);

                let (rpc_ref, _handle) = Actor::spawn_linked(
                    Some(actor_name.clone()),
                    RpcSourceActor,
                    (endpoint.clone(), resources.clone(), staging_ref.clone()),
                    supervisor.clone(),
                )
                .await
                .map_err(|e| format!("Failed to spawn RpcSourceActor: {}", e))?;

                rpc_sources.insert(actor_name, rpc_ref);
                datasource_count += 1;
            }
        }

        log::debug!("Spawned {} RPC datasource actors", datasource_count);

        // Optionally spawn GeyserSourceActor if we have a channel from the plugin (linked)
        let geyser_source = if let Some(receiver) = geyser_receiver {
            log::info!("Spawning GeyserSourceActor for plugin mode");

            let (geyser_ref, _handle) = Actor::spawn_linked(
                Some("geyser-source".to_string()),
                GeyserSourceActor,
                (receiver, resources.clone(), staging_ref.clone()),
                supervisor.clone(),
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

    /// Losing one datasource must not take down the node.
    ///
    /// ractor's default supervisor behaviour is to stop on any child exit, which
    /// propagates to the root supervisor and shuts the process down. That makes
    /// configuring redundant datasources actively harmful: any one endpoint
    /// flapping kills a node that had N-1 healthy sources still delivering.
    /// Escalate only when nothing is left to listen with.
    async fn handle_supervisor_evt(
        &self,
        myself: ActorRef<Self::Msg>,
        message: SupervisionEvent,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        let (who, why) = match message {
            SupervisionEvent::ActorTerminated(who, _, reason) => {
                (who, reason.unwrap_or_else(|| "terminated".to_string()))
            }
            SupervisionEvent::ActorFailed(who, error) => (who, error.to_string()),
            _ => return Ok(()),
        };

        let name = who.get_name().unwrap_or_default();
        state.rpc_sources.remove(&name);
        if state
            .geyser_source
            .as_ref()
            .is_some_and(|r| r.get_name().unwrap_or_default() == name)
        {
            state.geyser_source = None;
        }

        let rpc_live = state.rpc_sources.len();
        let total_live = rpc_live + usize::from(state.geyser_source.is_some());

        if total_live == 0 {
            log::error!(
                "Datasource {} down ({}). No datasources remain — escalating.",
                name,
                why
            );
            myself.stop(None);
            return Ok(());
        }

        log::warn!(
            "Datasource {} down ({}). {} datasource(s) still live.",
            name,
            why,
            total_live
        );

        if rpc_live == 0 {
            // Scheduling ticks come from the clock subscription on an RPC
            // datasource, so losing all of them stops execution even though a
            // Geyser feed is still delivering account updates.
            log::error!(
                "No RPC datasources remain — clock ticks have stopped, threads will not fire"
            );
        }

        Ok(())
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

/// Maximum subscription restarts allowed within `RESTART_WINDOW` before the
/// actor gives up on this endpoint.
const MAX_SUBSCRIPTION_RESTARTS: u32 = 3;

/// Window over which restarts are counted. Without a window this is a *lifetime*
/// budget: a connection that flaps three times over days of uptime is treated
/// the same as one flapping three times a minute, and the endpoint is abandoned
/// despite being healthy in between.
const RESTART_WINDOW: Duration = Duration::from_secs(300);

/// Rate limiter for subscription restarts.
#[derive(Debug, Default)]
struct RestartBudget {
    count: u32,
    window_start: Option<Instant>,
}

impl RestartBudget {
    /// Record a restart. Returns the count within the current window.
    fn record(&mut self) -> u32 {
        let now = Instant::now();
        match self.window_start {
            Some(start) if now.duration_since(start) <= RESTART_WINDOW => {
                self.count += 1;
            }
            _ => {
                // First restart, or the previous window has aged out — the
                // subscription was healthy for long enough to earn a fresh
                // budget.
                self.window_start = Some(now);
                self.count = 1;
            }
        }
        self.count
    }

    fn exhausted(&self) -> bool {
        self.count > MAX_SUBSCRIPTION_RESTARTS
    }
}

pub struct RpcSourceState {
    ws_url: String,
    staging_ref: ActorRef<StagingMessage>,
    resources: SharedResources,
    cancel_token: CancellationToken,
    program_restarts: RestartBudget,
    clock_restarts: RestartBudget,
    /// Guards against a reconnect storm stacking up backfills.
    backfill_in_flight: bool,
}

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
        log::debug!(
            "RpcSourceActor starting for: {} (ws: {})",
            endpoint.url,
            ws_url
        );
        log::debug!("  - Thread program: {}", resources.program_id);
        log::debug!("  - Clock sysvar: {}", solana_sdk::sysvar::clock::ID);

        let cancel_token = CancellationToken::new();

        // Spawn monitored subscription tasks
        spawn_program_subscription(&ws_url, &resources, myself.clone(), cancel_token.clone());
        spawn_clock_subscription(&ws_url, &resources, myself.clone(), cancel_token.clone());

        Ok(RpcSourceState {
            ws_url,
            staging_ref,
            resources,
            cancel_token,
            program_restarts: RestartBudget::default(),
            clock_restarts: RestartBudget::default(),
            backfill_in_flight: false,
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
                // Returns true only if this is new/newer data. With several
                // datasources racing, this is the arbitration point: the first
                // endpoint to deliver a given state wins and the rest are
                // dropped here.
                let is_new = state
                    .resources
                    .cache
                    .put_if_newer(update.pubkey, update.data.clone(), update.slot)
                    .await;

                state
                    .resources
                    .ingest_stats
                    .record_account(&state.ws_url, is_new);

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

                // Attribute the clock race. Whichever endpoint raises the slot
                // high-water mark first is the one actually driving scheduling;
                // the others are paying for a subscription that adds nothing.
                if let Some(lag) = state.resources.ingest_stats.record_clock(
                    &state.ws_url,
                    clock.slot,
                    Instant::now(),
                ) {
                    log::trace!(
                        "[{}] lost clock race for slot {} by {}ms",
                        state.ws_url,
                        clock.slot,
                        lag.as_millis()
                    );
                }

                // Clock is NOT cached - always forward fresh to StagingActor
                state
                    .staging_ref
                    .send_message(StagingMessage::ClockTick(clock))
                    .map_err(|e| format!("Failed to send clock to staging: {}", e))?;

                Ok(())
            }
            RpcSourceMessage::Reconnected => {
                // Backfill is an unpaginated getProgramAccounts — seconds on a
                // large program. Awaiting it here would hold this actor's
                // mailbox for the whole call, stalling every clock tick and
                // account update from this endpoint. Run it off the actor loop;
                // it already reports results back by message.
                if state.backfill_in_flight {
                    log::debug!("[{}] Backfill already in flight, skipping", state.ws_url);
                    return Ok(());
                }

                // Debounced across every datasource, not just this one. With
                // redundant endpoints each reconnect would otherwise trigger its
                // own full program scan, and all but the first are discarded as
                // duplicates by the cache.
                if !state.resources.ingest_stats.try_claim_backfill() {
                    log::debug!(
                        "[{}] Another datasource backfilled recently, skipping",
                        state.ws_url
                    );
                    return Ok(());
                }
                state.backfill_in_flight = true;

                let subscription = RpcSubscription::new(
                    state.ws_url.clone(),
                    state.resources.program_id,
                    state.resources.rpc_client.clone(),
                    state.resources.ingest_stats.clone(),
                );
                let ws_url = state.ws_url.clone();
                let actor_ref = myself.clone();
                tokio::spawn(async move {
                    if let Err(e) = subscription.perform_backfill(actor_ref.clone()).await {
                        log::error!("[{}] Backfill failed: {}", ws_url, e);
                    }
                    let _ = actor_ref.send_message(RpcSourceMessage::BackfillFinished);
                });

                Ok(())
            }
            RpcSourceMessage::BackfillFinished => {
                state.backfill_in_flight = false;
                Ok(())
            }
            RpcSourceMessage::SubscriptionDied(which) => {
                // A subscription background task has exited — restart it if the
                // endpoint has not been flapping.
                let (budget, limit_name) = match which.as_str() {
                    "program" => (&mut state.program_restarts, "program"),
                    "clock" => (&mut state.clock_restarts, "clock"),
                    other => {
                        log::warn!("[{}] Unknown subscription died: {}", state.ws_url, other);
                        return Ok(());
                    }
                };

                let count = budget.record();
                log::warn!(
                    "[{}] {} subscription died (restart {}/{} within {}s)",
                    state.ws_url,
                    limit_name,
                    count,
                    MAX_SUBSCRIPTION_RESTARTS,
                    RESTART_WINDOW.as_secs()
                );

                if budget.exhausted() {
                    log::error!(
                        "[{}] {} subscription is flapping, giving up on this endpoint",
                        state.ws_url,
                        limit_name
                    );
                    // Stopping this actor takes down only this datasource. The
                    // supervisor decides whether any remain.
                    return Err(From::from(format!(
                        "{} subscription exceeded restart budget",
                        limit_name
                    )));
                }

                // Re-spawn the dead subscription
                match which.as_str() {
                    "program" => {
                        spawn_program_subscription(
                            &state.ws_url,
                            &state.resources,
                            myself.clone(),
                            state.cancel_token.clone(),
                        );
                    }
                    "clock" => {
                        spawn_clock_subscription(
                            &state.ws_url,
                            &state.resources,
                            myself.clone(),
                            state.cancel_token.clone(),
                        );
                    }
                    _ => {}
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
        // Cancel all background subscription tasks so they exit cleanly
        state.cancel_token.cancel();
        log::info!("RpcSourceActor for {} stopped", state.ws_url);
        Ok(())
    }
}

/// Spawn a monitored program subscription task.
/// When the subscription task exits (for any reason), a watcher task
/// sends `SubscriptionDied("program")` to the actor so it can restart.
fn spawn_program_subscription(
    ws_url: &str,
    resources: &SharedResources,
    actor_ref: ActorRef<RpcSourceMessage>,
    cancel_token: CancellationToken,
) {
    let program_ws_url = ws_url.to_string();
    let program_id = resources.program_id;
    let rpc_client = resources.rpc_client.clone();
    let ingest_stats = resources.ingest_stats.clone();
    let sub_actor_ref = actor_ref.clone();

    let handle = tokio::spawn(async move {
        let subscription =
            RpcSubscription::new(program_ws_url, program_id, rpc_client, ingest_stats);
        tokio::select! {
            _ = subscription.subscribe_to_program_accounts(sub_actor_ref) => {}
            _ = cancel_token.cancelled() => {
                log::debug!("Program subscription cancelled");
            }
        }
    });

    // Watcher: notify the actor when the subscription task exits
    tokio::spawn(async move {
        let _ = handle.await;
        let _ = actor_ref.send_message(RpcSourceMessage::SubscriptionDied("program".to_string()));
    });
}

/// Spawn a monitored clock subscription task.
/// Same pattern as `spawn_program_subscription`.
fn spawn_clock_subscription(
    ws_url: &str,
    resources: &SharedResources,
    actor_ref: ActorRef<RpcSourceMessage>,
    cancel_token: CancellationToken,
) {
    let clock_ws_url = ws_url.to_string();
    let program_id = resources.program_id;
    let rpc_client = resources.rpc_client.clone();
    let ingest_stats = resources.ingest_stats.clone();
    let sub_actor_ref = actor_ref.clone();

    let handle = tokio::spawn(async move {
        let subscription = RpcSubscription::new(clock_ws_url, program_id, rpc_client, ingest_stats);
        tokio::select! {
            _ = subscription.subscribe_to_clock(sub_actor_ref) => {}
            _ = cancel_token.cancelled() => {
                log::debug!("Clock subscription cancelled");
            }
        }
    });

    // Watcher: notify the actor when the subscription task exits
    tokio::spawn(async move {
        let _ = handle.await;
        let _ = actor_ref.send_message(RpcSourceMessage::SubscriptionDied("clock".to_string()));
    });
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
    cancel_token: CancellationToken,
}

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

        let cancel_token = CancellationToken::new();

        // Spawn task to consume the channel
        let cache = resources.cache.clone();
        let staging = staging_ref.clone();
        let actor_ref = myself.clone();
        let task_token = cancel_token.clone();

        tokio::spawn(async move {
            log::info!("GeyserSourceActor channel consumer started");

            loop {
                tokio::select! {
                    update = receiver.recv() => {
                        let Some(update) = update else {
                            break; // Channel closed
                        };

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
                    _ = task_token.cancelled() => {
                        log::debug!("GeyserSourceActor channel consumer cancelled");
                        break;
                    }
                }
            }

            log::info!("GeyserSourceActor channel consumer stopped");

            // Signal actor to stop when channel closes
            let _ = actor_ref.send_message(GeyserSourceMessage::Shutdown);
        });

        Ok(GeyserSourceState {
            staging_ref,
            resources,
            cancel_token,
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
        state: &mut Self::State,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Cancel the background channel consumer task so it exits cleanly
        state.cancel_token.cancel();
        log::info!("GeyserSourceActor stopped");
        Ok(())
    }
}
