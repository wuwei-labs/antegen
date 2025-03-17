use std::mem::size_of;

use anchor_lang::{prelude::*, AnchorDeserialize, AnchorSerialize};
use antegen_utils::thread::{ClockData, SerializableInstruction, Trigger};

pub const SEED_THREAD: &[u8] = b"thread";

/// Current version of the Thread structure.
pub const CURRENT_THREAD_VERSION: u8 = 1;

/// Static space for next_instruction field.
pub const NEXT_INSTRUCTION_SIZE: usize = 1232;

/// Tracks the current state of a transaction thread on Solana.
#[account]
#[derive(Debug)]
pub struct Thread {
    /// The version of this thread structure, for migration purposes.
    pub version: u8,
    /// The owner of this thread.
    pub authority: Pubkey,
    /// The bump, used for PDA validation.
    pub bump: u8,
    /// The cluster clock at the moment the thread was created.
    pub created_at: ClockData,
    /// The context of the thread's current execution state.
    pub exec_context: Option<ExecContext>,
    /// The number of lamports to payout to workers per execution.
    pub fee: u64,
    /// The id of the thread, given by the authority.
    pub id: Vec<u8>,
    /// The instructions to be executed.
    pub instructions: Vec<SerializableInstruction>,
    /// The name of the thread.
    pub name: String,
    /// The next instruction to be executed.
    pub next_instruction: Option<SerializableInstruction>,
    /// Whether or not the thread is currently paused.
    pub paused: bool,
    /// The maximum number of execs allowed per slot.
    pub rate_limit: u64,
    /// The triggering event to kickoff a thread.
    pub trigger: Trigger,
}

impl Thread {
    /// Derive the pubkey of a thread account.
    pub fn pubkey(authority: Pubkey, id: impl AsRef<[u8]>) -> Pubkey {
        let id_bytes = id.as_ref();
        assert!(id_bytes.len() <= 32, "Thread ID must not exceed 32 bytes");

        Pubkey::find_program_address(
            &[SEED_THREAD, authority.as_ref(), id_bytes],
            &crate::ID,
        )
        .0
    }
}

impl PartialEq for Thread {
    fn eq(&self, other: &Self) -> bool {
        self.authority.eq(&other.authority) && self.id.eq(&other.id)
    }
}

impl Eq for Thread {}

impl TryFrom<Vec<u8>> for Thread {
    type Error = Error;

    fn try_from(data: Vec<u8>) -> std::result::Result<Self, Self::Error> {
        Thread::try_deserialize(&mut data.as_slice())
    }
}

/// Trait for reading and writing to a thread account.
pub trait ThreadAccount {
    /// Get the pubkey of the thread account.
    fn pubkey(&self) -> Pubkey;

    /// Allocate more memory for the account.
    fn realloc(&mut self) -> Result<()>;

    /// Migrate the thread to the current version if needed.
    fn migrate_if_needed(&mut self) -> Result<()>;

    /// Checks if thread has version field
    fn is_legacy_thread(&self) -> bool;

    /// Migrates thread from legacy to v1
    fn migrate_legacy_thread(&mut self) -> Result<()>;
}

