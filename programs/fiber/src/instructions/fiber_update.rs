use crate::state::*;
use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::Instruction;

/// Accounts required by the `update_fiber` instruction.
/// Thread PDA must be signer.
#[derive(Accounts)]
pub struct FiberUpdate<'info> {
    /// Thread PDA - must be signer
    pub thread: Signer<'info>,

    /// The fiber to update
    #[account(mut, has_one = thread)]
    pub fiber: Account<'info, FiberState>,
}

pub fn fiber_update(
    ctx: Context<FiberUpdate>,
    instruction: Instruction,
    priority_fee: Option<u64>,
) -> Result<()> {
    let fiber = &mut ctx.accounts.fiber;

    // Recompile the instruction
    let compiled = compile_instruction(instruction)?;
    let compiled_bytes = borsh::to_vec(&compiled)?;

    fiber.compiled_instruction = compiled_bytes;

    if let Some(fee) = priority_fee {
        fiber.priority_fee = fee;
    }

    // Reset execution stats
    fiber.last_executed = 0;
    fiber.exec_count = 0;

    Ok(())
}
