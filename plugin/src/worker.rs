use antegen_observer::{EventSource, ObservedEvent, ObserverService};
use antegen_submitter::SubmitterService;
use antegen_thread_program::state::Thread;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use log::{debug, error, info};
use solana_program::{clock::Clock, pubkey::Pubkey};
use solana_sdk::{
    signature::read_keypair_file,
    signer::{keypair::Keypair, Signer},
};
use std::fmt::Debug;
use tokio::runtime::Handle;
use tokio::sync::mpsc::{self, Receiver, Sender};

/// Worker that runs observer and submitter services in the plugin
pub struct PluginWorker {
    /// Channel to send ObservedEvents from Geyser to Observer
    event_sender: Sender<ObservedEvent>,
    /// Observer service (owned until start is called)
    observer_service: Option<ObserverService>,
    /// Submitter service for transaction submission and executor logic
    submitter_service: Option<SubmitterService>,
}

impl Debug for PluginWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginWorker")
            .field("event_sender", &"Sender<ObservedEvent>")
            .field("observer_service", &self.observer_service.is_some())
            .field("submitter_service", &self.submitter_service.is_some())
            .finish()
    }
}

impl PluginWorker {
    pub async fn new(
        rpc_url: String,
        ws_url: String,
        keypair_path: String,
        forgo_executor_commission: bool,
        enable_replay: bool,
        nats_url: Option<String>,
        replay_delay_ms: Option<u64>,
    ) -> Result<Self> {
        info!("=== Initializing PluginWorker ===");
        info!("RPC URL: {}", rpc_url);
        info!("WS URL: {}", ws_url);

        // Channel 1: Geyser -> Observer (observed events)
        let (observed_tx, observed_rx) = mpsc::channel(1000);
        info!("Created observed event channel (Geyser->Observer) with capacity 1000");

        // Create Geyser data source that receives observed events
        let event_source = Box::new(GeyserPluginEventSource::new(observed_rx));
        info!("Created GeyserPluginEventSource for observer");


        // Load keypair
        let keypair = load_keypair(&keypair_path)?;
        let observer_pubkey = keypair.pubkey();
        info!(
            "Loaded keypair from {}, pubkey: {}",
            keypair_path, observer_pubkey
        );

        // Create observer service with event source
        // This returns the service and receivers for executable threads, clock updates, and account updates
        let (observer_service, thread_receiver, clock_receiver, account_receiver) =
            ObserverService::new(event_source, observer_pubkey);
        info!("Created observer service with thread, clock, and account event channels");

        // Create submitter service with integrated executor functionality
        // Start with defaults to ensure all fields are present
        let mut submitter_config = antegen_submitter::SubmitterConfig::default();
        
        // Override specific fields for plugin operation
        submitter_config.executor_keypair_path = Some(keypair_path.clone()); // Enable full mode
        submitter_config.rpc_url = rpc_url.clone();
        submitter_config.forgo_executor_commission = forgo_executor_commission;
        submitter_config.enable_replay = enable_replay;
        submitter_config.nats_url = nats_url;
        submitter_config.replay_delay_ms = replay_delay_ms.unwrap_or(30_000); // Default 30s
        submitter_config.replay_max_age_ms = 3600_000; // 1 hour
        submitter_config.replay_max_attempts = 3;
        // Keep defaults for: simulate_before_submit, compute_unit_multiplier, 
        // max_compute_units, max_concurrent_threads, tpu_config, submission_mode
        
        let mut submitter_service = SubmitterService::new(submitter_config).await?;
        
        // Set all receivers from observer
        submitter_service.set_thread_receiver(thread_receiver)?;
        submitter_service.set_clock_receiver(clock_receiver)?;
        submitter_service.set_account_receiver(account_receiver)?;
        info!("Created submitter service with integrated executor and all channels");

        info!("=== PluginWorker initialization complete ===");

        Ok(Self {
            event_sender: observed_tx,
            observer_service: Some(observer_service),
            submitter_service: Some(submitter_service),
        })
    }

    /// Start the observer and submitter services
    pub fn start(&mut self, runtime: Handle) -> Result<()> {
        info!("=== Starting Worker Services ===");

        // Take ownership of the services
        let mut observer_service = self
            .observer_service
            .take()
            .ok_or_else(|| anyhow!("Observer service already started"))?;
        let submitter_service = self
            .submitter_service
            .take()
            .ok_or_else(|| anyhow!("Submitter service already started"))?;

        info!("Spawning observer service task...");

        // Spawn observer service
        runtime.spawn(async move {
            info!("OBSERVER: Task started, entering event loop");
            match observer_service.run().await {
                Ok(()) => info!("OBSERVER: Service completed normally"),
                Err(e) => error!("OBSERVER: Service stopped with error: {}", e),
            }
            info!("OBSERVER: Task exiting");
        });

        info!("Spawning submitter service task (with integrated executor)...");

        // Spawn submitter service (now includes executor logic)
        let mut submitter_service = submitter_service;
        
        runtime.spawn(async move {
            info!("SUBMITTER: Task started in full mode, entering event loop");
            match submitter_service.run().await {
                Ok(()) => info!("SUBMITTER: Service completed normally"),
                Err(e) => error!("SUBMITTER: Service stopped with error: {}", e),
            }
            info!("SUBMITTER: Task exiting");
        });

        info!("=== Worker Services Started ===");
        Ok(())
    }

