use crate::{errors::*, state::*, TRANSACTION_BASE_FEE_REIMBURSEMENT};
use anchor_lang::{
    prelude::*,
    solana_program::{
        instruction::Instruction,
        program::{get_return_data, invoke_signed},
    },
    AnchorDeserialize, InstructionData,
};
use antegen_network_program::{network_program::TOTAL_BASIS_POINTS, state::*, ANTEGEN_SQUADS};
use antegen_utils::thread::{
    transfer_lamports, SerializableInstruction, ThreadResponse, PAYER_PUBKEY,
};

/// Accounts required by the `thread_exec` instruction.
#[derive(Accounts)]
pub struct ThreadExec<'info> {
    /// CHECKED: with worker account validation
    #[account(mut)]
    authority: UncheckedAccount<'info>,

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
        constraint = thread.next_instruction.is_some(),
        constraint = thread.exec_context.is_some()
    )]
    pub thread: Box<Account<'info, Thread>>,

    /// The worker.
    #[account(
        has_one = authority,
        address = builder.pubkey(),
    )]
    pub builder: Account<'info, Builder>,

    #[account(
        mut,
        address = ANTEGEN_SQUADS
    )]
    pub network_fee: SystemAccount<'info>,
}

pub fn handler(ctx: Context<ThreadExec>) -> Result<()> {
    let clock: Clock = Clock::get().unwrap();
    let signatory: &mut Signer = &mut ctx.accounts.signatory;
    let thread: &mut Box<Account<Thread>> = &mut ctx.accounts.thread;
    let builder: &Account<Builder> = &ctx.accounts.builder;
    let authority: &mut UncheckedAccount = &mut ctx.accounts.authority;
    let network_fee: &mut SystemAccount = &mut ctx.accounts.network_fee;

    // If the rate limit has been met, exit early.
    if thread.exec_context.unwrap().last_exec_at == clock.slot
        && thread.exec_context.unwrap().execs_since_slot >= thread.rate_limit
    {
        return Err(AntegenThreadError::RateLimitExeceeded.into());
    }

    let initial_signatory_balance: u64 = signatory.lamports();

    // Get the instruction to execute.
    // We have already verified that it is not null during account validation.
    let instruction: &mut SerializableInstruction = &mut thread.next_instruction.clone().unwrap();
    for acc in instruction.accounts.iter_mut() {
        if acc.pubkey.eq(&PAYER_PUBKEY) {
            acc.pubkey = signatory.key();
        }
    }

    let is_delete: bool = instruction.data[..8] == *crate::instruction::ThreadDelete::DISCRIMINATOR;
    // Invoke the provided instruction.
    invoke_signed(
        &Instruction::from(&*instruction),
        ctx.remaining_accounts,
        &[&[
            SEED_THREAD,
            thread.authority.as_ref(),
            thread.id.as_slice(),
            &[thread.bump],
        ]],
    )?;

    if is_delete {
        thread.next_instruction = None;
        return Ok(());
    }

    // Verify the inner instruction did not write data to the signatory address.
    require!(
        signatory.data_is_empty(),
        AntegenThreadError::UnauthorizedWrite
    );

    // Parse the thread response
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

    // Grab the next instruction from the thread response.
    let mut close_to: Option<Pubkey> = None;
    let mut next_instruction: Option<SerializableInstruction> = None;
    if let Some(thread_response) = thread_response {
        close_to = thread_response.close_to;
        next_instruction = thread_response.dynamic_instruction;

        // Update the trigger.
        if let Some(trigger) = thread_response.trigger {
            thread.trigger = trigger.clone();

            // If the user updates an account trigger, the trigger context is no longer valid.
            // Here we reset the trigger context to zero to re-prime the trigger.
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
            })
        }
    }

    // If there is no dynamic next instruction, get the next instruction from the instruction set.
    let mut exec_index: u64 = thread.exec_context.unwrap().exec_index;
    if next_instruction.is_none() {
        if let Some(ix) = thread.instructions.get((exec_index + 1) as usize) {
            next_instruction = Some(ix.clone());
            exec_index = exec_index + 1;
        }
    }

    // Update the next instruction.
    if let Some(close_to) = close_to {
        thread.next_instruction = Some(
            Instruction {
                program_id: crate::ID,
                accounts: crate::accounts::ThreadDelete {
                    authority: thread.key(),
                    close_to,
                    thread: thread.key(),
                }
                .to_account_metas(Some(true)),
                data: crate::instruction::ThreadDelete {}.data(),
            }
            .into(),
        );
    } else {
        thread.next_instruction = next_instruction;
    };
    // Update the exec context.
    thread.exec_context = Some(ExecContext {
        exec_index,
        execs_since_slot: if clock.slot == thread.exec_context.unwrap().last_exec_at {
            thread
                .exec_context
                .unwrap()
                .execs_since_slot
                .checked_add(1)
                .unwrap()
        } else {
            1
        },
        last_exec_at: clock.slot,
        ..thread.exec_context.unwrap()
    });

    // Calculate reimbursement needs
    let should_reimburse_transaction: bool =
        clock.slot.gt(&thread.exec_context.unwrap().last_exec_at);

    let mut required_reimbursement: u64;
    if signatory.lamports().lt(&initial_signatory_balance) {
        required_reimbursement = initial_signatory_balance.saturating_sub(signatory.lamports());
    } else {
        required_reimbursement = 0;
    }

    if should_reimburse_transaction {
        required_reimbursement = required_reimbursement
            .checked_add(TRANSACTION_BASE_FEE_REIMBURSEMENT)
            .unwrap();
    }

    // Handle reimbursement if needed
    if required_reimbursement.gt(&0) {
        transfer_lamports(
            &thread.to_account_info(),
            &signatory.to_account_info(),
            required_reimbursement,
        )?;
    }

    if builder.pool.gt(&0) {
        let fee: u64 = thread.fee;
        let commission_rate: u64 = builder.commission_rate;
        let commission_bps: u64 = commission_rate.saturating_mul(100);
        let commission_amount: u64 = fee
            .saturating_mul(commission_bps)
            .saturating_div(TOTAL_BASIS_POINTS);
        let network_fee_amount: u64 = fee.saturating_sub(commission_amount);

        transfer_lamports(
            &thread.to_account_info(),
            &authority.to_account_info(),
            commission_amount,
        )?;

        // Transfer network fee
        transfer_lamports(
            &thread.to_account_info(),
            &network_fee.to_account_info(),
            network_fee_amount,
        )?;
    }

    Ok(())
}
