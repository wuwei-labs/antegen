use crate::{errors::*, *};
use anchor_lang::prelude::*;
use antegen_fiber_program::{
    program::AntegenFiber,
    state::SerializableInstruction,
};

/// Accounts required by the `fiber_update` instruction.
/// Validates authority, CPIs to Fiber Program to update (or init) the fiber.
/// Thread PDA pays for fiber init if needed.
#[derive(Accounts)]
#[instruction(fiber_index: u8)]
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

    /// CHECK: The fiber account to update (may not exist yet, validated by Fiber Program via CPI)
    #[account(mut)]
    pub fiber: UncheckedAccount<'info>,

    /// The Fiber Program for CPI
    pub fiber_program: Program<'info, AntegenFiber>,

    pub system_program: Program<'info, System>,
}

pub fn fiber_update(
    ctx: Context<FiberUpdate>,
    fiber_index: u8,
    instruction: SerializableInstruction,
    priority_fee: Option<u64>,
    track: bool,
) -> Result<()> {
    // Prevent thread_delete instructions in fibers
    if instruction.program_id.eq(&crate::ID)
        && instruction.data.len() >= 8
        && instruction.data[..8].eq(crate::instruction::DeleteThread::DISCRIMINATOR)
    {
        return Err(AntegenThreadError::InvalidInstruction.into());
    }

    let thread = &mut ctx.accounts.thread;

    // Track the fiber in the thread's fiber_ids before CPI
    if track && !thread.fiber_ids.contains(&fiber_index) {
        thread.fiber_ids.push(fiber_index);
        thread.fiber_ids.sort();
        if fiber_index >= thread.fiber_next_id {
            thread.fiber_next_id = fiber_index.saturating_add(1);
        }
    }

    // Pre-fund fiber account from thread PDA if not yet initialized
    let fiber_info = ctx.accounts.fiber.to_account_info();
    if fiber_info.data_len() == 0 {
        let space = 8 + antegen_fiber_program::state::FiberState::INIT_SPACE;
        let rent_lamports = Rent::get()?.minimum_balance(space);
        **thread.to_account_info().try_borrow_mut_lamports()? -= rent_lamports;
        **fiber_info.try_borrow_mut_lamports()? += rent_lamports;
    }

    // CPI to Fiber Program's update_fiber
    thread.sign(|signer| {
        antegen_fiber_program::cpi::update_fiber(
            CpiContext::new_with_signer(
                ctx.accounts.fiber_program.key(),
                antegen_fiber_program::cpi::accounts::FiberUpdate {
                    thread: thread.to_account_info(),
                    fiber: ctx.accounts.fiber.to_account_info(),
                    system_program: ctx.accounts.system_program.to_account_info(),
                },
                &[signer],
            ),
            fiber_index,
            instruction,
            priority_fee,
        )
    })?;

    Ok(())
}