    /// Send thread event from Geyser to observer
    pub async fn send_thread_event(
        &self,
        thread: Thread,
        thread_pubkey: Pubkey,
        slot: u64,
    ) -> Result<()> {
        info!(
            "GEYSER->OBSERVER: Thread event for {} at slot {}",
            thread_pubkey, slot
        );

        // Skip paused threads
        if thread.paused {
            info!(
                "GEYSER->OBSERVER: Thread {} is paused, skipping",
                thread_pubkey
            );
            return Ok(());
        }

        // Send thread as executable to observer
        let event = ObservedEvent::ThreadExecutable {
            thread_pubkey,
            thread: thread.clone(),
            slot,
        };

        info!(
            "GEYSER->OBSERVER: Sending ThreadExecutable event for {}",
            thread_pubkey
        );
        match self.event_sender.send(event).await {
            Ok(()) => {
                info!(
                    "GEYSER->OBSERVER: Successfully sent thread event for {}",
                    thread_pubkey
                );
            }
            Err(e) => {
                error!("GEYSER->OBSERVER: Failed to send thread event: {}", e);
                return Err(anyhow!("Failed to send thread event: {}", e));
            }
        }

        Ok(())
    }

    /// Send clock update to observer (which will forward to executor)
    pub async fn send_clock_event(
        &self,
        clock: Clock,
        slot: u64,
        _block_height: u64,
    ) -> Result<()> {
        // Send clock update as an ObservedEvent
        let event = ObservedEvent::ClockUpdate {
            slot,
            epoch: clock.epoch,
            unix_timestamp: clock.unix_timestamp,
        };

        debug!("GEYSER->OBSERVER: Sending clock event slot={}", slot);
        match self.event_sender.send(event).await {
            Ok(()) => {
                debug!("GEYSER->OBSERVER: Clock event sent successfully");
            }
            Err(e) => {
                error!("GEYSER->OBSERVER: Failed to send clock event: {}", e);
                return Err(anyhow!("Failed to send clock event: {}", e));
            }
        }

        Ok(())
    }

    /// Send account update from Geyser to observer
    pub async fn send_account_event(
        &self,
        pubkey: Pubkey,
        account: solana_sdk::account::Account,
        slot: u64,
    ) -> Result<()> {
        debug!(
            "Forwarding AccountUpdate event to observer: pubkey={}, slot={}, data_len={}",
            pubkey,
            slot,
            account.data.len()
        );

        let event = ObservedEvent::AccountUpdate {
            pubkey,
            account,
            slot,
        };

        self.event_sender.send(event).await?;
        Ok(())
    }
}

/// Event source that receives events from the Geyser plugin
struct GeyserPluginEventSource {
    receiver: Receiver<ObservedEvent>,
    running: bool,
}

impl GeyserPluginEventSource {
    fn new(receiver: Receiver<ObservedEvent>) -> Self {
        Self {
            receiver,
            running: false,
        }
    }
}

#[async_trait]
impl EventSource for GeyserPluginEventSource {
    async fn start(&mut self) -> Result<()> {
        info!("Starting GeyserPluginEventSource");
        self.running = true;
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        info!("Stopping GeyserPluginEventSource");
        self.running = false;
        Ok(())
    }

    async fn next_event(&mut self) -> Result<Option<ObservedEvent>> {
        if !self.running {
            return Ok(None);
        }

        // Non-blocking receive
        match self.receiver.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => {
                error!("Event channel disconnected");
                self.running = false;
                Ok(None)
            }
        }
    }

    async fn subscribe_thread(&mut self, _thread_pubkey: Pubkey) -> Result<()> {
        // No-op - we receive all events from Geyser
        Ok(())
    }

    async fn unsubscribe_thread(&mut self, _thread_pubkey: Pubkey) -> Result<()> {
        // No-op - we receive all events from Geyser
        Ok(())
    }

    async fn get_current_slot(&self) -> Result<u64> {
        // This would need to be tracked from clock events
        Ok(0)
    }

    fn name(&self) -> &str {
        "GeyserPluginEventSource"
    }
}

fn load_keypair(path: &str) -> Result<Keypair> {
    let keypair =
        read_keypair_file(path).map_err(|e| anyhow!("Failed to read keypair file: {}", e))?;
    Ok(keypair)
}
