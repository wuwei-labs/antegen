use anchor_lang::{AccountDeserialize, Discriminator};
use antegen_thread_program::state::Thread;
use log::{debug, error};
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
                debug!(
                    "Classified Clock sysvar - timestamp: {}, slot: {}, epoch: {}",
                    clock.unix_timestamp, clock.slot, clock.epoch
                );
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
            // Parse thread data (skip discriminator)
            match Thread::try_deserialize(&mut &account.data[8..]) {
                Ok(thread) => {
                    debug!(
                        "Classified Thread account {} - paused: {}, trigger: {:?}",
                        pubkey, thread.paused, thread.trigger
                    );
                    return AccountType::Thread(thread);
                }
                Err(e) => {
                    error!("Failed to deserialize Thread account {}: {}", pubkey, e);
                    return AccountType::Other;
                }
            }
        }
    }

    // Not a special account type
    debug!("Account {} classified as Other", pubkey);
    AccountType::Other
}
