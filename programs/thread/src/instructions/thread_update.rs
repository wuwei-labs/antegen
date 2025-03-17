use crate::{errors::*, state::*};

use anchor_lang::{
    prelude::*,
    solana_program::system_program,
    system_program::{transfer, Transfer},
};
use antegen_network_program::state::{Config, SEED_CONFIG};

/// Accounts required by the `thread_update` instruction.
#[derive(Accounts)]
#[instruction(settings: ThreadSettings)]
pub struct ThreadUpdate<'info> {
    /// The authority (owner) of the thread.
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        seeds = [SEED_CONFIG],
        bump
    )]
    pub config: Account<'info, Config>,

    /// The Solana system program
    #[account(address = system_program::ID)]
    pub system_program: Program<'info, System>,

    /// The thread to be updated.
    #[account(
            mut,
            constraint = authority.key().eq(&thread.authority) ||
                // allow config.admin to update system threads
                (authority.key().eq(&config.admin) &&
                    (thread.key().eq(&config.epoch_thread) || thread.key().eq(&config.hasher_thread))
                ),
            seeds = [
                SEED_THREAD,
                thread.authority.as_ref(),
                thread.id.as_slice(),
            ],
            bump = thread.bump
        )]
    pub thread: Account<'info, Thread>,
}

fn can_update_trigger(current: &Trigger, new: &Trigger) -> bool {
    // Same variant is always allowed
    if std::mem::discriminant(current) == std::mem::discriminant(new) {
        return true;
    }

    // Check for special allowed combinations
    matches!(
        (current, new),
        (Trigger::Cron { .. }, Trigger::Now) |
        (Trigger::Cron { .. }, Trigger::Timestamp { .. }) |
        (Trigger::Now, Trigger::Cron { .. }) |
        (Trigger::Now, Trigger::Timestamp { .. }) |
        (Trigger::Timestamp { .. }, Trigger::Cron { .. }) |
        (Trigger::Timestamp { .. }, Trigger::Now)
    )
}

pub fn handler(ctx: Context<ThreadUpdate>, settings: ThreadSettings) -> Result<()> {
    // Get accounts
    let authority = &ctx.accounts.authority;
    let thread = &mut ctx.accounts.thread;
    let system_program = &ctx.accounts.system_program;

    // Migrate thread if needed before updating - using the method on ThreadAccount trait
    thread.migrate_if_needed()?;

    // Update the thread.
    if let Some(fee) = settings.fee {
        thread.fee = fee;
    }

    // If provided, update the thread's instruction set.
    if let Some(instructions) = settings.instructions {
        thread.instructions = instructions;
    }

    // If provided, update the rate limit.
    if let Some(rate_limit) = settings.rate_limit {
        thread.rate_limit = rate_limit;
    }

    // If provided, update the thread's trigger and reset the exec context.
    if let Some(trigger) = settings.trigger {
        require!(
            can_update_trigger(&thread.trigger, &trigger),
            AntegenThreadError::InvalidTriggerVariant
        );
        thread.trigger = trigger.clone();

        // If the user updates an account trigger, the trigger context is no longer valid.
        // Here we reset the trigger context to zero to re-prime the trigger.
        if thread.exec_context.is_some() {
            thread.exec_context = Some(ExecContext {
                trigger_context: match trigger {
                    Trigger::Account {
                        address: _,
                        offset: _,
                        size: _,
                    } => TriggerContext::Account { data_hash: 0 },
                    _ => thread.exec_context.unwrap().trigger_context,
                },
                ..thread.exec_context.unwrap()
            });
        }
    }

    // Reallocate mem for the thread account
    thread.realloc()?;

    // If lamports are required to maintain rent-exemption, pay them
    let data_len = thread.to_account_info().data_len();
    let minimum_rent = Rent::get().unwrap().minimum_balance(data_len);
    if minimum_rent > thread.to_account_info().lamports() {
        transfer(
            CpiContext::new(
                system_program.to_account_info(),
                Transfer {
                    from: authority.to_account_info(),
                    to: thread.to_account_info(),
                },
            ),
            minimum_rent
                .checked_sub(thread.to_account_info().lamports())
                .unwrap(),
        )?;
    }

    Ok(())
}
