use anchor_lang::{AccountDeserialize, Discriminator};
use antegen_thread_program::state::Thread;
use log::{debug, error, info};
use solana_sdk::{account::Account, clock::Clock, pubkey::Pubkey, sysvar};

/// Classified account type for processing
#[derive(Debug)]
pub enum AccountType {
    /// Clock sysvar with parsed data
    Clock {
        unix_timestamp: i64,
        slot: u64,
        epoch: u64,
    },
    /// Thread account with parsed data
    Thread(Thread),
    /// Other account type (not processed)
    Other,
}

/// Classify an account based on its pubkey and data
pub fn classify_account(pubkey: &Pubkey, account: &Account) -> AccountType {
    // Check if it's the Clock sysvar
    if *pubkey == sysvar::clock::ID {
        // Parse clock data
        match bincode::deserialize::<Clock>(&account.data) {
            Ok(clock) => {
                info!("Clock update: slot={}, epoch={}, timestamp={}", 
                    clock.slot, clock.epoch, clock.unix_timestamp);
                return AccountType::Clock {
                    unix_timestamp: clock.unix_timestamp,
                    slot: clock.slot,
                    epoch: clock.epoch,
                };
            }
            Err(e) => {
                error!("Failed to deserialize Clock sysvar: {}", e);
                return AccountType::Other;
            }
        }
    }

    // Check if it's a Thread account
    if account.owner == antegen_thread_program::ID && account.data.len() > 8 {
        // Check discriminator first
        let discriminator = &account.data[0..8];

        if discriminator == Thread::DISCRIMINATOR {
            // Parse thread data (includes discriminator)
            match Thread::try_deserialize(&mut account.data.as_slice()) {
                Ok(thread) => {
                    info!("Thread parsed: {}", pubkey);
                    return AccountType::Thread(thread);
                }
                Err(e) => {
                    // This shouldn't happen if discriminator matches, but log it
                    debug!("Failed to deserialize Thread account {}: {}", pubkey, e);
                    return AccountType::Other;
                }
            }
        }
        // Not a Thread account (might be Fiber, ThreadConfig, etc.)
    }

    // Not a special account type
    AccountType::Other
}
