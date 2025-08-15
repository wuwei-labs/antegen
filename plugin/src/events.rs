use agave_geyser_plugin_interface::geyser_plugin_interface::{
    GeyserPluginError, ReplicaAccountInfo,
};
use anchor_lang::AccountDeserialize;
use antegen_thread_program::state::Thread;
use solana_program::{clock::Clock, pubkey::Pubkey, sysvar};

use log::{debug, info};

#[derive(Debug)]
pub enum AccountUpdateEvent {
    Clock { clock: Clock },
    Thread { thread: Thread },
}

impl TryFrom<&mut ReplicaAccountInfo<'_>> for AccountUpdateEvent {
    type Error = GeyserPluginError;
    fn try_from(account_info: &mut ReplicaAccountInfo) -> Result<Self, Self::Error> {
        // Parse pubkeys.
        let account_pubkey = Pubkey::try_from(account_info.pubkey).unwrap();
        let owner_pubkey = Pubkey::try_from(account_info.owner).unwrap();

        // If the account is the sysvar clock, parse it.
        if account_pubkey == sysvar::clock::ID {
            return Ok(AccountUpdateEvent::Clock {
                clock: bincode::deserialize::<Clock>(account_info.data).map_err(|_e| {
                    GeyserPluginError::AccountsUpdateError {
                        msg: "Failed to parse sysvar clock account".into(),
                    }
                })?,
            });
        }

        // If the account belongs to the thread v1 program, parse it.
        if owner_pubkey == antegen_thread_program::ID && account_info.data.len() > 8 {
            let data = account_info.data.to_vec();
            
            // Check discriminator (first 8 bytes)
            let actual_disc = &data[..8];
            
            // Thread discriminator: SHA256("account:Thread")[:8]
            const THREAD_DISC: [u8; 8] = [0xba, 0x1b, 0x9a, 0x6f, 0x33, 0x24, 0x9f, 0x5a];
            // FiberState discriminator: SHA256("account:FiberState")[:8]  
            const FIBER_STATE_DISC: [u8; 8] = [0x36, 0x0b, 0xfb, 0x3c, 0x3f, 0xc5, 0x55, 0x24];
            
            // Silently ignore FiberState accounts
            if actual_disc == FIBER_STATE_DISC {
                return Err(GeyserPluginError::AccountsUpdateError {
                    msg: "Account is not relevant to Antegen plugin".into(),
                });
            }
            
            // Only process Thread accounts
            if actual_disc == THREAD_DISC {
                match Thread::try_deserialize(&mut data.as_slice()) {
                    Ok(thread) => {
                        debug!(
                            "Successfully parsed thread {} (id: {:?}, trigger: {:?}, fibers: {:?}, paused: {})",
                            account_pubkey,
                            String::from_utf8_lossy(&thread.id),
                            thread.trigger,
                            thread.fibers,
                            thread.paused
                        );
                        return Ok(AccountUpdateEvent::Thread { thread });
                    }
                    Err(e) => {
                        info!(
                            "Failed to parse Thread account {}: {:?}",
                            account_pubkey, e
                        );
                        return Err(GeyserPluginError::AccountsUpdateError {
                            msg: format!("Failed to deserialize Thread for {}", account_pubkey),
                        });
                    }
                }
            }
            
            // Unknown discriminator for thread program account
            debug!(
                "Unknown account type for {} (discriminator: {:?})",
                account_pubkey, actual_disc
            );
        }

        Err(GeyserPluginError::AccountsUpdateError {
            msg: "Account is not relevant to Antegen plugin".into(),
        })
    }
}
