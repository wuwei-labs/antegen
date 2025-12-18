use crate::{errors::*, state::compile_instruction, *};
use anchor_lang::{prelude::*, solana_program::instruction::Instruction};

/// Accounts required by the `fiber_update` instruction.
#[derive(Accounts)]
#[instruction(instruction: SerializableInstruction)]
pub struct FiberUpdate<'info> {
    /// The authority of the thread or the thread itself
    #[account(
        constraint = authority.key().eq(&thread.authority) || authority.key().eq(&thread.key())
    )]
    pub authority: Signer<'info>,

    /// The thread the fiber belongs to
    #[account(
        address = fiber.thread,
    )]
    pub thread: Account<'info, Thread>,

    /// The fiber account to update
    #[account(
        mut,
        constraint = fiber.thread == thread.key() @ AntegenThreadError::InvalidFiberAccount,
    )]
    pub fiber: Account<'info, FiberState>,
}

pub fn fiber_update(ctx: Context<FiberUpdate>, instruction: Instruction) -> Result<()> {
    // Prevent thread_delete instructions in fibers
    if instruction.program_id.eq(&crate::ID)
        && instruction.data.len() >= 8
        && instruction.data[..8].eq(crate::instruction::DeleteThread::DISCRIMINATOR)
    {
        return Err(AntegenThreadError::InvalidInstruction.into());
    }

    let fiber: &mut Account<'_, FiberState> = &mut ctx.accounts.fiber;
    let thread: &Account<'_, Thread> = &ctx.accounts.thread;

    let signer_seeds = vec![vec![
        SEED_THREAD.to_vec(),
        thread.authority.to_bytes().to_vec(),
        thread.id.clone(),
    ]];

    // Compile the new instruction
    let compiled: CompiledInstructionV0 = compile_instruction(instruction, signer_seeds)?;
    let compiled_bytes: Vec<u8> = borsh::to_vec(&compiled)?;

    // Update fiber with new instruction and reset stats
    fiber.compiled_instruction = compiled_bytes;
    fiber.last_executed = 0;
    fiber.exec_count = 0;

    Ok(())
}
