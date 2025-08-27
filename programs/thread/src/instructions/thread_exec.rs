use crate::{errors::*, *};
use anchor_lang::{
    prelude::*,
    solana_program::{program::invoke_signed, system_program, sysvar::recent_blockhashes},
};

/// Accounts required by the `thread_exec` instruction.
#[derive(Accounts)]
pub struct ThreadExec<'info> {
    /// The executor sending and paying for the transaction
    #[account(mut)]
    pub executor: Signer<'info>,

    /// The thread being executed
    #[account(
        mut,
        seeds = [
            SEED_THREAD,
            thread.authority.as_ref(),
            thread.id.as_slice(),
        ],
        bump = thread.bump,
        constraint = !thread.paused @ AntegenThreadError::ThreadPaused,
        constraint = !thread.fibers.is_empty() @ AntegenThreadError::InvalidThreadState,
    )]
    pub thread: Box<Account<'info, Thread>>,

    /// The fiber to execute
    #[account(
        mut,
        seeds = [
            SEED_THREAD_FIBER,
            thread.key().as_ref(),
            &[thread.exec_index],
        ],
        bump,
    )]
    pub fiber: Box<Account<'info, FiberState>>,

    /// The config for fee distribution
    #[account(
        seeds = [SEED_CONFIG],
        bump = config.bump,
    )]
    pub config: Account<'info, ThreadConfig>,

    // The config admin (for core team fee distribution)
    /// CHECK: This is validated by the config account
    #[account(
        mut,
        constraint = admin.key().eq(&config.admin) @ AntegenThreadError::InvalidConfigAdmin,
    )]
    pub admin: UncheckedAccount<'info>,

    /// The authority of the thread (for fee distribution)
    /// CHECK: This is validated by the thread account
    #[account(
        mut,
        constraint = thread_authority.key() == thread.authority @ AntegenThreadError::InvalidThreadAuthority,
    )]
    pub thread_authority: UncheckedAccount<'info>,

    /// Optional nonce account for durable nonces
    /// CHECK: For new nonce authority (optional - only required if thread has nonce account)
    #[account(mut)]
    pub nonce_account: Option<UncheckedAccount<'info>>,

    /// CHECK: Recent blockhashes sysvar (optional - only required if thread has nonce account)
    #[account(address = recent_blockhashes::ID)]
    pub recent_blockhashes: Option<UncheckedAccount<'info>>,

    #[account(address = system_program::ID)]
    pub system_program: Program<'info, System>,
}

pub fn thread_exec(ctx: Context<ThreadExec>, forgo_commission: bool) -> Result<()> {
    let clock: Clock = Clock::get()?;
    let thread: &mut Box<Account<Thread>> = &mut ctx.accounts.thread;
    let fiber: &mut Box<Account<FiberState>> = &mut ctx.accounts.fiber;
    let config: &Account<ThreadConfig> = &ctx.accounts.config;

    let executor: &mut Signer = &mut ctx.accounts.executor;
    let executor_lamports_start: u64 = executor.lamports();

    // Check global pause
    require!(
        !ctx.accounts.config.paused,
        AntegenThreadError::GlobalPauseActive
    );

    // Handle nonce using trait method
    thread.advance_nonce_if_required(
        &thread.to_account_info(),
        &ctx.accounts.nonce_account,
        &ctx.accounts.recent_blockhashes,
    )?;

    // Process trigger and get elapsed time
    let time_since_ready = thread.validate_and_update_context(&clock, ctx.remaining_accounts)?;
    let instruction = fiber.get_instruction(&executor.key())?;
    thread.sign(|seeds| invoke_signed(&instruction, ctx.remaining_accounts, &[seeds]))?;

    // Verify the inner instruction did not write data to the executor account
    require!(
        executor.data_is_empty(),
        AntegenThreadError::UnauthorizedWrite
    );

    let balance_change = executor.lamports() as i64 - executor_lamports_start as i64;
    let payments = config.calculate_payments(time_since_ready, balance_change, forgo_commission);

    // Log execution timing and commission details
    if forgo_commission && payments.executor_commission.eq(&0) {
        let effective_commission = config.calculate_effective_commission(time_since_ready);
        let forgone = config.calculate_executor_fee(effective_commission);
        msg!(
            "Executed {}s after trigger, forgoing {} commission",
            time_since_ready, forgone
        );
    } else {
        msg!("Executed {}s after trigger", time_since_ready);
    }

    // Distribute payments using thread trait
    thread.distribute_payments(
        &thread.to_account_info(),
        &executor.to_account_info(),
        &ctx.accounts.admin.to_account_info(),
        &payments,
    )?;

    // Update fiber tracking
    fiber.last_executed = clock.unix_timestamp;
    fiber.execution_count += 1;
    thread.exec_count += 1; // Increment thread-level execution counter
    thread.exec_index = (thread.exec_index + 1) % thread.fibers.len() as u8;

    Ok(())
}
