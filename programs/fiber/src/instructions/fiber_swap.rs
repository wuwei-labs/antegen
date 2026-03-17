use crate::state::FiberState;
use anchor_lang::prelude::*;

/// Accounts required by the `swap_fiber` instruction.
/// Copies source's instruction into target, closes source.
/// Thread PDA is signer, receives source's rent back.
#[derive(Accounts)]
pub struct FiberSwap<'info> {
    /// Thread PDA - signer, receives source's rent back
    #[account(mut)]
    pub thread: Signer<'info>,

    /// Target fiber - receives source's instruction content
    #[account(mut, has_one = thread)]
    pub target: Account<'info, FiberState>,

    /// Source fiber - closed after its instruction is copied to target
    #[account(mut, has_one = thread, close = thread)]
    pub source: Account<'info, FiberState>,
}

pub fn fiber_swap(ctx: Context<FiberSwap>) -> Result<()> {
    let target = &mut ctx.accounts.target;
    let source = &ctx.accounts.source;

    // Copy instruction content from source to target
    target.compiled_instruction = source.compiled_instruction.clone();
    target.priority_fee = source.priority_fee;

    // Reset target's execution stats
    target.last_executed = 0;
    target.exec_count = 0;

    // Anchor's close = thread handles source closure and rent return
    Ok(())
}
