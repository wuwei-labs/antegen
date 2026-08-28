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

    /// CHECK: The thread to delete - deliberately not deserialized, so that
    /// threads too corrupt to decode can still be cleared. Ownership is still
    /// required, here and again in the handler.
    #[account(mut, owner = crate::ID)]
    pub thread: UncheckedAccount<'info>,
}

pub fn thread_delete(ctx: Context<ThreadDelete>) -> Result<()> {
    let admin = &ctx.accounts.admin;
    let thread = &ctx.accounts.thread;

    // The target is deliberately unchecked so that threads too corrupt to
    // decode can still be cleared. That must not extend to erasing arbitrary
    // accounts, so bound it to accounts this program owns.
    require_keys_eq!(
        *thread.to_account_info().owner,
        crate::ID,
        AntegenThreadError::InvalidAccountOwner
    );

    // Of the accounts this program owns, the config is the one that is never a
    // thread. Check the address and the discriminator: the address covers the
    // canonical config, the discriminator covers anything else that decodes as
    // one. A thread whose data is unreadable has neither and still gets swept.
    require_keys_neq!(
        thread.key(),
        ctx.accounts.config.key(),
        AntegenThreadError::InvalidAccountOwner
    );
    {
        /// Reads the leading 8-byte Anchor discriminator, if the account is
        /// long enough to have one. A thread too corrupt to decode may not be.
        fn try_from_slice(data: &[u8]) -> Result<Option<[u8; 8]>> {
            match data.get(..8) {
                None => Ok(None),
                Some(head) => head
                    .try_into()
                    .map(Some)
                    .map_err(|_| error!(AntegenThreadError::InvalidAccountOwner)),
            }
        }

        let data = thread.try_borrow_data()?;
        if let Some(disc) = try_from_slice(&data)? {
            require!(
                disc[..] != ThreadConfig::DISCRIMINATOR[..],
                AntegenThreadError::InvalidAccountOwner
            );
        }
    }

    // Transfer all lamports from thread to admin
    let thread_lamports = thread.lamports();
    **thread.try_borrow_mut_lamports()? = thread
        .lamports()
        .checked_sub(thread_lamports)
        .ok_or(AntegenThreadError::InvalidAccountOwner)?;
    **admin.try_borrow_mut_lamports()? = admin
        .lamports()
        .checked_add(thread_lamports)
        .ok_or(AntegenThreadError::InvalidAccountOwner)?;

    // Zero out account data to mark as closed
    thread.try_borrow_mut_data()?.fill(0);

    msg!("Deleting thread (admin)");
    Ok(())
}
