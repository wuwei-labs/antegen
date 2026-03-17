use crate::constants::*;
use crate::state::*;
use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::Instruction;

/// Accounts required by the `create_fiber` instruction.
/// Thread PDA is the signer (authority). A separate payer funds the account.
#[derive(Accounts)]
#[instruction(fiber_index: u8)]
pub struct FiberCreate<'info> {
    /// Thread PDA - signer (via invoke_signed from Thread Program)
    pub thread: Signer<'info>,

    /// Payer for the fiber account rent
    #[account(mut)]
    pub payer: Signer<'info>,

    /// The fiber account to create
    #[account(
        init,
        seeds = [SEED_THREAD_FIBER, thread.key().as_ref(), &[fiber_index]],
        bump,
        payer = payer,
        space = 8 + FiberState::INIT_SPACE,
    )]
    pub fiber: Account<'info, FiberState>,

    pub system_program: Program<'info, System>,
}

pub fn fiber_create(
    ctx: Context<FiberCreate>,
    instruction: Instruction,
    priority_fee: u64,
) -> Result<()> {
    let fiber = &mut ctx.accounts.fiber;

    // Compile the instruction
    let compiled = compile_instruction(instruction)?;
    let compiled_bytes = borsh::to_vec(&compiled)?;

    // Initialize the fiber
    fiber.thread = ctx.accounts.thread.key();
    fiber.compiled_instruction = compiled_bytes;
    fiber.priority_fee = priority_fee;
    fiber.last_executed = 0;
    fiber.exec_count = 0;

    Ok(())
}
