use {
    crate::{errors::AntegenThreadError, *},
    anchor_lang::prelude::*,
    antegen_fiber_program::{cpi::close, state::FiberState},
};

/// Accounts required by the `thread_close` instruction.
///
/// External fiber accounts (FiberState PDAs) should be passed via remaining_accounts.
/// All external fibers must be provided - partial deletion is not allowed.
#[derive(Accounts)]
pub struct ThreadClose<'info> {
    /// The authority (owner) of the thread OR the thread itself (for self-deletion via CPI).
    #[account(
        constraint = authority.key().eq(&thread.authority) || authority.key().eq(&thread.key())
    )]
    pub authority: Signer<'info>,

    /// The address to return the data rent lamports to.
    #[account(mut)]
    pub close_to: SystemAccount<'info>,

    /// The thread to be closed.
    #[account(
        mut,
        close = close_to,
        seeds = [
            SEED_THREAD,
            thread.authority.as_ref(),
            thread.id.as_slice(),
        ],
        bump = thread.bump
    )]
    pub thread: Account<'info, Thread>,

    /// The Fiber Program (required when closing fibers via remaining_accounts)
    pub fiber_program: Option<Program<'info, antegen_fiber_program::program::AntegenFiber>>,
}

pub fn thread_close<'info>(ctx: Context<'info, ThreadClose<'info>>) -> Result<()> {
    let thread = &mut ctx.accounts.thread;
    let thread_key = thread.key();

    // Process each fiber account from remaining_accounts via CPI to Fiber Program
    for account in ctx.remaining_accounts.iter() {
        // Deserialize to validate it's a FiberState
        let fiber = FiberState::try_deserialize(&mut &account.data.borrow()[..])?;

        // Validate fiber belongs to this thread
        require!(
            fiber.thread == thread_key,
            AntegenThreadError::InvalidFiberAccount
        );

        // Find which fiber_id this account corresponds to by checking PDA derivation
        let account_key = account.key();
        let pos = thread
            .fiber_ids
            .iter()
            .position(|&idx| FiberState::pubkey(thread_key, idx) == account_key)
            .ok_or(AntegenThreadError::InvalidFiberAccount)?;
        thread.fiber_ids.remove(pos);

        // CPI to Fiber Program's close_fiber (rent returns to thread PDA)
        let fiber_program = ctx
            .accounts
            .fiber_program
            .as_ref()
            .ok_or(AntegenThreadError::MissingFiberAccounts)?;

        thread.sign(|seeds| {
            close(CpiContext::new_with_signer(
                fiber_program.key(),
                antegen_fiber_program::cpi::accounts::Close {
                    thread: thread.to_account_info(),
                    fiber: account.to_account_info(),
                },
                &[seeds],
            ))
        })?;
    }

    // Validate ALL fibers were closed
    require!(
        thread.fiber_ids.is_empty(),
        AntegenThreadError::MissingFiberAccounts
    );

    // Anchor's close = close_to handles the thread account
    // (fiber rent returned to thread PDA is included in the transfer)
    Ok(())
}
