use {
    crate::{constants::*, errors::*, state::*},
    anchor_lang::prelude::*,
};

/// Accounts required by the `thread_claim` instruction.
#[derive(Accounts)]
pub struct ThreadClaim<'info> {
    /// The observer claiming the fiber
    #[account(mut)]
    pub observer: Signer<'info>,
    
    /// The thread being claimed
    #[account(
        seeds = [
            SEED_THREAD,
            thread.authority.as_ref(),
            thread.id.as_slice(),
        ],
        bump = thread.bump,
        constraint = !thread.paused @ AntegenThreadError::ThreadPaused,
    )]
    pub thread: Account<'info, Thread>,
    
    /// The fiber to claim (at current exec_index)
    #[account(
        mut,
        seeds = [
            SEED_THREAD_FIBER,
            thread.key().as_ref(),
            &[thread.exec_index],
        ],
        bump,
        constraint = fiber.index == thread.exec_index @ AntegenThreadError::WrongFiberIndex,
    )]
    pub fiber: Account<'info, FiberState>,
}

pub fn handler(ctx: Context<ThreadClaim>) -> Result<()> {
    let fiber = &mut ctx.accounts.fiber;
    let observer = &ctx.accounts.observer;
    let clock = Clock::get()?;
    
    // Check if already claimed
    if let Some(existing_observer) = fiber.observer {
        // Allow re-claiming by same observer (updating claim time)
        require!(
            existing_observer == observer.key(),
            AntegenThreadError::AlreadyClaimed
        );
    }
    
    // Create signature for this claim
    let _message = format!(
        "claim_fiber_{}_{}_{}",
        fiber.thread,
        fiber.index,
        clock.unix_timestamp
    );
    
    // For now, we'll use a placeholder signature
    // In production, this would be the actual signature from the observer
    let signature = [0u8; 64];
    
    // Record the claim
    fiber.observer = Some(observer.key());
    fiber.observer_signature = Some(signature);
    fiber.claimed_at = clock.unix_timestamp;
    
    msg!(
        "Observer {} claimed fiber {} for thread {} at timestamp {}",
        observer.key(),
        fiber.index,
        fiber.thread,
        clock.unix_timestamp
    );
    
    Ok(())
}