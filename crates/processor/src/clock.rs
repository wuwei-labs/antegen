use std::sync::Arc;
use tokio::sync::RwLock;

/// Blockchain clock state shared across all components
#[derive(Clone, Debug, Default)]
pub struct ClockState {
    pub slot: u64,
    pub epoch: u64,
    pub unix_timestamp: i64,
}

/// Shared blockchain clock that can be updated and read from multiple components
#[derive(Clone)]
pub struct SharedClock {
    state: Arc<RwLock<ClockState>>,
}

impl SharedClock {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(ClockState::default())),
        }
    }

    /// Update the clock state
    pub async fn update(&self, slot: u64, epoch: u64, unix_timestamp: i64) {
        let mut state = self.state.write().await;
        *state = ClockState {
            slot,
            epoch,
            unix_timestamp,
        };
    }

    /// Get the current clock state
    pub async fn get(&self) -> ClockState {
        self.state.read().await.clone()
    }

    /// Get just the current unix timestamp
    pub async fn get_timestamp(&self) -> i64 {
        self.state.read().await.unix_timestamp
    }

    /// Get just the current slot
    pub async fn get_slot(&self) -> u64 {
        self.state.read().await.slot
    }

    /// Get just the current epoch  
    pub async fn get_epoch(&self) -> u64 {
        self.state.read().await.epoch
    }
}

impl Default for SharedClock {
    fn default() -> Self {
        Self::new()
    }
}