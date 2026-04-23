use crate::constants::*;
use crate::state::*;
use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::Instruction;

use super::create::initialize_fiber;

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
) -> Result<()> {
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
        )?;
    } else {
        // Already initialized — update in place
        let mut data = fiber_info.try_borrow_mut_data()?;

        // Verify discriminator
        let discriminator = FiberState::DISCRIMINATOR;
        if data[..8] != discriminator[..] {
            return Err(anchor_lang::error::ErrorCode::AccountDiscriminatorMismatch.into());
        }

        let mut state: FiberState = FiberState::try_deserialize(&mut &data[..])?;
        state.thread = thread_key;

        if let Some(instruction) = instruction {
            // Compile and store new instruction
            let compiled = compile_instruction(instruction)?;
            state.compiled_instruction = borsh::to_vec(&compiled)?;
        } else {
            // Wipe — empty vec signals idle fiber
            state.compiled_instruction = vec![];
        }

        if let Some(fee) = priority_fee {
            state.priority_fee = fee;
        }
        state.last_executed = 0;
        state.exec_count = 0;

        // Re-serialize
        let state_bytes = borsh::to_vec(&state)?;
        data[8..8 + state_bytes.len()].copy_from_slice(&state_bytes);
    }

    Ok(())
}
