use {crate::*, anchor_lang::prelude::*};

/// Accounts required by the `thread_toggle` instruction.
#[derive(Accounts)]
pub struct ThreadToggle<'info> {
    /// The authority (owner) of the thread.
    #[account()]
    pub authority: Signer<'info>,

    /// The thread to toggle pause state.
    #[account(
        mut,
        has_one = authority,
        seeds = [
            SEED_THREAD,
            thread.authority.as_ref(),
            thread.id.as_slice(),
        ],
        bump = thread.bump,
    )]
    pub thread: Account<'info, Thread>,
}

pub fn thread_toggle(ctx: Context<ThreadToggle>) -> Result<()> {
    let thread = &mut ctx.accounts.thread;

    // Toggle the pause state
    thread.paused = !thread.paused;

    Ok(())
}
