pub mod plugin;

use std::{fmt::Debug, sync::Arc};

use agave_geyser_plugin_interface::geyser_plugin_interface::GeyserPluginError;
use plugin::ThreadObserver;

pub struct Observers {
    pub plugin: Arc<ThreadObserver>,
}

impl Observers {
    // Make this return a Result that can be propagated up
    pub async fn new() -> Result<Self, GeyserPluginError> {
        // Await the future and handle any errors
        let thread_observer = ThreadObserver::new().await.map_err(|e| {
            GeyserPluginError::Custom(format!("Failed to create ThreadObserver: {}", e).into())
        })?;

        Ok(Observers {
            plugin: Arc::new(thread_observer),
        })
    }
}

impl Debug for Observers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "observers")
    }
}
