use anchor_lang::{
    prelude::Pubkey,
    prelude::*,
    solana_program,
    solana_program::instruction::{AccountMeta, Instruction},
    AnchorDeserialize,
};
use static_pubkey::static_pubkey;
use std::collections::HashMap;
use std::fmt::Debug;

pub static PAYER_PUBKEY: Pubkey = static_pubkey!("AntegenPayer1111111111111111111111111111111");

/// Serializable version of Solana's Instruction for easier handling
#[derive(AnchorDeserialize, AnchorSerialize, Clone, Debug)]
pub struct SerializableInstruction {
    pub program_id: Pubkey,
    pub accounts: Vec<SerializableAccountMeta>,
    pub data: Vec<u8>,
}

/// Serializable version of AccountMeta
#[derive(AnchorDeserialize, AnchorSerialize, Clone, Debug)]
pub struct SerializableAccountMeta {
    pub pubkey: Pubkey,
    pub is_signer: bool,
    pub is_writable: bool,
}

impl From<Instruction> for SerializableInstruction {
    fn from(ix: Instruction) -> Self {
        SerializableInstruction {
            program_id: ix.program_id,
            accounts: ix
                .accounts
                .into_iter()
                .map(|acc| SerializableAccountMeta {
                    pubkey: acc.pubkey,
                    is_signer: acc.is_signer,
                    is_writable: acc.is_writable,
                })
                .collect(),
            data: ix.data,
        }
    }
}

impl From<SerializableInstruction> for Instruction {
    fn from(ix: SerializableInstruction) -> Self {
        Instruction {
            program_id: ix.program_id,
            accounts: ix
                .accounts
                .into_iter()
                .map(|acc| AccountMeta {
                    pubkey: acc.pubkey,
                    is_signer: acc.is_signer,
                    is_writable: acc.is_writable,
                })
                .collect(),
            data: ix.data,
        }
    }
}

/// The triggering conditions of a thread.
#[derive(AnchorDeserialize, AnchorSerialize, Clone, InitSpace, PartialEq, Debug)]
pub enum Trigger {
    /// Allows a thread to be kicked off whenever the data of an account changes.
    Account {
        /// The address of the account to monitor.
        address: Pubkey,
        /// The byte offset of the account data to monitor.
        offset: u64,
        /// The size of the byte slice to monitor (must be less than 1kb)
        size: u64,
    },

    /// Allows a thread to be kicked off as soon as it's created.
    Now,

    /// Allows a thread to be kicked off according to a unix timestamp.
    Timestamp { unix_ts: i64 },

    /// Allows a thread to be kicked off at regular intervals.
    Interval {
        /// Interval in seconds between executions
        seconds: i64,
        /// Boolean value indicating whether triggering moments may be skipped
        skippable: bool,
    },

    /// Allows a thread to be kicked off according to a one-time or recurring schedule.
    Cron {
        /// The schedule in cron syntax. Value must be parsable by the `solana_cron` package.
        #[max_len(255)]
        schedule: String,

        /// Boolean value indicating whether triggering moments may be skipped if they are missed (e.g. due to network downtime).
        /// If false, any "missed" triggering moments will simply be executed as soon as the network comes back online.
        skippable: bool,
    },

    /// Allows a thread to be kicked off according to a slot.
    Slot { slot: u64 },

    /// Allows a thread to be kicked off according to an epoch number.
    Epoch { epoch: u64 },
}

/// The event which allowed a particular transaction thread to be triggered.
#[derive(AnchorDeserialize, AnchorSerialize, Clone, InitSpace, Debug)]
pub enum TriggerContext {
    /// A running hash of the observed account data.
    Account { hash: u64 },

    /// The trigger context for Now, Timestamp, Cron, and Interval
    Timestamp { prev: i64, next: i64 },

    /// The trigger context for Slot and Epoch
    Block { prev: u64, next: u64 },
}

/// A response value target programs can return to update the thread.
#[derive(AnchorDeserialize, AnchorSerialize, Clone, Debug)]
pub struct ThreadResponse {
    pub close_to: Option<Pubkey>,
    pub next_instruction: Option<u8>,
    pub trigger: Option<Trigger>,
}

impl Default for ThreadResponse {
    fn default() -> Self {
        return Self {
            close_to: None,
            next_instruction: None,
            trigger: None,
        };
    }
}

/// Compiled instruction format for space-efficient storage
#[derive(AnchorDeserialize, AnchorSerialize, Clone, Debug)]
pub struct CompiledInstructionV0 {
    pub program_id_index: u8,
    pub accounts: Vec<u8>,
    pub data: Vec<u8>,
}

