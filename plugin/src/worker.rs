use antegen_executor::ExecutorService;
use antegen_observer::{EventSource, ObservedEvent, ObserverService};
use antegen_thread_program::state::Thread;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use log::{debug, error, info};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_program::{clock::Clock, pubkey::Pubkey};
use solana_sdk::{
    signature::read_keypair_file,
    signer::{keypair::Keypair, Signer},
};
use std::fmt::Debug;
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::sync::mpsc::{self, Receiver, Sender};

/// Worker that runs both observer and executor in the plugin
pub struct PluginWorker {
    /// Channel to send ObservedEvents from Geyser to Observer
    event_sender: Sender<ObservedEvent>,
    /// Observer service (owned until start is called)
    observer_service: Option<ObserverService>,
    /// Executor service (owned until start is called)  
    executor_service: Option<ExecutorService>,
}

impl Debug for PluginWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginWorker")
            .field("event_sender", &"Sender<ObservedEvent>")
            .field("observer_service", &self.observer_service.is_some())
            .field("executor_service", &self.executor_service.is_some())
            .finish()
    }
}

impl PluginWorker {
    pub async fn new(
        observer_id: u32,
        rpc_url: String,
        ws_url: String,
        keypair_path: String,
    ) -> Result<Self> {
        info!("=== Initializing PluginWorker ===");
        info!("Observer ID: {}", observer_id);
        info!("RPC URL: {}", rpc_url);
        info!("WS URL: {}", ws_url);

        // Channel 1: Geyser -> Observer (observed events)
        let (observed_tx, observed_rx) = mpsc::channel(1000);
        info!("Created observed event channel (Geyser->Observer) with capacity 1000");

        // Create Geyser data source that receives observed events
        let event_source = Box::new(GeyserPluginEventSource::new(observed_rx));
        info!("Created GeyserPluginEventSource for observer");

        // Create RPC client
        let rpc_client = Arc::new(RpcClient::new(rpc_url.clone()));

        // Load keypair
        let keypair = load_keypair(&keypair_path)?;
        let keypair = Arc::new(keypair);
        let observer_pubkey = keypair.pubkey();
        info!(
            "Loaded keypair from {}, pubkey: {}",
            keypair_path, observer_pubkey
        );

        // Create observer service with event source
        // This returns the service and a receiver for executor events
        let (observer_service, executor_event_rx) =
            ObserverService::new(event_source, observer_pubkey, rpc_client.clone());
        info!("Created observer service");

        // Create executor service that receives events from observer
        let executor_service = ExecutorService::new_with_observer(
            rpc_client,
            keypair.clone(),
            executor_event_rx,
            None, // TPU client config
        )
        .await?;
        info!("Created executor service");

        info!("=== PluginWorker initialization complete ===");

        Ok(Self {
            event_sender: observed_tx,
            observer_service: Some(observer_service),
            executor_service: Some(executor_service),
        })
    }

    /// Start the observer and executor services
    pub fn start(&mut self, runtime: Handle) -> Result<()> {
        info!("=== Starting Worker Services ===");

        // Take ownership of the services
        let mut observer_service = self
            .observer_service
            .take()
            .ok_or_else(|| anyhow!("Observer service already started"))?;
        let mut executor_service = self
            .executor_service
            .take()
            .ok_or_else(|| anyhow!("Executor service already started"))?;

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

        info!("Spawning executor service task...");

        // Spawn executor service
        runtime.spawn(async move {
            info!("EXECUTOR: Task started, entering event loop");
            match executor_service.run().await {
                Ok(()) => info!("EXECUTOR: Service completed normally"),
                Err(e) => error!("EXECUTOR: Service stopped with error: {}", e),
            }
            info!("EXECUTOR: Task exiting");
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
