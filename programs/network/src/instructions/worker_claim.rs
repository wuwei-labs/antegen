use {crate::state::*, anchor_lang::prelude::*};

#[derive(Accounts)]
pub struct WorkerClaim<'info> {
    #[account()]
    pub payer: Signer<'info>,

    /// CHECK: This account is unchecked because its validity is enforced by the
    /// #[account(address = worker.authority)] attribute, ensuring it matches
    /// the `authority` field in the `worker`. The external logic guarantees
    /// it is a valid account address.
    #[account(
        address = worker.authority
    )]
    pub authority: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [
            SEED_WORKER_COMMISSION,
            worker.key().as_ref(),
        ],
        bump,
    )]
    pub commission: Account<'info, WorkerCommission>,

    #[account(
        mut,
        seeds = [
            SEED_WORKER,
            worker.id.to_be_bytes().as_ref()
        ],
        bump,
        has_one = authority,
        constraint = commission.worker == worker.key()
    )]
    pub worker: Account<'info, Worker>,
}

pub fn handler(ctx: Context<WorkerClaim>) -> Result<()> {
    // Get accounts
    let commission = &ctx.accounts.commission;
    let pay_to = &ctx.accounts.authority;

    let commission_data_len = 8 + commission.try_to_vec()?.len();
    let commission_rent_balance = Rent::get()?.minimum_balance(commission_data_len);
    let commission_lamports = commission.to_account_info().lamports();
    let available_lamports = commission_lamports
        .checked_sub(commission_rent_balance)
        .unwrap_or(0);

    // Transfer commission to the pay_to account
    **commission.to_account_info().try_borrow_mut_lamports()? =
        commission_lamports.checked_sub(available_lamports).unwrap();
    **pay_to.to_account_info().try_borrow_mut_lamports()? = pay_to
        .to_account_info()
        .lamports()
        .checked_add(available_lamports)
        .unwrap();

    Ok(())
}
