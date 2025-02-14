use {crate::{state::*, TRANSACTION_BASE_FEE_REIMBURSEMENT}, anchor_lang::prelude::*};

/// Accounts required by the `thread_delete` instruction.
#[derive(Accounts)]
pub struct ThreadDelete<'info> {
    /// The authority (owner) of the thread.
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
        seeds = [
            SEED_THREAD,
            thread.authority.as_ref(),
            thread.id.as_slice(),
        ],
        bump = thread.bump
    )]
    pub thread: Account<'info, Thread>,
}

pub fn handler(ctx: Context<ThreadDelete>) -> Result<()> {
    let fee_payer = &ctx.accounts.to_account_infos()[0];
    let authority = &ctx.accounts.authority;
    let thread = &mut ctx.accounts.thread;
    let close_to = &ctx.accounts.close_to;

    // Get current lamports
    let initial_balance = thread.to_account_info().lamports();
    let fee_amount = TRANSACTION_BASE_FEE_REIMBURSEMENT + thread.fee;

    let final_balance = if authority.key().ne(&fee_payer.key()) {
        // Transfer fees to fee payer
        msg!("reimburse fee_payer...");
        **thread.to_account_info().try_borrow_mut_lamports()? = initial_balance
            .checked_sub(fee_amount)
            .unwrap();
        **fee_payer.try_borrow_mut_lamports()? = fee_payer
            .lamports()
            .checked_add(fee_amount)
            .unwrap();

        initial_balance.checked_sub(fee_amount).unwrap()
    } else {
        initial_balance
    };

    // Close thread
    **thread.to_account_info().try_borrow_mut_lamports()? = 0;
    **close_to.try_borrow_mut_lamports()? = close_to
        .lamports()
        .checked_add(final_balance)
        .unwrap();
    Ok(())
}