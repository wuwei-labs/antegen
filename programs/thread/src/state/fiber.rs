use crate::*;
use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::Instruction;

/// Trait for processing fiber instructions
pub trait FiberInstructionProcessor {
    /// Get the decompiled instruction from the fiber's compiled data,
    /// replacing PAYER_PUBKEY with the provided executor
    fn get_instruction(&self, executor: &Pubkey) -> Result<Instruction>;
}

/// Represents a single fiber (instruction) in a thread's execution sequence.
#[account]
#[derive(Debug, InitSpace)]
pub struct FiberState {
    /// The thread this fiber belongs to
    pub thread: Pubkey,
    /// The index of this fiber in the thread's execution sequence
    pub fiber_index: u8,
    /// The compiled instruction data
    #[max_len(1024)]
    pub compiled_instruction: Vec<u8>,
    /// When this fiber was last executed
    pub last_executed: i64,
    /// Total number of executions
    pub exec_count: u64,
    /// Priority fee in microlamports for compute unit price (0 = no priority fee)
    pub priority_fee: u64,
}

impl FiberState {
    /// Derive the pubkey of a fiber account.
    pub fn pubkey(thread: Pubkey, fiber_index: u8) -> Pubkey {
        Pubkey::find_program_address(&[SEED_THREAD_FIBER, thread.as_ref(), &[fiber_index]], &crate::ID).0
    }
}

impl FiberInstructionProcessor for FiberState {
    fn get_instruction(&self, executor: &Pubkey) -> Result<Instruction> {
        // Deserialize the compiled instruction
        let compiled = CompiledInstructionV0::try_from_slice(&self.compiled_instruction)?;
        // Decompile the instruction
        let mut instruction = decompile_instruction(&compiled)?;

        // Replace PAYER_PUBKEY with the actual executor
        for acc in instruction.accounts.iter_mut() {
            if acc.pubkey.eq(&PAYER_PUBKEY) {
                acc.pubkey = *executor;
            }
        }

        Ok(instruction)
    }
}
