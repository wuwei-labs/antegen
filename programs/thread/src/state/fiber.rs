use crate::*;
use anchor_lang::prelude::*;

/// Represents a single fiber (instruction) in a thread's execution sequence.
#[account]
#[derive(Debug, InitSpace)]
pub struct FiberState {
    /// The thread this fiber belongs to
    pub thread: Pubkey,
    /// The index of this fiber in the thread's execution sequence
    pub index: u8,
    /// The compiled instruction data
    #[max_len(1024)]
    pub compiled_instruction: Vec<u8>,
    /// When this fiber was last executed
    pub last_executed: i64,
    /// Total number of executions
    pub execution_count: u64,
}

impl FiberState {
    /// Derive the pubkey of a fiber account.
    pub fn pubkey(thread: Pubkey, index: u8) -> Pubkey {
        Pubkey::find_program_address(
            &[SEED_THREAD_FIBER, thread.as_ref(), &[index]],
            &crate::ID,
        )
        .0
    }
}