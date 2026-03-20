use crate::{
    errors::*,
    state::{decompile_instruction, CompiledInstructionV0, Signal},
    *,
};
use anchor_lang::{
    prelude::*,
    solana_program::program::{get_return_data, invoke_signed},
};

/// Accounts required by the `thread_exec` instruction.
#[derive(Accounts)]
#[instruction(forgo_commission: bool, fiber_cursor: u8)]
pub struct ThreadExec<'info> {
    /// The executor sending and paying for the transaction
    #[account(mut)]
    pub executor: Signer<'info>,

    /// The thread being executed
    /// Note: `dup` allows thread to appear in remaining_accounts (from compiled instruction)
    #[account(
        mut,
        dup,
        seeds = [
            SEED_THREAD,
            thread.authority.as_ref(),
            thread.id.as_slice(),
        ],
        bump = thread.bump,
        constraint = !thread.paused @ AntegenThreadError::ThreadPaused,
        constraint = !thread.fiber_ids.is_empty() @ AntegenThreadError::InvalidThreadState,
    )]
    pub thread: Box<Account<'info, Thread>>,

    /// The fiber to execute (owned by Fiber Program)
    pub fiber: Box<Account<'info, antegen_fiber_program::state::FiberState>>,

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

    /// Optional nonce account for durable nonces
    /// CHECK: Only required if thread has nonce account
    #[account(mut)]
    pub nonce_account: Option<UncheckedAccount<'info>>,

    /// CHECK: Recent blockhashes sysvar (optional - only required if thread has nonce account)
    pub recent_blockhashes: Option<UncheckedAccount<'info>>,

    #[account(address = anchor_lang::system_program::ID)]
    pub system_program: Program<'info, System>,
}

pub fn thread_exec(
    ctx: Context<ThreadExec>,
    forgo_commission: bool,
    fiber_cursor: u8,
) -> Result<()> {
    // ── Setup ──
    let clock: Clock = Clock::get()?;
    let thread: &mut Box<Account<Thread>> = &mut ctx.accounts.thread;
    let config: &Account<ThreadConfig> = &ctx.accounts.config;
    let executor: &mut Signer = &mut ctx.accounts.executor;
    let executor_lamports_start: u64 = executor.lamports();
    let thread_pubkey = thread.key();

    require!(
        !ctx.accounts.config.paused,
        AntegenThreadError::GlobalPauseActive
    );

    // ── Close path (early return) ──
    if thread.fiber_signal == Signal::Close {
        let compiled = CompiledInstructionV0::try_from_slice(&thread.close_fiber)?;
        let instruction = decompile_instruction(&compiled)?;

        msg!("Executing close_fiber to delete thread");

        thread.sign(|seeds| invoke_signed(&instruction, ctx.remaining_accounts, &[seeds]))?;

        return Ok(());
    }

    // ── Chaining detection ──
    let is_chained = thread.fiber_signal.eq(&Signal::Chain);

    // Sync fiber_cursor so advance_to_next_fiber works correctly
    if is_chained {
        thread.fiber_cursor = fiber_cursor;
    }

    // ── Pre-execution checks ──
    thread.validate_for_execution()?;

    thread.advance_nonce_if_required(
        &thread.to_account_info(),
        &ctx.accounts.nonce_account,
        &ctx.accounts.recent_blockhashes,
    )?;

    let time_since_ready = if is_chained {
        msg!("Chained execution");
        0
    } else {
        thread.validate_trigger(&clock, ctx.remaining_accounts, &thread_pubkey)?
    };

    // ── Execute fiber ──
    let fiber = &ctx.accounts.fiber;

    let expected_fiber = thread.fiber_at_index(&thread_pubkey, fiber_cursor);
    require!(
        fiber.key().eq(&expected_fiber),
        AntegenThreadError::WrongFiberIndex
    );

    let instruction = fiber.get_instruction(&executor.key())?;

    thread.sign(|seeds| invoke_signed(&instruction, ctx.remaining_accounts, &[seeds]))?;

    // Verify the CPI did not write data to the executor account
    require!(
        executor.data_is_empty(),
        AntegenThreadError::UnauthorizedWrite
    );

    // ── Parse signal ──
    let signal: Signal = match get_return_data() {
        None => Signal::None,
        Some((program_id, return_data)) => {
            if program_id.eq(&instruction.program_id) {
                Signal::try_from_slice(return_data.as_slice()).unwrap_or(Signal::None)
            } else {
                Signal::None
            }
        }
    };

    // Downgrade Chain → None if cursor is on last fiber (nowhere to chain to)
    let last_fiber = thread.fiber_ids.last().copied().unwrap_or(fiber_cursor);
    let signal = if signal.eq(&Signal::Chain) && last_fiber.eq(&fiber_cursor) {
        Signal::None
    } else {
        signal
    };

    // ── Payments (when chain ends) ──
    if signal.ne(&Signal::Chain) {
        let balance_change = executor.lamports() as i64 - executor_lamports_start as i64;
        let payments =
            config.calculate_payments(time_since_ready, balance_change, forgo_commission);

        if forgo_commission && payments.executor_commission.eq(&0) {
            let effective_commission = config.calculate_effective_commission(time_since_ready);
            let forgone = config.calculate_executor_fee(effective_commission);
            msg!(
                "Executed {}s after trigger, forgoing {} commission",
                time_since_ready,
                forgone
            );
        } else {
            msg!("Executed {}s after trigger", time_since_ready);
        }

        thread.distribute_payments(
            &thread.to_account_info(),
            &executor.to_account_info(),
            &ctx.accounts.admin.to_account_info(),
            &payments,
        )?;
    }

    // ── Apply signal to thread state ──
    // Only persist Chain/Close — the executor needs these between transactions.
    // All other signals are consumed inline and fiber_signal resets to None.
    thread.fiber_signal = Signal::None;
    match &signal {
        Signal::Chain | Signal::Close => {
            thread.fiber_signal = signal.clone();
        }
        Signal::Next { index } => {
            thread.fiber_cursor = *index;
        }
        Signal::Update { paused, trigger } => {
            if let Some(paused) = paused {
                thread.paused = *paused;
            }
            if let Some(trigger) = trigger {
                thread.trigger = trigger.clone();
            }
            thread.advance_to_next_fiber();
        }
        Signal::Repeat => {
            // Keep cursor on current fiber — no advancement
        }
        Signal::None => {
            thread.advance_to_next_fiber();
        }
    }

    // Immediate triggers: auto-close after fiber completes (unless chaining)
    if matches!(thread.trigger, Trigger::Immediate { .. }) && signal != Signal::Chain {
        thread.fiber_signal = Signal::Close;
    }

    // ── Finalize ──
    if signal != Signal::Chain {
        thread.update_schedule(&clock, ctx.remaining_accounts, &thread_pubkey)?;
    }

    // Fiber stats not updated — fiber is owned by Fiber Program
    thread.exec_count += 1;
    thread.last_executor = executor.key();

    Ok(())
}
