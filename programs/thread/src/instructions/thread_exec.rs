use anchor_lang::{
    prelude::*,
    solana_program::{
        instruction::Instruction,
        program::{get_return_data, invoke_signed}
    },
    AnchorDeserialize, InstructionData,
};
use antegen_network_program::state::{Pool, Worker, WorkerAccount, WorkerCommission, SEED_WORKER_COMMISSION};
use antegen_utils::thread::{SerializableInstruction, ThreadResponse, PAYER_PUBKEY};
use crate::{errors::*, state::*, TRANSACTION_BASE_FEE_REIMBURSEMENT};

/// The ID of the pool workers must be a member of to collect fees.
const POOL_ID: u64 = 0;

#[derive(Debug, Clone, Copy)]
struct BalanceSnapshot {
    signatory: u64,
    commission: u64,
}

/// Represents changes in lamport balances between two snapshots
#[derive(Debug)]
struct BalanceChanges {
    signatory: i64,
    commission: i64,
}

impl BalanceSnapshot {
    fn difference(&self, other: &Self) -> BalanceChanges {
        BalanceChanges {
            signatory: self.signatory as i64 - other.signatory as i64,
            commission: self.commission as i64 - other.commission as i64,
        }
    }
}

/// Accounts required by the `thread_exec` instruction.
#[derive(Accounts)]
pub struct ThreadExec<'info> {
    /// The worker's fee account.
    #[account(
        mut,
        seeds = [
            SEED_WORKER_COMMISSION,
            worker.key().as_ref(),
        ],
        bump,
        seeds::program = antegen_network_program::ID,
        has_one = worker,
    )]
    pub commission: Account<'info, WorkerCommission>,

    /// The active worker pool.
    #[account(address = Pool::pubkey(POOL_ID))]
    pub pool: Box<Account<'info, Pool>>,

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
    #[account(address = worker.pubkey())]
    pub worker: Account<'info, Worker>,
}

fn transfer_lamports<'info>(
    from: &AccountInfo<'info>,
    to: &AccountInfo<'info>,
    amount: u64,
) -> Result<()> {
    **from.try_borrow_mut_lamports()? = from
        .lamports()
        .checked_sub(amount)
        .unwrap();
    **to.try_borrow_mut_lamports()? = to
        .to_account_info()
        .lamports()
        .checked_add(amount)
        .unwrap();
    Ok(())
}

pub fn handler(ctx: Context<ThreadExec>) -> Result<()> {
    // Get accounts
    let clock = Clock::get().unwrap();
    let commission = &mut ctx.accounts.commission;
    let pool = &ctx.accounts.pool;
    let signatory = &mut ctx.accounts.signatory;
    let thread = &mut ctx.accounts.thread;
    let worker = &ctx.accounts.worker;

    // If the rate limit has been met, exit early.
    if thread.exec_context.unwrap().last_exec_at == clock.slot
        && thread.exec_context.unwrap().execs_since_slot >= thread.rate_limit
    {
        return Err(AntegenThreadError::RateLimitExeceeded.into());
    }

    let initial_balances = BalanceSnapshot {
        signatory: signatory.lamports(),
        commission: commission.to_account_info().lamports(),
    };

    // Get the instruction to execute.
    // We have already verified that it is not null during account validation.
    let instruction: &mut SerializableInstruction = &mut thread.next_instruction.clone().unwrap();
    for acc in instruction.accounts.iter_mut() {
        if acc.pubkey.eq(&PAYER_PUBKEY) {
            acc.pubkey = signatory.key();
        }
    }

    let is_delete = instruction.data[..8] == *crate::instruction::ThreadDelete::DISCRIMINATOR;
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

    // Verify the inner instruction did not write data to the signatory address.
    require!(signatory.data_is_empty(), AntegenThreadError::UnauthorizedWrite);

    if is_delete {
        thread.next_instruction = None;
        return Ok(());
    }

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
    let mut close_to = None;
    let mut next_instruction = None;
    if let Some(thread_response) = thread_response {
        close_to = thread_response.close_to;
        next_instruction = thread_response.dynamic_instruction;

        // Update the trigger.
        if let Some(trigger) = thread_response.trigger {
            require!(
                std::mem::discriminant(&thread.trigger) == std::mem::discriminant(&trigger),
                AntegenThreadError::InvalidTriggerVariant
            );
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
    let mut exec_index = thread.exec_context.unwrap().exec_index;
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
            }.into(),
        );
    } else {
        thread.next_instruction = next_instruction;
    }

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

    // Calculate actual balance changes from inner instruction
    let post_inner_balances = BalanceSnapshot {
        signatory: signatory.lamports(),
        commission: commission.to_account_info().lamports(),
    };

    let balance_changes = post_inner_balances.difference(&initial_balances);
    // Calculate reimbursement needs
    let should_reimburse_transaction = clock.slot > thread.exec_context.unwrap().last_exec_at;
    let mut required_reimbursement = if balance_changes.signatory.lt(&0) {
        balance_changes.signatory.unsigned_abs()
    } else {
        0
    };

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

    // Only process worker fees if they haven't already been processed by inner instruction
    if pool.clone().into_inner().workers.contains(&worker.key())
        && balance_changes.commission.eq(&0)
    {
        transfer_lamports(
            &thread.to_account_info(),
            &commission.to_account_info(),
            thread.fee,
        )?;
    }

    Ok(())
}
