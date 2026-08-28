use {
    crate::{errors::AntegenThreadError, *},
    anchor_lang::prelude::*,
    antegen_fiber_program::{
        cpi::close,
        state::{Fiber, FiberState},
    },
};

/// Accounts required by the `thread_close` instruction.
///
/// External fiber accounts (FiberState PDAs) should be passed via remaining_accounts.
/// Every id in `fiber_ids` must be accounted for — partial deletion is not
/// allowed. An id whose account no longer exists is accounted for by passing
/// that account anyway: the address still derives, and an empty slot is taken
/// as proof there is nothing left to close.
#[derive(Accounts)]
pub struct ThreadClose<'info> {
    /// The authority (owner) of the thread OR the thread itself (for self-deletion via CPI).
    #[account(
        constraint = authority.key().eq(&thread.authority) || authority.key().eq(&thread.key())
    )]
    pub authority: Signer<'info>,

    /// The address to return the data rent lamports to.
    ///
    /// Deliberately not pinned to `thread.authority`: the authority signs this
    /// instruction and may direct its own rent anywhere, which the CLI relies
    /// on. The one target that is never meaningful is the thread itself —
    /// Anchor would credit the account it is about to zero, burning the rent.
    #[account(mut)]
    pub close_to: SystemAccount<'info>,

    /// The thread to be closed.
    #[account(
        mut,
        close = close_to,
        constraint = thread.to_account_info().owner == &crate::ID
            @ AntegenThreadError::InvalidAccountOwner,
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

    // See `close_to` above: any target the authority names is fair game except
    // the thread itself, which would credit the rent to the account Anchor is
    // about to zero out.
    let close_to = ctx.accounts.close_to.key();
    if close_to == thread_key {
        return Err(AntegenThreadError::InvalidCloseTarget.into());
    }

    // Process each fiber account from remaining_accounts via CPI to Fiber Program
    for account in ctx.remaining_accounts.iter() {
        // Find which fiber_id this account corresponds to by checking PDA
        // derivation. Done before reading the account so that a fiber whose
        // account is gone can still be identified — derivation is what binds
        // the slot to this thread, and it holds whether or not anything is
        // there.
        let account_key = account.key();
        let pos = thread
            .fiber_ids
            .iter()
            .position(|&idx| FiberState::pubkey(thread_key, idx) == account_key)
            .ok_or(AntegenThreadError::InvalidFiberAccount)?;
        let fiber_index = thread.fiber_ids.remove(pos);

        // A tracked fiber whose account no longer exists. `fiber_ids` records
        // indices, and nothing on chain keeps that list in agreement with the
        // accounts those indices name — a fiber can be closed out from under a
        // thread that still lists it, which is what the 2026-08-26 rent sweep
        // did at scale.
        //
        // Dropping the id is the whole remedy: there is nothing left to close,
        // and no rent to return. Refusing to drop it is what made such a thread
        // impossible to close by *anyone* — passing the dead account failed to
        // deserialize, and omitting it left `fiber_ids` non-empty below. The
        // caller cannot resolve that from the outside, because only this
        // program can edit the list.
        if account.data_is_empty() {
            continue;
        }

        // Validate the slot holds a fiber (legacy or V1) belonging to this thread.
        let fiber_read = Fiber::try_deserialize(&mut &account.data.borrow()[..])?;
        require!(
            fiber_read.thread() == thread_key,
            AntegenThreadError::InvalidFiberAccount
        );

        // CPI to Fiber Program's close_fiber (rent returns to thread PDA)
        let fiber_program = ctx
            .accounts
            .fiber_program
            .as_ref()
            .ok_or(AntegenThreadError::MissingFiberAccounts)?;

        thread.sign(|seeds| {
            close(
                CpiContext::new_with_signer(
                    fiber_program.key(),
                    antegen_fiber_program::cpi::accounts::Close {
                        thread: thread.to_account_info(),
                        fiber: account.to_account_info(),
                    },
                    &[seeds],
                ),
                fiber_index,
            )
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
