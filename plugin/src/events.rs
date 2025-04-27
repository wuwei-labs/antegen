use agave_geyser_plugin_interface::geyser_plugin_interface::{
    GeyserPluginError, ReplicaAccountInfo,
};
use anchor_lang::{prelude::AccountInfo, AccountDeserialize};
use antegen_thread_program::state::Thread;
use pyth_sdk_solana::{state::SolanaPriceAccount, PriceFeed};
use solana_program::{clock::Clock, pubkey::Pubkey, sysvar};
use static_pubkey::static_pubkey;

static PYTH_ORACLE_PROGRAM_ID_MAINNET: Pubkey =
    static_pubkey!("FsJ3A3u2vn5cTVofAjvy6y5kwABJAqYWpe4975bi2epH");
static PYTH_ORACLE_PROGRAM_ID_DEVNET: Pubkey =
    static_pubkey!("gSbePebfvPy7tRqimPoVecS2UsBvYv46ynrzWocc92s");

use log::info;

#[derive(Debug)]
pub enum AccountUpdateEvent {
    Clock { clock: Clock },
    Thread { thread: Thread },
    PriceFeed { price_feed: PriceFeed },
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

        // If the account belongs to Pyth, attempt to parse it.
        if owner_pubkey == PYTH_ORACLE_PROGRAM_ID_MAINNET
            || owner_pubkey == PYTH_ORACLE_PROGRAM_ID_DEVNET
        {
            let data = &mut account_info.data.to_vec();
            let acc_info = AccountInfo::new(
                &account_pubkey,
                false,
                false,
                &mut account_info.lamports,
                data,
                &owner_pubkey,
                account_info.executable,
                account_info.rent_epoch,
            );
            let price_feed = SolanaPriceAccount::account_info_to_feed(&acc_info).map_err(|_| {
                GeyserPluginError::AccountsUpdateError {
                    msg: "Failed to parse Pyth price account".into(),
                }
            })?;
            return Ok(AccountUpdateEvent::PriceFeed { price_feed });
        }

        Err(GeyserPluginError::AccountsUpdateError {
            msg: "Account is not relevant to Antegen plugin".into(),
        })
    }
}
