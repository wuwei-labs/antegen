//! Observability Actor
//!
//! Wraps the loa-core agent and runs it within the antegen actor hierarchy.

use crate::config::ObservabilityConfig;
use loa_core::Agent;
use ractor::{Actor, ActorProcessingErr, ActorRef};
use std::error::Error;

/// Messages for the ObservabilityActor
pub enum ObservabilityMessage {
    Shutdown,
}

pub struct ObservabilityActor;

pub struct ObservabilityState {
    #[allow(dead_code)] // Agent kept alive to run loa-core's actor tree
    _agent: Agent,
}

#[ractor::async_trait]
impl Actor for ObservabilityActor {
    type Msg = ObservabilityMessage;
    type State = ObservabilityState;
    type Arguments = ObservabilityConfig;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        config: Self::Arguments,
    ) -> Result<Self::State, Box<dyn Error + Send + Sync>> {
        log::info!("ObservabilityActor starting...");

        // Expand the storage path
        let storage_path = shellexpand::tilde(&config.storage_path).to_string();
        log::debug!("Loa storage path: {}", storage_path);

        // Build and start the loa-core agent
        let agent = Agent::builder()
            .storage_path(&storage_path)
            .build()
            .await
            .map_err(|e| format!("Failed to build loa-core agent: {}", e))?;

        log::info!("Loa observability agent started");

        Ok(ObservabilityState { _agent: agent })
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            ObservabilityMessage::Shutdown => {
                log::info!("ObservabilityActor received shutdown signal");
                myself.stop(Some("Shutdown requested".to_string()));
                Ok(())
            }
        }
    }

    async fn post_stop(
        &self,
        _myself: ActorRef<Self::Msg>,
        _state: &mut Self::State,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        log::info!("ObservabilityActor stopped");
        Ok(())
    }
}
