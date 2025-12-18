use crate::{state::*, utils::next_timestamp, *};
use anchor_lang::prelude::*;

/// Accounts required by the `thread_update` instruction.
#[derive(Accounts)]
pub struct ThreadUpdate<'info> {
    /// The authority (owner) of the thread.
    #[account(mut)]
    pub authority: Signer<'info>,

    /// The thread to be updated.
    #[account(
        mut,
        constraint = authority.key().eq(&thread.authority),
        seeds = [
            SEED_THREAD,
            thread.authority.as_ref(),
            thread.id.as_slice(),
        ],
        bump = thread.bump,
    )]
    pub thread: Account<'info, Thread>,
}

pub fn thread_update(ctx: Context<ThreadUpdate>, new_trigger: Option<Trigger>) -> Result<()> {
    let thread = &mut ctx.accounts.thread;

    // Update the trigger if provided
    if let Some(trigger) = new_trigger {
        let clock = Clock::get()?;
        let current_timestamp = clock.unix_timestamp;
        let thread_pubkey = thread.key();

        thread.trigger = trigger.clone();

        // Initialize schedule based on trigger type (mirrors thread_create logic)
        thread.schedule = match &trigger {
            Trigger::Account { .. } => Schedule::OnChange { prev: 0 },
            Trigger::Cron {
                schedule, jitter, ..
            } => {
                let base_next =
                    next_timestamp(current_timestamp, schedule.clone()).unwrap_or(current_timestamp);
                let jitter_offset =
                    crate::utils::calculate_jitter_offset(current_timestamp, &thread_pubkey, *jitter);
                Schedule::Timed {
                    prev: current_timestamp,
                    next: base_next.saturating_add(jitter_offset),
                }
            }
            Trigger::Immediate { .. } => Schedule::Timed {
                prev: current_timestamp,
                next: current_timestamp,
            },
            Trigger::Slot { slot } => Schedule::Block {
                prev: clock.slot,
                next: *slot,
            },
            Trigger::Epoch { epoch } => Schedule::Block {
                prev: clock.epoch,
                next: *epoch,
            },
            Trigger::Interval {
                seconds, jitter, ..
            } => {
                let base_next = current_timestamp.saturating_add(*seconds);
                let jitter_offset =
                    crate::utils::calculate_jitter_offset(current_timestamp, &thread_pubkey, *jitter);
                Schedule::Timed {
                    prev: current_timestamp,
                    next: base_next.saturating_add(jitter_offset),
                }
            }
            Trigger::Timestamp { unix_ts, .. } => Schedule::Timed {
                prev: current_timestamp,
                next: *unix_ts,
            },
        };
    }

    Ok(())
}
