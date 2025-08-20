use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use crate::state::{
    decompile_instruction, CompiledInstructionV0, Trigger, TriggerContext, PAYER_PUBKEY,
};
use crate::{constants::*, errors::*, state::*};
use anchor_lang::{
    prelude::*,
    solana_program::{
        program::invoke_signed, system_instruction::advance_nonce_account, system_program,
        sysvar::recent_blockhashes,
    },
};
use chrono::{DateTime, Utc};
use solana_cron::Schedule;
use std::str::FromStr;

/// Accounts required by the `thread_exec` instruction.
#[derive(Accounts)]
pub struct ThreadExec<'info> {
    /// The executor sending the transaction
    #[account(mut)]
    pub executor: Signer<'info>,

    /// The fee payer for the transaction
    #[account(mut)]
    pub fee_payer: Signer<'info>,

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

    /// The authority of the thread (for fee distribution)
    /// CHECK: This is validated by the thread account
    #[account(
        mut,
        constraint = thread_authority.key() == thread.authority @ AntegenThreadError::InvalidThreadAuthority,
    )]
    pub thread_authority: UncheckedAccount<'info>,

    /// The config admin (for core team fee distribution)
    /// CHECK: This is validated by the config account
    #[account(
        mut,
        constraint = config_admin.key() == config.admin @ AntegenThreadError::InvalidConfigAdmin,
    )]
    pub config_admin: UncheckedAccount<'info>,

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

fn next_timestamp(after: i64, schedule: String) -> Option<i64> {
    Schedule::from_str(&schedule)
        .unwrap()
        .next_after(&DateTime::<Utc>::from_timestamp(after, 0).unwrap())
        .take()
        .map(|datetime| datetime.timestamp())
}

