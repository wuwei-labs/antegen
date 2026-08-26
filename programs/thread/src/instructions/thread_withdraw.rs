use {
    crate::{errors::*, *},
    anchor_lang::prelude::*,
};

/// Accounts required by the `thread_withdraw` instruction.
#[derive(Accounts)]
#[instruction(amount: u64)]
pub struct ThreadWithdraw<'info> {
    /// The authority (owner) of the thread.
    #[account()]
    pub authority: Signer<'info>,

    /// CHECK: The account to withdraw lamports to.
    #[account(mut)]
    pub pay_to: UncheckedAccount<'info>,

    /// The thread to be.
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

pub fn thread_withdraw(ctx: Context<ThreadWithdraw>, amount: u64) -> Result<()> {
    // Get accounts
    let pay_to = &mut ctx.accounts.pay_to;
    let thread = &mut ctx.accounts.thread;

    // Size rent from what the account occupies, not from what the struct
    // currently serializes to. `Vec`/`String` fields are allocated at their
    // `max_len`, so a real thread serializes to a fraction of its allocation
    // — measuring the serialization sets the floor hundreds of bytes too low.
    //
    // The runtime is the real backstop here: it rejects any transaction that
    // leaves a writable account below rent exemption, so the loose floor was
    // never a way to strand a thread. It only meant a withdrawal the program
    // said was fine failed later as `InsufficientFundsForRent`, which says
    // nothing about which account or why. Failing here returns
    // `WithdrawalTooLarge` instead.
    let minimum_rent = Rent::get()?.minimum_balance(thread.to_account_info().data_len());
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
    **thread.to_account_info().try_borrow_mut_lamports()? -= amount;
    **pay_to.try_borrow_mut_lamports()? += amount;
    Ok(())
}
