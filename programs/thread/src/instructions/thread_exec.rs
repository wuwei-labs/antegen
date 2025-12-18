use crate::{
    errors::*,
    state::{decompile_instruction, CompiledInstructionV0, Signal, PAYER_PUBKEY},
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

    /// The fiber to execute (optional - not needed if fiber_cursor == 0 and default fiber exists)
    /// Seeds validation is done in the instruction body when fiber is Some
    pub fiber: Option<Box<Account<'info, FiberState>>>,

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
    let clock: Clock = Clock::get()?;
    let thread: &mut Box<Account<Thread>> = &mut ctx.accounts.thread;
    let config: &Account<ThreadConfig> = &ctx.accounts.config;

    let executor: &mut Signer = &mut ctx.accounts.executor;
    let executor_lamports_start: u64 = executor.lamports();

    // Check global pause
    require!(
        !ctx.accounts.config.paused,
        AntegenThreadError::GlobalPauseActive
    );

    let thread_pubkey = thread.key();

    // Handle close_fiber execution when Signal::Close is set
    if thread.fiber_signal == Signal::Close {
        // Decompile and execute the close_fiber (CPIs to thread_delete)
        let compiled = CompiledInstructionV0::try_from_slice(&thread.close_fiber)?;
        let instruction = decompile_instruction(&compiled)?;

        msg!("Executing close_fiber to delete thread");

        // Invoke thread_delete via CPI with thread signing as authority
        thread.sign(|seeds| invoke_signed(&instruction, ctx.remaining_accounts, &[seeds]))?;

        // Thread is now closed by thread_delete, nothing more to do
        return Ok(());
    }

    // Check if this is a chained execution (previous fiber signaled Chain)
    let is_chained = thread.fiber_signal == Signal::Chain;

    // Sync fiber_cursor for chained executions so advance_to_next_fiber works correctly
    if is_chained {
        thread.fiber_cursor = fiber_cursor;
    }

    // Normal fiber execution path
    // Validate thread is ready for execution (has fibers and valid exec_index)
    thread.validate_for_execution()?;

    // Handle nonce using trait method
    thread.advance_nonce_if_required(
        &thread.to_account_info(),
        &ctx.accounts.nonce_account,
        &ctx.accounts.recent_blockhashes,
    )?;

    // Validate trigger and get elapsed time (skip for chained executions)
    let time_since_ready = if is_chained {
        msg!(
            "Chained execution from fiber_signal={:?}",
            thread.fiber_signal
        );
        0 // No elapsed time for chained executions
    } else {
        thread.validate_trigger(&clock, ctx.remaining_accounts, &thread_pubkey)?
    };

    // Get instruction from default fiber or fiber account
    let (instruction, _priority_fee, is_inline) =
        if fiber_cursor == 0 && thread.default_fiber.is_some() {
            // Use default fiber at index 0
            let compiled =
                CompiledInstructionV0::try_from_slice(thread.default_fiber.as_ref().unwrap())?;
            let mut ix = decompile_instruction(&compiled)?;

            // Replace PAYER_PUBKEY with executor
            for acc in ix.accounts.iter_mut() {
                if acc.pubkey.eq(&PAYER_PUBKEY) {
                    acc.pubkey = executor.key();
                }
            }

            (ix, thread.default_fiber_priority_fee, true)
        } else {
            // Use fiber account at fiber_cursor
            let fiber = ctx
                .accounts
                .fiber
                .as_ref()
                .ok_or(AntegenThreadError::FiberAccountRequired)?;

            // Verify we're loading the correct fiber account
            let expected_fiber = thread.fiber_at_index(&thread_pubkey, fiber_cursor);
            require!(
                fiber.key() == expected_fiber,
                AntegenThreadError::WrongFiberIndex
            );

            (
                fiber.get_instruction(&executor.key())?,
                fiber.priority_fee,
                false,
            )
        };

    // Invoke the instruction
    thread.sign(|seeds| invoke_signed(&instruction, ctx.remaining_accounts, &[seeds]))?;

    // Verify the inner instruction did not write data to the executor account
    require!(
        executor.data_is_empty(),
        AntegenThreadError::UnauthorizedWrite
    );

    // Parse the signal from return data and store for executor to read
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

    // Calculate and distribute payments when chain ends (signal != Chain)
    // This ensures fees are calculated once at the end of a chain, capturing total balance change
    if signal != Signal::Chain {
        let balance_change = executor.lamports() as i64 - executor_lamports_start as i64;
        let payments =
            config.calculate_payments(time_since_ready, balance_change, forgo_commission);

        // Log execution timing and commission details
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

        // Distribute payments using thread trait
        thread.distribute_payments(
            &thread.to_account_info(),
            &executor.to_account_info(),
            &ctx.accounts.admin.to_account_info(),
            &payments,
        )?;
    }

    // Store signal for executor to read after simulation
    thread.fiber_signal = signal.clone();

    // For Immediate triggers: auto-close after fiber completes (unless chaining)
    // Since Immediate triggers set next = i64::MAX after execution,
    // there's no reason to keep the thread alive - auto-delete to reclaim rent
    if matches!(thread.trigger, Trigger::Immediate { .. }) && signal != Signal::Chain {
        thread.fiber_signal = Signal::Close;
    }

    match &signal {
        Signal::Next { index } => {
            thread.fiber_cursor = *index;
        }
        Signal::UpdateTrigger { trigger } => {
            thread.trigger = trigger.clone();
            thread.advance_to_next_fiber();
        }
        Signal::None => {
            thread.advance_to_next_fiber();
        }
        _ => {}
    }

    // Update schedule for next execution (skip for chained - only first fiber updates schedule)
    if !is_chained {
        thread.update_schedule(&clock, ctx.remaining_accounts, &thread_pubkey)?;
    }

    // Update fiber tracking
    if !is_inline {
        let fiber = ctx
            .accounts
            .fiber
            .as_mut()
            .ok_or(AntegenThreadError::FiberAccountRequired)?;
        fiber.last_executed = clock.unix_timestamp;
        fiber.exec_count += 1;
    }

    thread.exec_count += 1;
    thread.last_executor = executor.key();
    thread.last_error_time = None;

    Ok(())
}