pub fn thread_exec(ctx: Context<ThreadExec>, forgo_commission: bool) -> Result<()> {
    let thread = &mut ctx.accounts.thread;
    let fiber = &mut ctx.accounts.fiber;
    let config = &ctx.accounts.config;
    let executor = &ctx.accounts.executor;
    let _fee_payer = &ctx.accounts.fee_payer;
    let clock = Clock::get()?;

    // Check if global pause is active
    require!(!config.paused, AntegenThreadError::GlobalPauseActive);

    // Verify durable nonce is required
    require!(
        ctx.accounts.nonce_account.is_some(),
        AntegenThreadError::NonceRequired
    );

    // Advance nonce account (required for all threads)
    {
        match (
            &ctx.accounts.nonce_account,
            &ctx.accounts.recent_blockhashes,
        ) {
            (Some(nonce_account), Some(recent_blockhashes)) => {
                // Thread PDA signs to advance its own nonce
                let thread_seeds = &[
                    SEED_THREAD,
                    thread.authority.as_ref(),
                    thread.id.as_slice(),
                    &[thread.bump],
                ];

                invoke_signed(
                    &advance_nonce_account(&nonce_account.key(), &thread.key()),
                    &[
                        nonce_account.to_account_info(),
                        recent_blockhashes.to_account_info(),
                        thread.to_account_info(),
                    ],
                    &[thread_seeds],
                )?;
            }
            _ => {
                return Err(AntegenThreadError::InvalidNonceAccount.into());
            }
        }
    }

    // First verify trigger is ready and get when it became ready
    let trigger_ready_time = verify_trigger_and_get_ready_time(
        &thread.trigger,
        &thread.trigger_context,
        &clock,
        ctx.remaining_accounts.first(),
    )?;

    // Get the last started at timestamp for trigger context updates
    let last_started_at = match &thread.trigger_context {
        TriggerContext::Timestamp { prev, .. } => *prev,
        TriggerContext::Block { prev, .. } => *prev as i64,
        TriggerContext::Account { .. } => thread.created_at,
    };

    // Update trigger context based on trigger type
    match thread.trigger.clone() {
        Trigger::Account {
            address,
            offset,
            size,
        } => {
            // Verify proof that account data has been updated
            match ctx.remaining_accounts.first() {
                None => {
                    return Err(AntegenThreadError::TriggerConditionFailed.into());
                }
                Some(account_info) => {
                    // Verify the remaining account is the account this thread is listening for
                    require!(
                        address.eq(account_info.key),
                        AntegenThreadError::TriggerConditionFailed
                    );

                    // Begin computing the data hash of this account
                    let mut hasher = DefaultHasher::new();
                    let data = &account_info.try_borrow_data().unwrap();
                    let offset = offset as usize;
                    let range_end = offset.checked_add(size as usize).unwrap() as usize;
                    if data.len().gt(&range_end) {
                        data[offset..range_end].hash(&mut hasher);
                    } else {
                        data[offset..].hash(&mut hasher)
                    }
                    let data_hash = hasher.finish();

                    // Verify the data hash is different than the prior data hash
                    match &thread.trigger_context {
                        TriggerContext::Account { hash: prior_hash } => {
                            require!(
                                data_hash.ne(prior_hash),
                                AntegenThreadError::TriggerConditionFailed
                            )
                        }
                        _ => {}
                    }

                    // Update trigger context with new data hash
                    thread.trigger_context = TriggerContext::Account { hash: data_hash };
                }
            }
        }
        Trigger::Cron {
            schedule,
            skippable,
        } => {
            // Calculate the next scheduled timestamp
            let threshold_timestamp = next_timestamp(last_started_at, schedule.clone())
                .ok_or(AntegenThreadError::TriggerConditionFailed)?;

            // Validate we've reached the scheduled time
            require!(
                clock.unix_timestamp.ge(&threshold_timestamp),
                AntegenThreadError::TriggerConditionFailed
            );

            // If skippable, use current time; otherwise use scheduled time
            let started_at = if skippable {
                clock.unix_timestamp
            } else {
                threshold_timestamp
            };

            // Update trigger context
            thread.trigger_context = TriggerContext::Timestamp {
                prev: last_started_at,
                next: started_at,
            };
        }
        Trigger::Now => {
            // Now triggers are always valid
            thread.trigger_context = TriggerContext::Timestamp {
                prev: last_started_at,
                next: clock.unix_timestamp,
            };
        }
        Trigger::Slot { slot } => {
            // Validate we've reached the target slot
            require!(
                clock.slot.ge(&slot),
                AntegenThreadError::TriggerConditionFailed
            );

            thread.trigger_context = TriggerContext::Block {
                prev: last_started_at as u64,
                next: slot,
            };
        }
        Trigger::Epoch { epoch } => {
            // Validate we've reached the target epoch
            require!(
                clock.epoch.ge(&epoch),
                AntegenThreadError::TriggerConditionFailed
            );

            thread.trigger_context = TriggerContext::Block {
                prev: last_started_at as u64,
                next: epoch,
            };
        }
        Trigger::Interval { seconds, skippable } => {
            // Calculate next trigger time
            let next_timestamp = last_started_at.saturating_add(seconds);

            // Validate we've reached the interval time
            require!(
                clock.unix_timestamp.ge(&next_timestamp),
                AntegenThreadError::TriggerConditionFailed
            );

            // If skippable, use current time; otherwise use scheduled time
            let started_at = if skippable {
                clock.unix_timestamp
            } else {
                next_timestamp
            };

            // Update trigger context
            thread.trigger_context = TriggerContext::Timestamp {
                prev: last_started_at,
                next: started_at,
            };
        }
        Trigger::Timestamp { unix_ts } => {
            // Validate we've reached the target timestamp
            require!(
                clock.unix_timestamp.ge(&unix_ts),
                AntegenThreadError::TriggerConditionFailed
            );

            thread.trigger_context = TriggerContext::Timestamp {
                prev: last_started_at,
                next: unix_ts,
            };
        }
    }

    // Decompile the fiber instruction
    let compiled = CompiledInstructionV0::try_from_slice(&fiber.compiled_instruction)?;
    let mut instruction = decompile_instruction(&compiled)?;

    // Replace PAYER_PUBKEY with executor
    for acc in instruction.accounts.iter_mut() {
        if acc.pubkey.eq(&PAYER_PUBKEY) {
            acc.pubkey = executor.key();
        }
    }

    // Execute the fiber instruction via CPI
    let thread_seeds = &[
        SEED_THREAD,
        thread.authority.as_ref(),
        thread.id.as_slice(),
        &[thread.bump],
    ];

    invoke_signed(&instruction, &ctx.remaining_accounts, &[thread_seeds])?;

    // Update exec_index to next fiber (wraps around if at end)
    thread.exec_index = (thread.exec_index + 1) % thread.fibers.len() as u8;

    // Calculate time since trigger was ready
    let time_since_ready = clock.unix_timestamp.saturating_sub(trigger_ready_time);

    // Calculate commission multiplier based on timing
    let commission_multiplier = if time_since_ready <= config.grace_period_seconds {
        // Within grace period: full commission
        1.0
    } else if time_since_ready <= config.grace_period_seconds + config.fee_decay_seconds {
        // Within decay period: linear decay from 100% to 0%
        let time_into_decay = (time_since_ready - config.grace_period_seconds) as f64;
        let decay_progress = time_into_decay / config.fee_decay_seconds as f64;
        1.0 - decay_progress  // Goes from 1.0 to 0.0
    } else {
        // After grace + decay period: no commission
        0.0
    };

    // Calculate effective commission fee
    let effective_commission = (config.commission_fee as f64 * commission_multiplier) as u64;

    // Distribute using fixed percentages
    let executor_fee = if forgo_commission {
        0 // Executor forgoes their commission
    } else {
        (effective_commission * config.executor_fee_bps) / 10_000
    };
    let core_team_fee = (effective_commission * config.core_team_bps) / 10_000;

    msg!("Execution timing: {}s after trigger ready", time_since_ready);
    msg!("Commission: {} lamports ({}% of base)", 
        effective_commission, 
        (commission_multiplier * 100.0) as u64
    );
    
    if forgo_commission && effective_commission > 0 {
        msg!("Executor forgoing commission: {} lamports retained by thread", 
            (effective_commission * config.executor_fee_bps) / 10_000);
    }

    // Transfer fees only if non-zero
    if executor_fee > 0 {
        **ctx.accounts.executor.try_borrow_mut_lamports()? += executor_fee;
        **ctx.accounts.thread_authority.try_borrow_mut_lamports()? -= executor_fee;
    }

    if core_team_fee > 0 {
        **ctx.accounts.config_admin.try_borrow_mut_lamports()? += core_team_fee;
        **ctx.accounts.thread_authority.try_borrow_mut_lamports()? -= core_team_fee;
    }

    // Log warning if late execution
    if commission_multiplier == 0.0 {
        msg!("WARNING: Execution >{}s late - no commission paid!", 
            config.grace_period_seconds + config.fee_decay_seconds);
    }

    // Update execution tracking
    fiber.last_executed = clock.unix_timestamp;
    fiber.execution_count += 1;

    Ok(())
}

