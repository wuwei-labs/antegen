use crate::{errors::*, *};
use anchor_lang::prelude::*;
use antegen_fiber_program::state::SerializableInstruction;

/// Accounts required by the `fiber_create` instruction.
/// Validates authority, pre-funds fiber from thread, CPIs to Fiber Program to create,
/// updates thread fiber tracking.
#[derive(Accounts)]
#[instruction(fiber_index: u8)]
pub struct FiberCreate<'info> {
    /// The authority of the thread or the thread itself
    #[account(
        constraint = authority.key().eq(&thread.authority) || authority.key().eq(&thread.key())
    )]
    pub authority: Signer<'info>,

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

    /// CHECK: Initialized by Fiber Program via CPI
    #[account(mut)]
    pub fiber: UncheckedAccount<'info>,

    /// The Fiber Program for CPI
    pub fiber_program: Program<'info, antegen_fiber_program::program::AntegenFiber>,

    #[account(address = anchor_lang::system_program::ID)]
    pub system_program: Program<'info, System>,
}

pub fn fiber_create(
    ctx: Context<FiberCreate>,
    fiber_index: u8,
    instruction: SerializableInstruction,
    priority_fee: u64,
) -> Result<()> {
    let thread = &mut ctx.accounts.thread;

    // Prevent thread_delete instructions in fibers
    if instruction.program_id.eq(&crate::ID)
        && instruction.data.len().ge(&8)
        && instruction.data[..8].eq(crate::instruction::DeleteThread::DISCRIMINATOR)
    {
        return Err(AntegenThreadError::InvalidInstruction.into());
    }

    // Conditional pre-funding: only pre-fund if fiber account is not yet initialized
    if ctx.accounts.fiber.to_account_info().data_len() == 0 {
        let space = 8 + antegen_fiber_program::state::FiberState::INIT_SPACE;
        let rent_lamports = Rent::get()?.minimum_balance(space);
        **thread.to_account_info().try_borrow_mut_lamports()? -= rent_lamports;
        **ctx.accounts.fiber.to_account_info().try_borrow_mut_lamports()? += rent_lamports;
    }

    thread.sign(|seeds| {
        antegen_fiber_program::cpi::create(
            CpiContext::new_with_signer(
                ctx.accounts.fiber_program.key(),
                antegen_fiber_program::cpi::accounts::Create {
                    thread: thread.to_account_info(),
                    fiber: ctx.accounts.fiber.to_account_info(),
                    system_program: ctx.accounts.system_program.to_account_info(),
                },
                &[seeds],
            ),
            fiber_index,
            instruction,
            priority_fee,
        )
    })?;

    // Track fiber: push to fiber_ids if not present, bump fiber_next_id if needed
    if !thread.fiber_ids.contains(&fiber_index) {
        thread.fiber_ids.push(fiber_index);
        thread.fiber_ids.sort();
    }
    if fiber_index >= thread.fiber_next_id {
        thread.fiber_next_id = fiber_index.saturating_add(1);
    }

    Ok(())
}
