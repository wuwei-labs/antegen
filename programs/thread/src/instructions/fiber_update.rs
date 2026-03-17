use crate::{errors::*, *};
use anchor_lang::prelude::*;
use antegen_fiber_program::state::SerializableInstruction;

/// Accounts required by the `fiber_update` instruction.
/// Validates authority, CPIs to Fiber Program to update the fiber.
#[derive(Accounts)]
pub struct FiberUpdate<'info> {
    /// The authority of the thread or the thread itself
    #[account(
        constraint = authority.key().eq(&thread.authority) || authority.key().eq(&thread.key())
    )]
    pub authority: Signer<'info>,

    /// The thread the fiber belongs to
    #[account(
        mut,
        seeds = [SEED_THREAD, thread.authority.as_ref(), thread.id.as_slice()],
        bump = thread.bump,
    )]
    pub thread: Account<'info, Thread>,

    /// The fiber account to update (must exist, owned by Fiber Program)
    #[account(mut)]
    pub fiber: Account<'info, antegen_fiber_program::state::FiberState>,

    /// The Fiber Program for CPI
    pub fiber_program: Program<'info, antegen_fiber_program::program::AntegenFiber>,
}

pub fn fiber_update(
    ctx: Context<FiberUpdate>,
    instruction: SerializableInstruction,
    priority_fee: Option<u64>,
) -> Result<()> {
    // Prevent thread_delete instructions in fibers
    if instruction.program_id.eq(&crate::ID)
        && instruction.data.len() >= 8
        && instruction.data[..8].eq(crate::instruction::DeleteThread::DISCRIMINATOR)
    {
        return Err(AntegenThreadError::InvalidInstruction.into());
    }

    let thread = &ctx.accounts.thread;

    // CPI to Fiber Program's update_fiber
    thread.sign(|signer| {
        antegen_fiber_program::cpi::update_fiber(
            CpiContext::new_with_signer(
                ctx.accounts.fiber_program.key(),
                antegen_fiber_program::cpi::accounts::FiberUpdate {
                    thread: thread.to_account_info(),
                    fiber: ctx.accounts.fiber.to_account_info(),
                },
                &[signer],
            ),
            instruction,
            priority_fee,
        )
    })?;

    Ok(())
}