/// Verify trigger is ready and return when it became ready
fn verify_trigger_and_get_ready_time(
    trigger: &Trigger,
    context: &TriggerContext,
    clock: &Clock,
    remaining_account: Option<&AccountInfo>,
) -> Result<i64> {
    match trigger {
        Trigger::Now => {
            // Now triggers are always ready
            Ok(clock.unix_timestamp)
        }
        Trigger::Timestamp { unix_ts } => {
            require!(
                clock.unix_timestamp >= *unix_ts,
                AntegenThreadError::TriggerNotReady
            );
            Ok(*unix_ts)
        }
        Trigger::Slot { slot } => {
            require!(clock.slot >= *slot, AntegenThreadError::TriggerNotReady);
            // Approximate when slot was reached (assuming 400ms per slot)
            Ok(clock.unix_timestamp - ((clock.slot - slot) as i64 * 400 / 1000))
        }
        Trigger::Epoch { epoch } => {
            require!(clock.epoch >= *epoch, AntegenThreadError::TriggerNotReady);
            // Approximate when epoch was reached
            Ok(clock.unix_timestamp)
        }
        Trigger::Interval { seconds, .. } => {
            let prev_time = match context {
                TriggerContext::Timestamp { prev, .. } => *prev,
                _ => 0,
            };
            let next_time = prev_time + seconds;
            require!(
                clock.unix_timestamp >= next_time,
                AntegenThreadError::TriggerNotReady
            );
            Ok(next_time)
        }
        Trigger::Cron { schedule, .. } => {
            let prev_time = match context {
                TriggerContext::Timestamp { prev, .. } => *prev,
                _ => 0,
            };
            let next_time = next_timestamp(prev_time, schedule.clone())
                .ok_or(AntegenThreadError::TriggerNotReady)?;
            require!(
                clock.unix_timestamp >= next_time,
                AntegenThreadError::TriggerNotReady
            );
            Ok(next_time)
        }
        Trigger::Account { address, .. } => {
            // For account triggers, verify account changed
            let account_info = remaining_account.ok_or(AntegenThreadError::TriggerNotReady)?;
            require!(
                address == account_info.key,
                AntegenThreadError::TriggerNotReady
            );
            // Account triggers are ready "now" when detected
            Ok(clock.unix_timestamp)
        }
    }
}
