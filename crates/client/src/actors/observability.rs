//! Observability Actor
//!
//! Wraps the loa-core agent and runs it within the antegen actor hierarchy.

use crate::config::ObservabilityConfig;
use loa_core::{Runtime, RuntimeHandle};
use ractor::{Actor, ActorProcessingErr, ActorRef};
use std::error::Error;
use std::path::PathBuf;

/// Backend the agent reports to.
///
/// loa-core 2.x defaulted to this internally; 3.0 requires it explicitly, so
/// the previous behaviour is pinned here rather than silently changing.
const LOA_API_URL: &str = "https://api.loa.sh";

/// Workspace this node's telemetry is attributed to.
///
/// In loa-core 2.x this was passed to `Agent::builder().claim(..)`, which was
/// simply a setter for `workspace_id` — the same value, under the name 3.0 uses.
const LOA_WORKSPACE_ID: &str = "jx7dy16t7pm9q6273bxxtfgr757ykemr";

/// Messages for the ObservabilityActor
pub enum ObservabilityMessage {
    Shutdown,
}

pub struct ObservabilityActor;

pub struct ObservabilityState {
    /// Kept to run loa-core's subsystems, and taken on stop so they can be shut
    /// down cleanly. Dropping the handle leaves the tasks running.
    handle: Option<RuntimeHandle>,
}

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

        let handle = Runtime::builder(PathBuf::from(&storage_path), LOA_API_URL.to_string())
            .workspace_id(LOA_WORKSPACE_ID)
            .agent_version(env!("CARGO_PKG_VERSION"))
            .build()
            .start()
            .await
            .map_err(|e| format!("Failed to start loa-core runtime: {}", e))?;

        log::info!(
            "Loa observability agent started (agent {})",
            handle.identity.slug()
        );

        Ok(ObservabilityState {
            handle: Some(handle),
        })
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
        state: &mut Self::State,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        // 3.0 exposes an explicit shutdown; 2.x had none, so the agent's tasks
        // were simply left running until the process exited.
        if let Some(handle) = state.handle.take() {
            handle.shutdown().await;
        }
        log::info!("ObservabilityActor stopped");
        Ok(())
    }
}
