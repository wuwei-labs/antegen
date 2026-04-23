pub mod constants;
pub mod errors;
pub mod instructions;
pub mod state;

pub use constants::*;
use instructions::*;
use state::*;

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::Instruction;

declare_id!("AgFv5afjW9DmSPkiEvJ1er5bAAmRUqaBeTB6Cr8e1hKx");

#[program]
pub mod antegen_fiber {
    use super::*;

    /// Creates a fiber (instruction account) for a thread.
    /// Thread PDA must be signer and payer.
    pub fn create(
        ctx: Context<Create>,
        fiber_index: u8,
        instruction: SerializableInstruction,
        priority_fee: u64,
    ) -> Result<()> {
        let instruction: Instruction = instruction.into();
        instructions::create::create(ctx, fiber_index, instruction, priority_fee)
    }

    /// Updates a fiber's instruction content (or initializes if it doesn't exist).
    /// Thread PDA must be signer and payer. Resets execution stats.
    /// Pass `None` to wipe the compiled instruction (idle fiber).
    pub fn update(
        ctx: Context<Update>,
        fiber_index: u8,
        instruction: Option<SerializableInstruction>,
        priority_fee: Option<u64>,
    ) -> Result<()> {
        let instruction = instruction.map(|i| i.into());
        instructions::update::update(ctx, fiber_index, instruction, priority_fee)
    }

    /// Closes a fiber account, returns rent to thread PDA.
    pub fn close(ctx: Context<Close>) -> Result<()> {
        instructions::close::close(ctx)
    }

    /// Copies source fiber's instruction into target, closes source.
    /// Target keeps its PDA/index, source is deleted.
    pub fn swap(ctx: Context<Swap>) -> Result<()> {
        instructions::swap::swap(ctx)
    }
}
