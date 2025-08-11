use agave_geyser_plugin_interface::geyser_plugin_interface::{
    GeyserPluginError, ReplicaAccountInfo,
};
use anchor_lang::AccountDeserialize;
use antegen_thread_program::state::Thread;
use solana_program::{clock::Clock, pubkey::Pubkey, sysvar};

use log::info;

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
            match Thread::try_deserialize(&mut data.as_slice()) {
                Ok(thread) => {
                    info!("Successfully parsed thread program {}", account_pubkey);
                    return Ok(AccountUpdateEvent::Thread { thread });
                }
                Err(e) => {
                    info!("Failed to parse Thread: {:?}", e);
                    info!("Raw account data: {:?}", &data[..8]);
                    return Err(GeyserPluginError::AccountsUpdateError {
                        msg: format!("Failed to deserialize Thread for {}", account_pubkey),
                    });
                }
            };
        }

        Err(GeyserPluginError::AccountsUpdateError {
            msg: "Account is not relevant to Antegen plugin".into(),
        })
    }
}
