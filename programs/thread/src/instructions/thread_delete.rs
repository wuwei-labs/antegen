use {
    crate::{errors::AntegenThreadError, state::FiberState, *},
    anchor_lang::prelude::*,
};

/// Accounts required by the `thread_delete` instruction.
///
/// External fiber accounts (FiberState PDAs) should be passed via remaining_accounts.
/// All external fibers must be provided - partial deletion is not allowed.
#[derive(Accounts)]
pub struct ThreadDelete<'info> {
    /// The authority (owner) of the thread OR the thread itself (for self-deletion via CPI).
    #[account(
        constraint = authority.key().eq(&thread.authority) || authority.key().eq(&thread.key())
    )]
    pub authority: Signer<'info>,

    /// The address to return the data rent lamports to.
    #[account(mut)]
    pub close_to: SystemAccount<'info>,

    /// The thread to be deleted.
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
}

pub fn thread_delete(ctx: Context<ThreadDelete>) -> Result<()> {
    let thread = &mut ctx.accounts.thread;
    let close_to = &ctx.accounts.close_to;
    let thread_key = thread.key();

    // Process each fiber account from remaining_accounts
    for account in ctx.remaining_accounts.iter() {
        // Deserialize to validate it's a FiberState
        let fiber = FiberState::try_deserialize(&mut &account.data.borrow()[..])?;

        // Validate fiber belongs to this thread
        require!(
            fiber.thread == thread_key,
            AntegenThreadError::InvalidFiberAccount
        );

        // Remove fiber_index from fiber_ids (fails if not found/duplicate)
        let pos = thread
            .fiber_ids
            .iter()
            .position(|&idx| idx == fiber.fiber_index)
            .ok_or(AntegenThreadError::InvalidFiberAccount)?;
        thread.fiber_ids.remove(pos);

        // Transfer lamports to close_to
        let lamports = account.lamports();
        **account.try_borrow_mut_lamports()? = 0;
        **close_to.to_account_info().try_borrow_mut_lamports()? += lamports;

        // Zero account data & reassign owner to system program
        account.try_borrow_mut_data()?.fill(0);
        account.assign(&anchor_lang::system_program::ID);
    }

    // Validate ALL external fibers were closed
    // After processing: fiber_ids should only contain inline fiber (0) or be empty
    let valid_end_state = if thread.default_fiber.is_some() {
        thread.fiber_ids == vec![0]
    } else {
        thread.fiber_ids.is_empty()
    };
    require!(valid_end_state, AntegenThreadError::MissingFiberAccounts);

    // Anchor's close = close_to handles the thread account
    Ok(())
}
