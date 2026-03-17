use crate::state::FiberState;
use anchor_lang::prelude::*;

/// Accounts required by the `close_fiber` instruction.
/// Thread PDA is signer, receives rent back.
#[derive(Accounts)]
pub struct FiberClose<'info> {
    /// Thread PDA - signer, receives rent back
    #[account(mut)]
    pub thread: Signer<'info>,

    /// The fiber to close
    #[account(mut, has_one = thread, close = thread)]
    pub fiber: Account<'info, FiberState>,
}

pub fn fiber_close(_ctx: Context<FiberClose>) -> Result<()> {
    Ok(())
}
