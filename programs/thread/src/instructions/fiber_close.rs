use crate::{errors::AntegenThreadError, *};
use anchor_lang::prelude::*;

/// Accounts required by the `fiber_close` instruction.
/// Validates authority, CPIs to Fiber Program to close, updates thread fiber tracking.
#[derive(Accounts)]
#[instruction(fiber_index: u8)]
pub struct FiberClose<'info> {
    /// The authority of the thread or the thread itself
    #[account(
        constraint = authority.key().eq(&thread.authority) || authority.key().eq(&thread.key())
    )]
    pub authority: Signer<'info>,

    /// The thread to remove the fiber from
    #[account(
        mut,
        constraint = thread.fiber_ids.contains(&fiber_index) @ AntegenThreadError::InvalidFiberIndex,
        seeds = [
            SEED_THREAD,
            thread.authority.as_ref(),
            thread.id.as_slice(),
        ],
        bump = thread.bump,
    )]
    pub thread: Account<'info, Thread>,

    /// The fiber account to close (owned by Fiber Program)
    #[account(
        mut,
        constraint = fiber.thread.eq(&thread.key()) @ AntegenThreadError::InvalidFiberAccount,
    )]
    pub fiber: Account<'info, antegen_fiber_program::state::FiberState>,

    /// The Fiber Program for CPI
    pub fiber_program: Program<'info, antegen_fiber_program::program::FiberProgram>,
}

pub fn fiber_close(ctx: Context<FiberClose>, fiber_index: u8) -> Result<()> {
    let thread = &mut ctx.accounts.thread;

    // If we're closing the current fiber, advance to next one first
    if thread.fiber_cursor.eq(&fiber_index) && thread.fiber_ids.len().gt(&1) {
        thread.advance_to_next_fiber();
    }

    thread.fiber_ids.retain(|&x| x != fiber_index);
    if thread.fiber_ids.is_empty() {
        thread.fiber_cursor = 0;
    }

    thread.sign(|seeds| {
        antegen_fiber_program::cpi::close_fiber(CpiContext::new_with_signer(
            ctx.accounts.fiber_program.key(),
            antegen_fiber_program::cpi::accounts::FiberClose {
                thread: thread.to_account_info(),
                fiber: ctx.accounts.fiber.to_account_info(),
            },
            &[seeds],
        ))
    })?;

    Ok(())
}
