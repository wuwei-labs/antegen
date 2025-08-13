use crate::{state::*, *};
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

pub fn handler(ctx: Context<ThreadUpdate>, new_trigger: Option<Trigger>) -> Result<()> {
    let thread = &mut ctx.accounts.thread;

    // Update the trigger if provided
    if let Some(trigger) = new_trigger {
        thread.trigger = trigger.clone();
        
        // Reset trigger context based on new trigger type
        thread.trigger_context = match trigger {
            Trigger::Account { .. } => TriggerContext::Account { hash: 0 },
            Trigger::Now | Trigger::Timestamp { .. } | Trigger::Interval { .. } | Trigger::Cron { .. } => {
                TriggerContext::Timestamp { prev: 0, next: 0 }
            }
            Trigger::Slot { .. } | Trigger::Epoch { .. } => {
                TriggerContext::Block { prev: 0, next: 0 }
            }
        };
    }

    Ok(())
}