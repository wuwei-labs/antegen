use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use crate::{errors::*, *};
use anchor_lang::{
    prelude::*,
    solana_program::{
        instruction, program::invoke_signed, system_instruction::advance_nonce_account,
        system_program, sysvar::recent_blockhashes,
    },
};
use crate::state::{Trigger, TriggerContext};
use chrono::{DateTime, Utc};
use solana_cron::Schedule;
use std::str::FromStr;

/// Accounts required by the `thread_kickoff` instruction.
#[derive(Accounts)]
pub struct ThreadKickoff<'info> {
    #[account(mut)]
    pub signatory: Signer<'info>,

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

pub fn handler(ctx: Context<ThreadKickoff>) -> Result<()> {
    // Check if called via CPI (must be from thread program)
    let stack_height = instruction::get_stack_height();
    require!(stack_height > 1, AntegenThreadError::MustBeCalledViaCPI);

    let thread: &mut Box<Account<Thread>> = &mut ctx.accounts.thread;

    let clock: Clock = Clock::get().unwrap();

    // Advance nonce account if thread has one
    if thread.has_nonce_account() {
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

    // Get the last started at timestamp
    let last_started_at = match &thread.trigger_context {
        TriggerContext::Timestamp { prev, .. } => *prev,
        TriggerContext::Block { prev, .. } => *prev as i64,
        TriggerContext::Account { .. } => thread.created_at,
    };

    match thread.trigger.clone() {
        Trigger::Account {
            address,
            offset,
            size,
        } => {
            // Verify proof that account data has been updated.
            match ctx.remaining_accounts.first() {
                None => {
                    return Err(AntegenThreadError::TriggerConditionFailed.into());
                }
                Some(account_info) => {
                    // Verify the remaining account is the account this thread is listening for.
                    require!(
                        address.eq(account_info.key),
                        AntegenThreadError::TriggerConditionFailed
                    );

                    // Begin computing the data hash of this account.
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

                    // Verify the data hash is different than the prior data hash.
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
                    // Set exec_index to first fiber
                    thread.exec_index = thread.fibers.first().copied().unwrap_or(0);
                }
            }
        }
        Trigger::Cron {
            schedule,
            skippable,
        } => {
            // Calculate the next scheduled timestamp (validation happens in thread_submit)
            let threshold_timestamp = next_timestamp(last_started_at, schedule.clone())
                .ok_or(AntegenThreadError::TriggerConditionFailed)?;

            // If the schedule is marked as skippable, set the started_at of the exec context to be the current timestamp.
            // Otherwise, the exec context must iterate through each scheduled kickoff moment.
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
            // Set exec_index to first fiber
            thread.exec_index = thread.fibers.first().copied().unwrap_or(0);
        }
        Trigger::Now => {
            // Update trigger context
            thread.trigger_context = TriggerContext::Timestamp {
                prev: last_started_at,
                next: clock.unix_timestamp,
            };
            // Set exec_index to first fiber
            thread.exec_index = thread.fibers.first().copied().unwrap_or(0);
        }
        Trigger::Slot { slot } => {
            // Validation happens in thread_submit
            thread.trigger_context = TriggerContext::Block {
                prev: last_started_at as u64,
                next: slot,
            };
            // Set exec_index to first fiber
            thread.exec_index = thread.fibers.first().copied().unwrap_or(0);
        }
        Trigger::Epoch { epoch } => {
            // Validation happens in thread_submit
            thread.trigger_context = TriggerContext::Block {
                prev: last_started_at as u64,
                next: epoch,
            };
            // Set exec_index to first fiber
            thread.exec_index = thread.fibers.first().copied().unwrap_or(0);
        }
        Trigger::Interval { seconds, skippable } => {
            // Calculate next trigger time (validation happens in thread_submit)
            let next_timestamp = last_started_at.saturating_add(seconds);

            // If skippable, set started_at to current time
            // Otherwise, use the scheduled time to catch up on missed intervals
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
            // Set exec_index to first fiber
            thread.exec_index = thread.fibers.first().copied().unwrap_or(0);
        }
        Trigger::Timestamp { unix_ts } => {
            // Validation happens in thread_submit
            thread.trigger_context = TriggerContext::Timestamp {
                prev: last_started_at,
                next: unix_ts,
            };
            // Set exec_index to first fiber
            thread.exec_index = thread.fibers.first().copied().unwrap_or(0);
        }
    }

    Ok(())
}
