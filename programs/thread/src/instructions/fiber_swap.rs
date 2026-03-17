use crate::{errors::AntegenThreadError, *};
use anchor_lang::prelude::*;

/// Accounts required by the `swap_fiber` instruction.
/// Copies source fiber's instruction into target, closes source.
/// Validates authority, CPIs to Fiber Program to swap, updates thread fiber tracking.
#[derive(Accounts)]
#[instruction(source_fiber_index: u8)]
pub struct FiberSwap<'info> {
    /// The authority of the thread or the thread itself
    #[account(
        constraint = authority.key().eq(&thread.authority) || authority.key().eq(&thread.key())
    )]
    pub authority: Signer<'info>,

    /// The thread that owns both fibers
    #[account(
        mut,
        constraint = thread.fiber_ids.contains(&source_fiber_index) @ AntegenThreadError::InvalidFiberIndex,
        seeds = [
            SEED_THREAD,
            thread.authority.as_ref(),
            thread.id.as_slice(),
        ],
        bump = thread.bump,
    )]
    pub thread: Account<'info, Thread>,

    /// The target fiber — receives source's instruction content
    #[account(
        mut,
        constraint = target.thread.eq(&thread.key()) @ AntegenThreadError::InvalidFiberAccount,
    )]
    pub target: Account<'info, antegen_fiber_program::state::FiberState>,

    /// The source fiber — closed after its instruction is copied to target
    #[account(
        mut,
        constraint = source.thread.eq(&thread.key()) @ AntegenThreadError::InvalidFiberAccount,
    )]
    pub source: Account<'info, antegen_fiber_program::state::FiberState>,

    /// The Fiber Program for CPI
    pub fiber_program: Program<'info, antegen_fiber_program::program::AntegenFiber>,
}

pub fn fiber_swap(ctx: Context<FiberSwap>, source_fiber_index: u8) -> Result<()> {
    let thread = &mut ctx.accounts.thread;

    // CPI to Fiber Program's swap_fiber
    thread.sign(|seeds| {
        antegen_fiber_program::cpi::swap_fiber(CpiContext::new_with_signer(
            ctx.accounts.fiber_program.key(),
            antegen_fiber_program::cpi::accounts::FiberSwap {
                thread: thread.to_account_info(),
                target: ctx.accounts.target.to_account_info(),
                source: ctx.accounts.source.to_account_info(),
            },
            &[seeds],
        ))
    })?;

    // Remove source fiber index from fiber_ids
    thread.fiber_ids.retain(|&x| x != source_fiber_index);

    // Adjust cursor if it was pointing to the removed source
    if thread.fiber_cursor == source_fiber_index {
        if thread.fiber_ids.is_empty() {
            thread.fiber_cursor = 0;
        } else {
            thread.fiber_cursor = thread.fiber_ids[0];
        }
    }

    Ok(())
}
