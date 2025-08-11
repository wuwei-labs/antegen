use std::{fmt::Debug, sync::Arc};

use agave_geyser_plugin_interface::geyser_plugin_interface::Result as PluginResult;
use antegen_thread_program::state::Thread;
use async_nats::jetstream::Error;
use log::info;
use solana_program::{clock::Clock, pubkey::Pubkey};

pub struct ThreadObserver {}

impl Debug for ThreadObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "thread-observer")
    }
}

// fn next_moment(after: i64, schedule: String) -> Option<i64> {
//     match Schedule::from_str(&schedule) {
//         Err(_) => None,
//         Ok(schedule) => schedule
//             .next_after(&DateTime::<Utc>::from_timestamp(after, 0).unwrap())
//             .take()
//             .map(|datetime| datetime.timestamp()),
//     }
// }

impl ThreadObserver {
    pub async fn new() -> Result<Self, Error> {
        Ok(Self {})
    }

    pub async fn process_slot(self: Arc<Self>, slot: u64) -> PluginResult<()> {
        info!("processing slot: {}", slot);
        Ok(())
    }

    pub async fn observe_clock(self: Arc<Self>, clock: Clock) -> PluginResult<()> {
        // Insert the clock data into the database
        info!("observed clock: {:?}", clock);
        Ok(())
    }

    pub async fn observe_account(
        self: Arc<Self>,
        account_pubkey: Pubkey,
        _slot: u64,
    ) -> PluginResult<()> {
        info!("observed account: {}", account_pubkey.to_string());
        Ok(())
    }

    pub async fn observe_thread(
        self: Arc<Self>,
        thread: Thread,
        thread_pubkey: Pubkey,
        _slot: u64,
    ) -> PluginResult<()> {
        if thread.paused {
            return Ok(());
        }

        info!("observed thread: {:?}", thread_pubkey.to_string());
        Ok(())
    }
}
