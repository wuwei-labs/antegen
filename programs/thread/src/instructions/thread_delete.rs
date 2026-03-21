use crate::{errors::AntegenThreadError, state::ThreadConfig, *};
use anchor_lang::prelude::*;

/// Force delete a thread - admin only, skips all checks.
/// Used for cleaning up stuck/broken threads that may not deserialize.
#[derive(Accounts)]
pub struct ThreadDelete<'info> {
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

    /// CHECK: The thread to delete - unchecked so we can close broken/undeserializable accounts
    #[account(mut)]
    pub thread: UncheckedAccount<'info>,
}

pub fn thread_delete(ctx: Context<ThreadDelete>) -> Result<()> {
    let admin = &ctx.accounts.admin;
    let thread = &ctx.accounts.thread;

    // Transfer all lamports from thread to admin
    let thread_lamports = thread.lamports();
    **thread.try_borrow_mut_lamports()? -= thread_lamports;
    **admin.try_borrow_mut_lamports()? += thread_lamports;

    // Zero out account data to mark as closed
    thread.try_borrow_mut_data()?.fill(0);

    msg!("Deleting thread (admin)");
    Ok(())
}
