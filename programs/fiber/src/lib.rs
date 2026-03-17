pub mod constants;
pub mod errors;
pub mod instructions;
pub mod state;

pub use constants::*;
use instructions::*;
use state::*;

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::Instruction;

declare_id!("AgYwUcNjF3Levf4G71FzBN51rLSWSwcUos8F8DR6bFbr");

#[program]
pub mod fiber_program {
    use super::*;

    /// Creates a fiber (instruction account) for a thread.
    /// Thread PDA must be signer and payer.
    pub fn create_fiber(
        ctx: Context<FiberCreate>,
        _fiber_index: u8,
        instruction: SerializableInstruction,
        priority_fee: u64,
    ) -> Result<()> {
        let instruction: Instruction = instruction.into();
        instructions::fiber_create::fiber_create(ctx, instruction, priority_fee)
    }

    /// Updates a fiber's instruction content.
    /// Thread PDA must be signer. Resets execution stats.
    pub fn update_fiber(
        ctx: Context<FiberUpdate>,
        instruction: SerializableInstruction,
        priority_fee: Option<u64>,
    ) -> Result<()> {
        let instruction: Instruction = instruction.into();
        instructions::fiber_update::fiber_update(ctx, instruction, priority_fee)
    }

    /// Closes a fiber account, returns rent to thread PDA.
    pub fn close_fiber(ctx: Context<FiberClose>) -> Result<()> {
        instructions::fiber_close::fiber_close(ctx)
    }

    /// Copies source fiber's instruction into target, closes source.
    /// Target keeps its PDA/index, source is deleted.
    pub fn swap_fiber(ctx: Context<FiberSwap>) -> Result<()> {
        instructions::fiber_swap::fiber_swap(ctx)
    }
}
