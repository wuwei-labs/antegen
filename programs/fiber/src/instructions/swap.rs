use crate::errors::AntegenFiberError;
use crate::state::*;
use anchor_lang::prelude::*;

use super::close::sweep_fiber_lamports;
use super::create::{write_legacy, write_versioned};

/// Accounts required by the `swap_fiber` instruction.
/// Copies source's instruction into target, closes source.
/// Thread PDA is signer, receives source's rent back. Works for both legacy
/// and V1 fiber shapes on either side.
#[derive(Accounts)]
pub struct Swap<'info> {
    /// Thread PDA - signer, receives source's rent back
    #[account(mut)]
    pub thread: Signer<'info>,

    /// CHECK: target fiber, shape-agnostic — bound to this program by the
    /// owner constraint, then read manually via `Fiber` below.
    #[account(
        mut,
        constraint = target.to_account_info().owner == &crate::ID
            @ AntegenFiberError::InvalidAccountOwner,
    )]
    pub target: UncheckedAccount<'info>,

    /// CHECK: source fiber, shape-agnostic — bound to this program by the
    /// owner constraint, then read manually via `Fiber` below.
    #[account(
        mut,
        constraint = source.to_account_info().owner == &crate::ID
            @ AntegenFiberError::InvalidAccountOwner,
    )]
    pub source: UncheckedAccount<'info>,
}

pub fn swap(ctx: Context<Swap>) -> Result<()> {
    let thread_key = ctx.accounts.thread.key();
    let target_info = ctx.accounts.target.to_account_info();
    let source_info = ctx.accounts.source.to_account_info();

    // Copying a fiber onto itself would wipe it and then sweep its rent — the
    // caller ends up with neither the source nor the target it asked for.
    require_keys_neq!(
        target_info.key(),
        source_info.key(),
        AntegenFiberError::SwapSourceIsTarget
    );

    let source_read = Fiber::try_from(&source_info)?;
    require_keys_eq!(
        source_read.thread(),
        thread_key,
        AntegenFiberError::InvalidFiberPDA
    );

    let target_read = Fiber::try_from(&target_info)?;
    require_keys_eq!(
        target_read.thread(),
        thread_key,
        AntegenFiberError::InvalidFiberPDA
    );

    let new_compiled = source_read.compiled_instruction().to_vec();
    let new_priority_fee = source_read.priority_fee();

    // Preserve target's on-disk shape: write back as legacy or V1 to match.
    match target_read {
        Fiber::Legacy(mut state) => {
            state.compiled_instruction = new_compiled;
            state.priority_fee = new_priority_fee;
            state.last_executed = 0;
            state.exec_count = 0;
            write_legacy(&target_info, &state)?;
        }
        Fiber::V1(mut state) => {
            state.version = CURRENT_FIBER_VERSION;
            state.compiled_instruction = new_compiled;
            state.priority_fee = new_priority_fee;
            state.last_executed = 0;
            state.exec_count = 0;
            // lookup_tables on target stay as they were — source's ALT set
            // pertains to source's instruction at construction, not what's
            // being copied in. Callers needing different ALTs should issue
            // a follow-up fiber_update.
            write_versioned(&target_info, &state)?;
        }
    }

    sweep_fiber_lamports(&source_info, &ctx.accounts.thread.to_account_info())?;
    Ok(())
}
