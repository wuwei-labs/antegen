use crate::{errors::*, *};
use anchor_lang::{
    prelude::*,
    solana_program::program::{get_return_data, invoke_signed},
    AnchorDeserialize,
};
use antegen_utils::thread::{
    decompile_instruction, transfer_lamports, CompiledTransactionV0, ThreadResponse, PAYER_PUBKEY,
};

/// Accounts required by the `thread_exec` instruction.
#[derive(Accounts)]
pub struct ThreadExec<'info> {
    /// The signatory.
    #[account(mut)]
    pub signatory: Signer<'info>,

    /// The thread to execute.
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
        seeds = [
            SEED_THREAD_FIBER,
            thread.key().as_ref(),
            &[thread.exec_index],
        ],
        bump,
    )]
    pub fiber: Account<'info, FiberState>,
}

pub fn handler(ctx: Context<ThreadExec>) -> Result<()> {
    let signatory = &mut ctx.accounts.signatory;
    let thread = &mut ctx.accounts.thread;
    let fiber = &ctx.accounts.fiber;

    let initial_signatory_balance = signatory.lamports();

    // Deserialize the compiled instruction
    let compiled = CompiledTransactionV0::try_from_slice(&fiber.compiled_instruction)?;
    let mut instruction = decompile_instruction(&compiled)?;

    // Replace PAYER_PUBKEY with signatory
    for acc in instruction.accounts.iter_mut() {
        if acc.pubkey.eq(&PAYER_PUBKEY) {
            acc.pubkey = signatory.key();
        }
    }

    // Check if this is a thread_delete instruction
    let is_delete = instruction.data.len() >= 8
        && instruction.data[..8] == *crate::instruction::ThreadDelete::DISCRIMINATOR;

    // Invoke the instruction
    invoke_signed(
        &instruction,
        ctx.remaining_accounts,
        &[&[
            SEED_THREAD,
            thread.authority.as_ref(),
            thread.id.as_slice(),
            &[thread.bump],
        ]],
    )?;

    if is_delete {
        // Thread is being deleted, no need to update exec_index
        return Ok(());
    }

    // Verify the inner instruction did not write data to the signatory address
    require!(
        signatory.data_is_empty(),
        AntegenThreadError::UnauthorizedWrite
    );

    // Parse the thread response if any
    let thread_response: Option<ThreadResponse> = match get_return_data() {
        None => None,
        Some((program_id, return_data)) => {
            require!(
                program_id.eq(&instruction.program_id),
                AntegenThreadError::InvalidThreadResponse
            );
            ThreadResponse::try_from_slice(return_data.as_slice()).ok()
        }
    };

    // Handle thread response
    if let Some(response) = thread_response {
        if let Some(trigger) = response.trigger {
            thread.trigger = trigger.clone();

            // Reset trigger context if account trigger was updated
            if let Trigger::Account { .. } = trigger {
                thread.trigger_context = TriggerContext::Account { hash: 0 };
            }
        }

        // Handle next instruction override
        if let Some(next_idx) = response.next_instruction {
            if thread.fibers.contains(&next_idx) {
                thread.exec_index = next_idx;
            }
        } else {
            // Move to next fiber in sequence
            thread.advance_to_next_fiber();
        }

        // Handle close_to
        if response.close_to.is_some() {
            // The thread should be closed - this would be handled by a separate instruction
            // For now, just pause the thread
            thread.paused = true;
        }
    } else {
        // Move to next fiber in sequence
        thread.advance_to_next_fiber();
    }

    // Calculate reimbursement
    let mut required_reimbursement = 0u64;
    if signatory.lamports() < initial_signatory_balance {
        required_reimbursement = initial_signatory_balance.saturating_sub(signatory.lamports());
    }

    // Add transaction base fee
    required_reimbursement = required_reimbursement
        .checked_add(TRANSACTION_BASE_FEE_REIMBURSEMENT)
        .unwrap();

    // Handle reimbursement
    if required_reimbursement > 0 {
        transfer_lamports(
            &thread.to_account_info(),
            &signatory.to_account_info(),
            required_reimbursement,
        )?;
    }

    Ok(())
}
