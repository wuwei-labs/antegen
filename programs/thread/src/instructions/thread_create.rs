use crate::{
    errors::AntegenThreadError,
    state::{compile_instruction, Schedule, SerializableInstruction, Signal, ThreadSeeds, Trigger},
    utils::next_timestamp,
    *,
};
use anchor_lang::{
    prelude::*,
    solana_program::instruction::Instruction,
    system_program::{create_nonce_account, transfer, CreateNonceAccount, Transfer},
    InstructionData, ToAccountMetas,
};
use solana_nonce::state::State;

/// Accounts required by the `thread_create` instruction.
///
/// For simple thread creation (no durable nonce), only authority, payer, thread, and system_program are needed.
/// For durable nonce threads, also provide nonce_account, recent_blockhashes, and rent.
#[derive(Accounts)]
#[instruction(amount: u64, id: ThreadId)]
pub struct ThreadCreate<'info> {
    /// CHECK: the authority (owner) of the thread. Allows for program ownership
    #[account()]
    pub authority: Signer<'info>,

    /// The payer for account initializations.
    #[account(mut)]
    pub payer: Signer<'info>,

    /// The thread to be created.
    #[account(
        init,
        seeds = [
            SEED_THREAD,
            authority.key().as_ref(),
            id.as_ref(),
        ],
        bump,
        payer = payer,
        space = 8 + Thread::INIT_SPACE
    )]
    pub thread: Account<'info, Thread>,

    /// CHECK: Nonce account (optional - only for durable nonce threads)
    /// When provided, recent_blockhashes and rent must also be provided.
    #[account(mut)]
    pub nonce_account: Option<Signer<'info>>,

    /// CHECK: Recent blockhashes sysvar (optional - only required for durable nonce threads)
    pub recent_blockhashes: Option<UncheckedAccount<'info>>,

    /// CHECK: Rent sysvar (optional - only required for durable nonce threads)
    pub rent: Option<UncheckedAccount<'info>>,

    pub system_program: Program<'info, System>,

    /// CHECK: Fiber account (optional — only when creating with an instruction)
    #[account(mut)]
    pub fiber: Option<UncheckedAccount<'info>>,

    /// Fiber Program (optional — only required when fiber is provided)
    pub fiber_program: Option<Program<'info, antegen_fiber_program::program::FiberProgram>>,
}

