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

    /// CHECK: fiber account to close (owned by Fiber Program). Shape-agnostic
    /// — the fiber program validates the `thread` field against the signer.
    ///
    /// Bound to `fiber_index` here as well, because this instruction drops that
    /// index from `fiber_ids` regardless of which account it hands the fiber
    /// program. Without the binding, naming index N while passing index M's
    /// account retires N in the thread's bookkeeping and destroys M — leaving
    /// the thread pointing at an account that no longer exists and stranding
    /// N's rent in an account nothing tracks.
    #[account(
        mut,
        seeds = [SEED_THREAD_FIBER, thread.key().as_ref(), &[fiber_index]],
        bump,
        seeds::program = antegen_fiber_program::ID,
    )]
    pub fiber: UncheckedAccount<'info>,

    /// The Fiber Program for CPI
    pub fiber_program: Program<'info, antegen_fiber_program::program::AntegenFiber>,
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
        antegen_fiber_program::cpi::close(
            CpiContext::new_with_signer(
                ctx.accounts.fiber_program.key(),
                antegen_fiber_program::cpi::accounts::Close {
                    thread: thread.to_account_info(),
                    fiber: ctx.accounts.fiber.to_account_info(),
                },
                &[seeds],
            ),
            fiber_index,
        )
    })?;

    Ok(())
}