/// Compiled transaction containing deduplicated accounts
#[derive(AnchorDeserialize, AnchorSerialize, Clone, Debug)]
pub struct CompiledTransactionV0 {
    pub num_ro_signers: u8,
    pub num_rw_signers: u8,
    pub num_rw: u8,
    pub instructions: Vec<CompiledInstructionV0>,
    pub signer_seeds: Vec<Vec<Vec<u8>>>,
    pub accounts: Vec<Pubkey>,
}

/// Compile an instruction into a space-efficient format
pub fn compile_instruction(
    instruction: Instruction,
    signer_seeds: Vec<Vec<Vec<u8>>>,
) -> Result<CompiledTransactionV0> {
    let mut pubkeys_to_metadata: HashMap<Pubkey, AccountMeta> = HashMap::new();

    // Add program ID
    pubkeys_to_metadata.insert(
        instruction.program_id,
        AccountMeta {
            pubkey: instruction.program_id,
            is_signer: false,
            is_writable: false,
        },
    );

    // Process accounts
    for acc in &instruction.accounts {
        let entry = pubkeys_to_metadata
            .entry(acc.pubkey)
            .or_insert(AccountMeta {
                pubkey: acc.pubkey,
                is_signer: false,
                is_writable: false,
            });
        entry.is_signer |= acc.is_signer;
        entry.is_writable |= acc.is_writable;
    }

    // Sort accounts by priority
    let mut sorted_accounts: Vec<Pubkey> = pubkeys_to_metadata.keys().cloned().collect();
    sorted_accounts.sort_by(|a, b| {
        let a_meta = &pubkeys_to_metadata[a];
        let b_meta = &pubkeys_to_metadata[b];

        fn get_priority(meta: &AccountMeta) -> u8 {
            match (meta.is_signer, meta.is_writable) {
                (true, true) => 0,
                (true, false) => 1,
                (false, true) => 2,
                (false, false) => 3,
            }
        }

        get_priority(a_meta).cmp(&get_priority(b_meta))
    });

    // Count account types
    let mut num_rw_signers = 0u8;
    let mut num_ro_signers = 0u8;
    let mut num_rw = 0u8;

    for pubkey in &sorted_accounts {
        let meta = &pubkeys_to_metadata[pubkey];
        if meta.is_signer && meta.is_writable {
            num_rw_signers += 1;
        } else if meta.is_signer && !meta.is_writable {
            num_ro_signers += 1;
        } else if meta.is_writable {
            num_rw += 1;
        }
    }

    // Create index mapping
    let accounts_to_index: HashMap<Pubkey, u8> = sorted_accounts
        .iter()
        .enumerate()
        .map(|(i, k)| (*k, i as u8))
        .collect();

    // Create compiled instruction
    let compiled_instruction = CompiledInstructionV0 {
        program_id_index: *accounts_to_index.get(&instruction.program_id).unwrap(),
        accounts: instruction
            .accounts
            .iter()
            .map(|acc| *accounts_to_index.get(&acc.pubkey).unwrap())
            .collect(),
        data: instruction.data,
    };

    Ok(CompiledTransactionV0 {
        num_ro_signers,
        num_rw_signers,
        num_rw,
        instructions: vec![compiled_instruction],
        signer_seeds,
        accounts: sorted_accounts,
    })
}

/// Decompile a compiled instruction back to a regular instruction
pub fn decompile_instruction(compiled: &CompiledTransactionV0) -> Result<Instruction> {
    if compiled.instructions.is_empty() {
        return Err(ProgramError::InvalidInstructionData.into());
    }

    let ix = &compiled.instructions[0];
    let program_id = compiled.accounts[ix.program_id_index as usize];

    let accounts: Vec<AccountMeta> = ix
        .accounts
        .iter()
        .enumerate()
        .map(|(_i, &idx)| {
            let pubkey = compiled.accounts[idx as usize];
            let is_writable = if idx < compiled.num_rw_signers {
                true
            } else if idx < compiled.num_rw_signers + compiled.num_ro_signers {
                false
            } else if idx < compiled.num_rw_signers + compiled.num_ro_signers + compiled.num_rw {
                true
            } else {
                false
            };
            let is_signer = idx < compiled.num_rw_signers + compiled.num_ro_signers;

            AccountMeta {
                pubkey,
                is_signer,
                is_writable,
            }
        })
        .collect();

    Ok(Instruction {
        program_id,
        accounts,
        data: ix.data.clone(),
    })
}

pub fn transfer_lamports<'info>(
    from: &AccountInfo<'info>,
    to: &AccountInfo<'info>,
    amount: u64,
) -> Result<()> {
    **from.try_borrow_mut_lamports()? = from.lamports().checked_sub(amount).unwrap();
    **to.try_borrow_mut_lamports()? = to.to_account_info().lamports().checked_add(amount).unwrap();
    Ok(())
}
