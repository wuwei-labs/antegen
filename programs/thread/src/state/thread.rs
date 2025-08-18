use crate::*;
use anchor_lang::{
    prelude::*,
    solana_program::instruction::{AccountMeta, Instruction},
    AnchorDeserialize, AnchorSerialize,
};
use std::collections::HashMap;

/// Current version of the Thread structure.
pub const CURRENT_THREAD_VERSION: u8 = 1;

/// Static pubkey for the payer placeholder - this is a placeholder address
/// "AntegenPayer1111111111111111111111111111111" in base58  
pub const PAYER_PUBKEY: Pubkey = anchor_lang::prelude::pubkey!("AntegenPayer1111111111111111111111111111111");

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
        Self {
            close_to: None,
            next_instruction: None,
            trigger: None,
        }
    }
}

/// Compiled instruction data for space-efficient storage
#[derive(AnchorDeserialize, AnchorSerialize, Clone, Debug)]
pub struct CompiledInstructionData {
    pub program_id_index: u8,
    pub accounts: Vec<u8>,
    pub data: Vec<u8>,
}

/// Compiled instruction containing deduplicated accounts
#[derive(AnchorDeserialize, AnchorSerialize, Clone, Debug)]
pub struct CompiledInstructionV0 {
    pub num_ro_signers: u8,
    pub num_rw_signers: u8,
    pub num_rw: u8,
    pub instructions: Vec<CompiledInstructionData>,
    pub signer_seeds: Vec<Vec<Vec<u8>>>,
    pub accounts: Vec<Pubkey>,
}

/// Tracks the current state of a transaction thread on Solana.
#[account]
#[derive(Debug, InitSpace)]
pub struct Thread {
    /// The version of this thread structure, for migration purposes.
    pub version: u8,
    /// The bump, used for PDA validation.
    pub bump: u8,
    /// The owner of this thread.
    pub authority: Pubkey,
    /// The raw bytes to ensure 32 byte seed limit is maintained
    #[max_len(32)]
    pub id: Vec<u8>,
    /// The string representation of the id
    #[max_len(64)]
    pub name: String,
    pub created_at: i64,
    /// Whether or not the thread is currently paused.
    pub paused: bool,
    /// The instructions to be executed. (aka fibers)
    #[max_len(50)]
    pub fibers: Vec<u8>,
    pub exec_index: u8,

    pub nonce_account: Pubkey,
    #[max_len(44)]
    pub last_nonce: String,

    /// The triggering event to kickoff a thread.
    pub trigger: Trigger,
    pub trigger_context: TriggerContext,
}

impl Thread {
    /// Derive the pubkey of a thread account.
    pub fn pubkey(authority: Pubkey, id: impl AsRef<[u8]>) -> Pubkey {
        let id_bytes = id.as_ref();
        assert!(id_bytes.len() <= 32, "Thread ID must not exceed 32 bytes");

        Pubkey::find_program_address(&[SEED_THREAD, authority.as_ref(), id_bytes], &crate::ID).0
    }

    /// Check if this thread has a nonce account.
    pub fn has_nonce_account(&self) -> bool {
        self.nonce_account != anchor_lang::solana_program::system_program::ID
    }

    /// Advance exec_index to the next fiber in the sequence.
    pub fn advance_to_next_fiber(&mut self) {
        if self.fibers.is_empty() {
            self.exec_index = 0;
            return;
        }

        // Find current index position in fibers vec
        if let Some(current_pos) = self.fibers.iter().position(|&x| x == self.exec_index) {
            // Move to next fiber, or wrap to beginning
            let next_pos = (current_pos + 1) % self.fibers.len();
            self.exec_index = self.fibers[next_pos];
        } else {
            // Current exec_index not found, reset to first fiber
            self.exec_index = self.fibers.first().copied().unwrap_or(0);
        }
    }

}

impl TryFrom<Vec<u8>> for Thread {
    type Error = Error;

    fn try_from(data: Vec<u8>) -> std::result::Result<Self, Self::Error> {
        Thread::try_deserialize(&mut data.as_slice())
    }
}

/// Compile an instruction into a space-efficient format
pub fn compile_instruction(
    instruction: Instruction,
    signer_seeds: Vec<Vec<Vec<u8>>>,
) -> Result<CompiledInstructionV0> {
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
    let compiled_instruction = CompiledInstructionData {
        program_id_index: *accounts_to_index.get(&instruction.program_id).unwrap(),
        accounts: instruction
            .accounts
            .iter()
            .map(|acc| *accounts_to_index.get(&acc.pubkey).unwrap())
            .collect(),
        data: instruction.data,
    };

    Ok(CompiledInstructionV0 {
        num_ro_signers,
        num_rw_signers,
        num_rw,
        instructions: vec![compiled_instruction],
        signer_seeds,
        accounts: sorted_accounts,
    })
}

/// Decompile a compiled instruction back to a regular instruction
pub fn decompile_instruction(compiled: &CompiledInstructionV0) -> Result<Instruction> {
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
