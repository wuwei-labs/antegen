use crate::{errors::AntegenThreadError, state::ThreadConfig, *};
use anchor_lang::prelude::*;

/// Force delete a thread - admin only, skips all checks
/// Used for cleaning up stuck/broken threads during development
#[derive(Accounts)]
pub struct ThreadForceDelete<'info> {
    /// The config admin (must sign)
    #[account(
        mut,
        constraint = admin.key() == config.admin @ AntegenThreadError::InvalidConfigAdmin,
    )]
    pub admin: Signer<'info>,

    /// The config account
    #[account(
        seeds = [SEED_CONFIG],
        bump = config.bump,
    )]
    pub config: Account<'info, ThreadConfig>,

    /// The thread to force delete
    #[account(
        mut,
        close = admin,
    )]
    pub thread: Account<'info, Thread>,
}

pub fn thread_force_delete(_ctx: Context<ThreadForceDelete>) -> Result<()> {
    msg!("Force deleting thread (admin override)");
    Ok(())
}