pub fn thread_create(
    ctx: Context<ThreadCreate>,
    amount: u64,
    id: ThreadId,
    trigger: Trigger,
    paused: Option<bool>,
    instruction: Option<SerializableInstruction>,
    priority_fee: Option<u64>,
) -> Result<()> {
    let authority: &Signer = &ctx.accounts.authority;
    let payer: &Signer = &ctx.accounts.payer;
    let thread: &mut Account<Thread> = &mut ctx.accounts.thread;

    // Check if nonce account is provided for durable nonce thread
    let create_durable_thread = ctx.accounts.nonce_account.is_some();

    if create_durable_thread {
        // Validate that required sysvars are provided for nonce creation
        let nonce_account = ctx.accounts.nonce_account.as_ref().unwrap();
        let recent_blockhashes = ctx.accounts.recent_blockhashes.as_ref().ok_or(error!(
            crate::errors::AntegenThreadError::InvalidNonceAccount
        ))?;
        let rent_program = ctx.accounts.rent.as_ref().ok_or(error!(
            crate::errors::AntegenThreadError::InvalidNonceAccount
        ))?;

        let rent: Rent = Rent::get()?;
        let nonce_account_size: usize = State::size();
        let nonce_lamports: u64 = rent.minimum_balance(nonce_account_size);

        create_nonce_account(
            CpiContext::new(
                anchor_lang::system_program::ID,
                CreateNonceAccount {
                    from: payer.to_account_info(),
                    nonce: nonce_account.to_account_info(),
                    recent_blockhashes: recent_blockhashes.to_account_info(),
                    rent: rent_program.to_account_info(),
                },
            ),
            nonce_lamports,
            &thread.key(),
        )?;

        thread.nonce_account = nonce_account.key();
    } else {
        thread.nonce_account = crate::ID;
    }

    // Initialize the thread
    let clock = Clock::get().unwrap();
    let current_timestamp = clock.unix_timestamp;

    thread.version = CURRENT_THREAD_VERSION;
    thread.authority = authority.key();
    thread.bump = ctx.bumps.thread;
    thread.created_at = current_timestamp;
    thread.name = id.to_name();
    thread.id = id.into();
    thread.paused = paused.unwrap_or(false);
    thread.trigger = trigger.clone();

    // Initialize schedule based on trigger type
    // Use created_at as initial prev value for proper fee calculation on first execution
    let thread_pubkey = thread.key();
    thread.schedule = match &trigger {
        Trigger::Account { .. } => Schedule::OnChange { prev: 0 },
        Trigger::Cron {
            schedule, jitter, ..
        } => {
            let base_next =
                next_timestamp(current_timestamp, schedule.clone()).unwrap_or(current_timestamp);
            let jitter_offset =
                crate::utils::calculate_jitter_offset(current_timestamp, &thread_pubkey, *jitter);
            let next = base_next.saturating_add(jitter_offset);
            Schedule::Timed {
                prev: current_timestamp,
                next,
            }
        }
        Trigger::Immediate { .. } => Schedule::Timed {
            prev: current_timestamp,
            next: current_timestamp,
        },
        Trigger::Slot { slot } => Schedule::Block {
            prev: clock.slot,
            next: *slot,
        },
        Trigger::Epoch { epoch } => Schedule::Block {
            prev: clock.epoch,
            next: *epoch,
        },
        Trigger::Interval {
            seconds, jitter, ..
        } => {
            let base_next = current_timestamp.saturating_add(*seconds);
            let jitter_offset =
                crate::utils::calculate_jitter_offset(current_timestamp, &thread_pubkey, *jitter);
            let next = base_next.saturating_add(jitter_offset);
            Schedule::Timed {
                prev: current_timestamp,
                next,
            }
        }
        Trigger::Timestamp { unix_ts, .. } => Schedule::Timed {
            prev: current_timestamp,
            next: *unix_ts,
        },
    };

    thread.exec_count = 0;
    thread.last_executor = Pubkey::default();
    thread.fiber_signal = Signal::None;

    // Optionally create fiber index 0 via CPI to fiber program
    if let Some(instruction) = instruction {
        // Prevent thread_delete instructions in fibers (same check as fiber_create)
        if instruction.program_id.eq(&crate::ID)
            && instruction.data.len().ge(&8)
            && instruction.data[..8].eq(crate::instruction::DeleteThread::DISCRIMINATOR)
        {
            return Err(AntegenThreadError::InvalidInstruction.into());
        }

        // Require fiber and fiber_program accounts
        let fiber = ctx
            .accounts
            .fiber
            .as_ref()
            .ok_or(AntegenThreadError::MissingFiberAccount)?;
        let fiber_program = ctx
            .accounts
            .fiber_program
            .as_ref()
            .ok_or(AntegenThreadError::MissingFiberAccount)?;

        let priority_fee = priority_fee.unwrap_or(0);

        thread.sign(|seeds| {
            antegen_fiber_program::cpi::create_fiber(
                CpiContext::new_with_signer(
                    fiber_program.key(),
                    antegen_fiber_program::cpi::accounts::FiberCreate {
                        thread: thread.to_account_info(),
                        payer: payer.to_account_info(),
                        fiber: fiber.to_account_info(),
                        system_program: ctx.accounts.system_program.to_account_info(),
                    },
                    &[seeds],
                ),
                0, // fiber_index = 0
                instruction.clone(),
                priority_fee,
            )
        })?;

        thread.fiber_next_id = 1;
        thread.fiber_ids = vec![0];
        thread.fiber_cursor = 0;
    } else {
        // No default fiber — users add fibers separately via create_fiber
        thread.fiber_next_id = 0;
        thread.fiber_ids = Vec::new();
        thread.fiber_cursor = 0;
    }

    // Build and store pre-compiled thread_close instruction for self-closing
    let close_ix = Instruction {
        program_id: crate::ID,
        accounts: crate::accounts::ThreadClose {
            authority: thread_pubkey,   // thread signs as authority
            close_to: thread.authority, // rent goes to owner
            thread: thread_pubkey,
            fiber_program: Some(antegen_fiber_program::ID),
        }
        .to_account_metas(None),
        data: crate::instruction::CloseThread {}.data(),
    };

    let compiled = compile_instruction(close_ix)?;
    thread.close_fiber = borsh::to_vec(&compiled)?;

    // Transfer SOL from payer to the thread.
    transfer(
        CpiContext::new(
            anchor_lang::system_program::ID,
            Transfer {
                from: payer.to_account_info(),
                to: thread.to_account_info(),
            },
        ),
        amount,
    )?;

    Ok(())
}
