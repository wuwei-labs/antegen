use anyhow::Result;
use antegen_builder::{
    data_source::{DataSource, ObservedEvent},
    service::BuilderService,
};
use antegen_submitter::SubmitterService;
use antegen_thread_program::state::Thread;
use async_trait::async_trait;
use log::{error, info};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_program::{clock::Clock, pubkey::Pubkey};
use solana_sdk::signer::keypair::Keypair;
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::mpsc::{self, Receiver, Sender};

/// Worker that runs both builder and submitter in the plugin
pub struct PluginWorker {
    /// Channel to send events from Geyser to builder
    event_sender: Sender<ObservedEvent>,
}

impl Debug for PluginWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginWorker")
            .field("event_sender", &"Sender<ObservedEvent>")
            .finish()
    }
}

impl PluginWorker {
    pub async fn new(
        builder_id: u32,
        rpc_url: String,
        ws_url: String,
        keypair_path: String,
    ) -> Result<Self> {
        info!("Initializing PluginWorker with builder_id: {}", builder_id);

        // Create channel for Geyser -> Builder communication
        let (event_tx, event_rx) = mpsc::channel(1000);
        
        // Create Geyser data source that receives from plugin
        let data_source = Box::new(GeyserPluginDataSource::new(event_rx));
        
        // Create RPC client
        let rpc_client = Arc::new(RpcClient::new(rpc_url.clone()));
        
        // Create builder in worker mode (outputs to channel)
        let (mut builder_service, _tx_sender) = BuilderService::new_worker(
            data_source,
            builder_id,
            rpc_client,
        );
        
        // Load keypair
        let keypair = load_keypair(&keypair_path)?;
        let keypair = Arc::new(keypair);
        
        // Create submitter with local queue
        let (mut submitter_service, _tx_receiver) = SubmitterService::with_local_queue(
            rpc_url,
            ws_url,
            keypair,
            100, // buffer size
        );
        
        // Start the services in background tasks
        tokio::spawn(async move {
            if let Err(e) = builder_service.run().await {
                error!("Builder service stopped: {}", e);
            }
        });
        
        tokio::spawn(async move {
            if let Err(e) = submitter_service.run().await {
                error!("Submitter service stopped: {}", e);
            }
        });
        
        Ok(Self {
            event_sender: event_tx,
        })
    }
    
    
    /// Send thread event from Geyser to builder
    pub async fn send_thread_event(
        &self,
        thread: Thread,
        thread_pubkey: Pubkey,
        slot: u64,
    ) -> Result<()> {
        // Skip paused threads
        if thread.paused {
            return Ok(());
        }
        
        info!("Observed thread: {} at slot {}", thread_pubkey, slot);
        
        // Check if thread is potentially executable
        if thread.builders.is_empty() {
            let event = ObservedEvent::ThreadExecutable {
                thread_pubkey,
                thread,
                slot,
            };
            
            info!("Thread {} is executable, sending to builder", thread_pubkey);
            self.event_sender.send(event).await?;
        }
        Ok(())
    }
    
    /// Send clock update from Geyser to builder
    pub async fn send_clock_event(&self, clock: Clock, slot: u64) -> Result<()> {
        let event = ObservedEvent::ClockUpdate {
            slot,
            epoch: clock.epoch,
            unix_timestamp: clock.unix_timestamp,
        };
        
        self.event_sender.send(event).await?;
        Ok(())
    }
    
    /// Send account update from Geyser to builder
    pub async fn send_account_event(
        &self,
        pubkey: Pubkey,
        account: solana_sdk::account::Account,
        slot: u64,
    ) -> Result<()> {
        let event = ObservedEvent::AccountUpdate {
            pubkey,
            account,
            slot,
        };
        
        self.event_sender.send(event).await?;
        Ok(())
    }
}

/// Data source that receives events from the Geyser plugin
struct GeyserPluginDataSource {
    receiver: Receiver<ObservedEvent>,
    running: bool,
}

impl GeyserPluginDataSource {
    fn new(receiver: Receiver<ObservedEvent>) -> Self {
        Self {
            receiver,
            running: false,
        }
    }
}

#[async_trait]
impl DataSource for GeyserPluginDataSource {
    async fn start(&mut self) -> Result<()> {
        info!("Starting GeyserPluginDataSource");
        self.running = true;
        Ok(())
    }
    
    async fn stop(&mut self) -> Result<()> {
        info!("Stopping GeyserPluginDataSource");
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
        "GeyserPluginDataSource"
    }
}

fn load_keypair(path: &str) -> Result<Keypair> {
    let keypair_bytes = std::fs::read(path)?;
    let keypair = Keypair::from_bytes(&keypair_bytes)?;
    Ok(keypair)
}