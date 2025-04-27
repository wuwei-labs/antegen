use {crate::state::*, anchor_lang::prelude::*};

/// Accounts required by the `thread_delete` instruction.
#[derive(Accounts)]
pub struct ThreadPause<'info> {
    /// The authority (owner) of the thread.
    #[account()]
    pub authority: Signer<'info>,

    /// The thread to be paused.
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

pub fn handler(ctx: Context<ThreadPause>) -> Result<()> {
    let thread = &mut ctx.accounts.thread;

    // Pause the thread
    thread.paused = true;

    Ok(())
}
