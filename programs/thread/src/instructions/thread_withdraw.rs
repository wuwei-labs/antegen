use {
    crate::{errors::*, state::*},
    anchor_lang::prelude::*,
};

/// Accounts required by the `thread_withdraw` instruction.
#[derive(Accounts)]
#[instruction(amount: u64)]
pub struct ThreadWithdraw<'info> {
    /// The authority (owner) of the thread.
    #[account(
        address = thread.authority
    )]
    pub authority: Signer<'info>,

    /// The thread to be.
    #[account(
        mut,
        seeds = [
            SEED_THREAD,
            thread.authority.as_ref(),
            thread.id.as_slice(),
        ],
        bump = thread.bump,
        has_one = authority,
    )]
    pub thread: Account<'info, Thread>,
}

pub fn handler(ctx: Context<ThreadWithdraw>, amount: u64) -> Result<()> {
    // Get accounts
    let authority = &mut ctx.accounts.authority;
    let thread = &mut ctx.accounts.thread;

    // Calculate the minimum rent threshold
    let data_len = 8 + thread.try_to_vec()?.len();
    let minimum_rent = Rent::get().unwrap().minimum_balance(data_len);
    let post_balance = thread
        .to_account_info()
        .lamports()
        .checked_sub(amount)
        .unwrap();
    require!(
        post_balance.gt(&minimum_rent),
        AntegenThreadError::WithdrawalTooLarge
    );

    // Withdraw balance from thread to the pay_to account
    **thread.to_account_info().try_borrow_mut_lamports()? = thread
        .to_account_info()
        .lamports()
        .checked_sub(amount)
        .unwrap();
    **authority.to_account_info().try_borrow_mut_lamports()? = authority
        .to_account_info()
        .lamports()
        .checked_add(amount)
        .unwrap();

    Ok(())
}
