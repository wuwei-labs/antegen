use crate::*;
use anchor_lang::prelude::*;

/// Accounts required by the `fiber_delete` instruction.
#[derive(Accounts)]
#[instruction(fiber_index: u8)]
pub struct FiberDelete<'info> {
    /// The authority of the thread or the thread itself
    #[account(
        constraint = authority.key().eq(&thread.authority) || authority.key().eq(&thread.key())
    )]
    pub authority: Signer<'info>,

    /// The address to return the data rent lamports to
    #[account(mut)]
    pub close_to: SystemAccount<'info>,

    /// The thread to remove the fiber from
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

    /// The fiber account to delete (optional - not needed if deleting inline fiber)
    #[account(
        mut,
        seeds = [
            SEED_THREAD_FIBER,
            thread.key().as_ref(),
            &[fiber_index],
        ],
        bump,
        close = close_to,
    )]
    pub fiber: Option<Account<'info, FiberState>>,
}

pub fn fiber_delete(ctx: Context<FiberDelete>, fiber_index: u8) -> Result<()> {
    let thread = &mut ctx.accounts.thread;

    // Check if deleting default fiber (index 0 with default_fiber present)
    if fiber_index == 0 && thread.default_fiber.is_some() {
        // Clear default fiber
        thread.default_fiber = None;
        thread.default_fiber_priority_fee = 0;

        // If we're deleting the current fiber, advance to next one first
        if thread.fiber_cursor == 0 && thread.fiber_ids.len() > 1 {
            thread.advance_to_next_fiber();
        }

        // Remove from fiber_ids
        thread.fiber_ids.retain(|&x| x != 0);

        // If this was the last fiber, reset fiber_cursor
        if thread.fiber_ids.is_empty() {
            thread.fiber_cursor = 0;
        }
    } else {
        // Deleting account-based fiber - ensure account is provided
        require!(
            ctx.accounts.fiber.is_some(),
            crate::errors::AntegenThreadError::FiberAccountRequired
        );

        // If we're deleting the current fiber, advance to next one first
        if thread.fiber_cursor == fiber_index && thread.fiber_ids.len() > 1 {
            thread.advance_to_next_fiber();
        }

        // Remove the fiber index from the thread's fiber_ids
        thread.fiber_ids.retain(|&x| x != fiber_index);

        // If this was the last fiber, reset fiber_cursor
        if thread.fiber_ids.is_empty() {
            thread.fiber_cursor = 0;
        }

        // Account closure is handled by Anchor's close constraint
    }

    Ok(())
}
