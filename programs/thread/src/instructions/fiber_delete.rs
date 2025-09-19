use crate::*;
use anchor_lang::prelude::*;

/// Accounts required by the `fiber_delete` instruction.
#[derive(Accounts)]
#[instruction(index: u8)]
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

    /// The fiber account to delete
    #[account(
        mut,
        seeds = [
            SEED_THREAD_FIBER,
            thread.key().as_ref(),
            &[index],
        ],
        bump,
        close = close_to,
    )]
    pub fiber: Account<'info, FiberState>,
}

pub fn fiber_delete(ctx: Context<FiberDelete>, index: u8) -> Result<()> {
    let thread = &mut ctx.accounts.thread;

    // If we're deleting the current fiber, advance to next one first
    if thread.exec_index == index && thread.fibers.len() > 1 {
        thread.advance_to_next_fiber();
    }

    // Now remove the fiber index from the thread's mapping
    thread.fibers.retain(|&x| x != index);

    // If this was the last fiber, reset exec_index
    if thread.fibers.is_empty() {
        thread.exec_index = 0;
    }

    Ok(())
}
