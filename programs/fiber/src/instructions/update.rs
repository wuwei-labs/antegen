use crate::constants::*;
use crate::errors::AntegenFiberError;
use crate::state::*;
use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::Instruction;

use super::create::{initialize_fiber, write_legacy, write_versioned};

/// Accounts required by the `update_fiber` instruction.
/// Thread PDA must be signer. Fiber must be pre-funded if not yet initialized.
#[derive(Accounts)]
#[instruction(fiber_index: u8)]
pub struct Update<'info> {
    /// Thread PDA - must be signer
    pub thread: Signer<'info>,

    /// CHECK: Validated via seeds — may not be initialized yet
    #[account(
        mut,
        seeds = [SEED_THREAD_FIBER, thread.key().as_ref(), &[fiber_index]],
        bump,
    )]
    pub fiber: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub fn update(
    ctx: Context<Update>,
    fiber_index: u8,
    instruction: Option<Instruction>,
    priority_fee: Option<u64>,
    lookup_tables: Option<Vec<Pubkey>>,
) -> Result<()> {
    if let Some(ref lt) = lookup_tables {
        require!(
            lt.len() <= MAX_LOOKUP_TABLES_PER_FIBER,
            AntegenFiberError::LookupTablesExceedMax
        );
    }

    let thread_key = ctx.accounts.thread.key();
    let fiber_info = ctx.accounts.fiber.to_account_info();

    if fiber_info.data_len().eq(&0) {
        // Not initialized — do full init (same as fiber_create)
        let instruction = instruction.ok_or(anchor_lang::error::ErrorCode::InstructionMissing)?;
        let fee = priority_fee.unwrap_or(0);
        initialize_fiber(
            &ctx.accounts.fiber,
            &ctx.accounts.system_program,
            &thread_key,
            fiber_index,
            &instruction,
            fee,
            lookup_tables.unwrap_or_default(),
        )?;
        return Ok(());
    }

    let fiber_read = {
        let data = fiber_info.try_borrow_data()?;
        Fiber::try_deserialize(&mut &data[..])?
    };

    match fiber_read {
        Fiber::Legacy(mut state) => {
            // Legacy fibers cannot grow to hold lookup_tables.
            if let Some(ref lt) = lookup_tables {
                require!(
                    lt.is_empty(),
                    AntegenFiberError::LegacyFiberLookupTablesUnsupported
                );
            }
            state.thread = thread_key;
            apply_instruction_update(&mut state.compiled_instruction, instruction)?;
            if let Some(fee) = priority_fee {
                state.priority_fee = fee;
            }
            state.last_executed = 0;
            state.exec_count = 0;
            write_legacy(&fiber_info, &state)?;
        }
        Fiber::V1(mut state) => {
            state.version = CURRENT_FIBER_VERSION;
            state.thread = thread_key;
            apply_instruction_update(&mut state.compiled_instruction, instruction)?;
            if let Some(fee) = priority_fee {
                state.priority_fee = fee;
            }
            if let Some(lt) = lookup_tables {
                state.lookup_tables = lt;
            }
            state.last_executed = 0;
            state.exec_count = 0;
            write_versioned(&fiber_info, &state)?;
        }
    }

    Ok(())
}

/// Update the compiled_instruction blob.
///   - `Some(ix)` → compile and store.
///   - `None`     → wipe (empty vec, signals idle fiber).
fn apply_instruction_update(
    compiled_instruction: &mut Vec<u8>,
    instruction: Option<Instruction>,
) -> Result<()> {
    if let Some(ix) = instruction {
        let compiled = compile_instruction(ix)?;
        *compiled_instruction = borsh::to_vec(&compiled)?;
    } else {
        compiled_instruction.clear();
    }
    Ok(())
}
