use crate::{errors::*, state::compile_instruction, *};
use anchor_lang::{
    prelude::*,
    solana_program::{instruction::Instruction, system_program},
};

/// Accounts required by the `fiber_create` instruction.
#[derive(Accounts)]
#[instruction(index: u8)]
pub struct FiberCreate<'info> {
    /// The authority of the thread or the thread itself
    #[account(
        constraint = authority.key().eq(&thread.authority) || authority.key().eq(&thread.key())
    )]
    pub authority: Signer<'info>,

    /// The payer for account initializations
    #[account(mut)]
    pub payer: Signer<'info>,

    /// The thread to add the fiber to
    #[account(
        mut,
        seeds = [
            SEED_THREAD,
            thread.authority.as_ref(),
            thread.id.as_slice(),
        ],
        bump = thread.bump,
    )]
    pub thread: Account<'info, Thread>,

    /// The fiber account to create
    #[account(
        init,
        seeds = [
            SEED_THREAD_FIBER,
            thread.key().as_ref(),
            &[index],
        ],
        bump,
        payer = payer,
        space = 8 + FiberState::INIT_SPACE
    )]
    pub fiber: Account<'info, FiberState>,

    #[account(address = system_program::ID)]
    pub system_program: Program<'info, System>,
}

pub fn fiber_create(
    ctx: Context<FiberCreate>,
    index: u8,
    instruction: Instruction,
    signer_seeds: Vec<Vec<Vec<u8>>>,
    priority_fee: u64,
) -> Result<()> {
    // Prevent thread_delete instructions in fibers
    if instruction.program_id.eq(&crate::ID)
        && instruction.data.len().ge(&8)
        && instruction.data[..8].eq(crate::instruction::DeleteThread::DISCRIMINATOR)
    {
        return Err(AntegenThreadError::InvalidInstruction.into());
    }

    let thread = &mut ctx.accounts.thread;
    let fiber = &mut ctx.accounts.fiber;

    // Compile the instruction
    let compiled = compile_instruction(instruction, signer_seeds)?;
    let compiled_bytes = compiled.try_to_vec()?;

    // Initialize the fiber
    fiber.thread = thread.key();
    fiber.index = index;
    fiber.compiled_instruction = compiled_bytes;
    fiber.priority_fee = priority_fee;

    // Update thread's fiber mapping
    if !thread.fibers.contains(&index) {
        thread.fibers.push(index);
        thread.fibers.sort();
    }

    Ok(())
}