impl ThreadAccount for Account<'_, Thread> {
    fn pubkey(&self) -> Pubkey {
        Thread::pubkey(self.authority, self.id.clone())
    }

    fn realloc(&mut self) -> Result<()> {
        // Realloc memory for the thread account
        let data_len = 8 +            // discriminator
            size_of::<Thread>() +            // base struct
            self.id.len() +                  // id length
            4 + (self.instructions.len() * size_of::<SerializableInstruction>()) + // vec length prefix + items
            size_of::<Trigger>() +            // trigger enum
            NEXT_INSTRUCTION_SIZE;            // next instruction

        self.to_account_info().realloc(data_len, false)?;
        Ok(())
    }

    fn migrate_if_needed(&mut self) -> Result<()> {
        if self.is_legacy_thread() {
            msg!("Detected legacy thread, migrating to current version");
            self.migrate_legacy_thread()?;
            return Ok(());
        }

        // Handle regular version upgrades
        if self.version < CURRENT_THREAD_VERSION {
            // Migrate through each version sequentially
            while self.version < CURRENT_THREAD_VERSION {
                let from_version: u8 = self.version;
                let to_version: u8 = from_version + 1;
                msg!("Upgrading thread from version {} to {}", from_version, to_version);

                // Perform version-specific migrations
                match from_version {
                    // Add cases for future versions here
                    // For example:
                    // 1 => {
                    //     // Version 1 to 2 upgrade
                    //     // Calculate new size if adding fields
                    //     let current_size = self.to_account_info().data_len();
                    //     let new_size = current_size + size_of::<NewFieldType>();
                    //     self.to_account_info().realloc(new_size, false)?;
                    //     self.new_field = default_value;
                    // },
                    _ => {}
                }

                self.version = to_version;
            }
            
            msg!("Thread successfully upgraded to version {}", CURRENT_THREAD_VERSION);
        }
        
        Ok(())
    }

    /// Detects if a thread is a legacy thread (pre-versioning)
    fn is_legacy_thread(&self) -> bool {
        // Quick check: If version is already set to a non-zero value, it's definitely not a legacy thread
        if self.version.gt(&0) {
            return false;
        }

        // Look at the first byte of authority to see if it matches the version
        // In a legacy thread, the version field would actually be the first byte of the authority
        self.version.eq(&self.authority.as_ref()[0])
    }

    /// Migrates a legacy thread to the current version.
    fn migrate_legacy_thread(&mut self) -> Result<()> {
        let current_data_len: usize = self.to_account_info().data_len();
        let new_space: usize = current_data_len + 1;
        self.to_account_info().realloc(new_space, false)?;
        let authority: Pubkey = self.authority;

        self.authority = authority;
        self.version = 1;

        msg!("Legacy thread migrated to version {} (size: {} -> {})", 
            CURRENT_THREAD_VERSION, current_data_len, new_space);
        Ok(())
    }
}

/// The execution context of a particular transaction thread.
#[derive(AnchorDeserialize, AnchorSerialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecContext {
    /// Index of the next instruction to be executed.
    pub exec_index: u64,

    /// Number of execs since the last tx reimbursement.
    /// To be deprecated in v3 since we now reimburse for every transaction.
    pub execs_since_reimbursement: u64,

    /// Number of execs in this slot.
    pub execs_since_slot: u64,

    /// Slot of the current exec
    pub last_exec_at: u64,

    /// Unix timestamp of last exec
    pub last_exec_timestamp: i64,

    /// Context for the triggering condition
    pub trigger_context: TriggerContext,
}

/// The event which allowed a particular transaction thread to be triggered.
#[derive(AnchorDeserialize, AnchorSerialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerContext {
    /// A running hash of the observed account data.
    Account {
        /// The account's data hash.
        data_hash: u64,
    },

    /// A cron execution context.
    Cron {
        /// The threshold moment the schedule was waiting for.
        started_at: i64,
    },

    /// The trigger context for threads with a "now" trigger.
    Now,

    /// The trigger context for threads with a "slot" trigger.
    Slot {
        /// The threshold slot the schedule was waiting for.
        started_at: u64,
    },

    /// The trigger context for threads with an "epoch" trigger.
    Epoch {
        /// The threshold epoch the schedule was waiting for.
        started_at: u64,
    },

    /// The trigger context for threads with an "timestamp" trigger.
    Timestamp {
        /// The threshold moment the schedule was waiting for.
        started_at: i64,
    },

    /// The trigger context for threads with a "pyth" trigger.
    Pyth { price: i64 },
}

/// The properties of threads which are updatable.
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct ThreadSettings {
    pub fee: Option<u64>,
    pub instructions: Option<Vec<SerializableInstruction>>,
    pub name: Option<String>,
    pub rate_limit: Option<u64>,
    pub trigger: Option<Trigger>,
}
