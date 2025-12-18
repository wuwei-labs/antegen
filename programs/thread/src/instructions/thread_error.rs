use crate::{errors::*, state::PaymentDetails, *};
use anchor_lang::prelude::*;

/// Accounts required by the `thread_error` instruction.
#[derive(Accounts)]
pub struct ThreadError<'info> {
    /// The executor reporting the error
    #[account(mut)]
    pub executor: Signer<'info>,

    /// The thread that failed to execute
    #[account(
        mut,
        seeds = [
            SEED_THREAD,
            thread.authority.as_ref(),
            thread.id.as_slice(),
        ],
        bump = thread.bump,
        constraint = !thread.paused @ AntegenThreadError::ThreadPaused,
    )]
    pub thread: Box<Account<'info, Thread>>,

    /// The config for calculating reimbursement
    #[account(
        seeds = [SEED_CONFIG],
        bump = config.bump,
    )]
    pub config: Account<'info, ThreadConfig>,

    /// The config admin (receives 0 fee but needed for payment distribution)
    /// CHECK: Validated by config
    #[account(
        mut,
        constraint = admin.key().eq(&config.admin) @ AntegenThreadError::InvalidConfigAdmin,
    )]
    pub admin: UncheckedAccount<'info>,

    #[account(address = anchor_lang::system_program::ID)]
    pub system_program: Program<'info, System>,
}

pub fn thread_error(
    ctx: Context<ThreadError>,
    error_code: u32,
    error_message: String,
) -> Result<()> {
    let clock = Clock::get()?;
    let thread = &mut ctx.accounts.thread;
    let config = &ctx.accounts.config;
    let executor = &ctx.accounts.executor;

    // Track starting balance for verification
    let executor_lamports_start = executor.lamports();

    // VALIDATION: Only allow error if we were the last executor (or no one was)
    // This prevents reporting errors for threads executed by others
    let last_executor_is_default = thread.last_executor == Pubkey::default();
    let we_were_last_executor = thread.last_executor == executor.key();

    require!(
        last_executor_is_default || we_were_last_executor,
        AntegenThreadError::NotLastExecutor
    );

    // VALIDATION 1: No error already reported
    require!(
        thread.last_error_time.is_none(),
        AntegenThreadError::ErrorAlreadyReported
    );

    // VALIDATION 2: Must be overdue beyond grace + decay period
    // Use the trait method to calculate time since ready
    let thread_pubkey = thread.key();
    let time_since_ready =
        thread.validate_trigger(&clock, ctx.remaining_accounts, &thread_pubkey)?;
    let error_threshold = config.grace_period_seconds + config.fee_decay_seconds;

    require!(
        time_since_ready >= error_threshold,
        AntegenThreadError::ThreadNotSufficientlyOverdue
    );

    // Calculate reimbursement (no commission on errors)
    const ERROR_REIMBURSEMENT: u64 = 10_000;

    let rent_sysvar = Rent::get()?;
    let available_lamports = thread
        .to_account_info()
        .lamports()
        .saturating_sub(rent_sysvar.minimum_balance(thread.to_account_info().data_len()));

    let payments = PaymentDetails {
        fee_payer_reimbursement: ERROR_REIMBURSEMENT.min(available_lamports),
        executor_commission: 0,
        core_team_fee: 0,
    };

    // Use the existing trait method to distribute payments
    thread.distribute_payments(
        &thread.to_account_info(),
        &executor.to_account_info(),
        &ctx.accounts.admin.to_account_info(),
        &payments,
    )?;

    // Set error timestamp
    thread.last_error_time = Some(clock.unix_timestamp);

    // Log the error details
    msg!(
        "ANTEGEN_ERROR: thread={}, executor={}, code={}, message={}, overdue_by={}s",
        thread.key(),
        executor.key(),
        error_code,
        error_message,
        time_since_ready
    );

    // Verify executor balance increased (reimbursement happened)
    let balance_change = executor.lamports() as i64 - executor_lamports_start as i64;
    require!(balance_change >= 0, AntegenThreadError::PaymentFailed);

    Ok(())
}
