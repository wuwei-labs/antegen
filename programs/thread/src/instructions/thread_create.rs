use crate::{
    state::{compile_instruction, FiberState, SerializableInstruction, Trigger, TriggerContext},
    utils::next_timestamp,
    *,
};
use anchor_lang::{
    prelude::*,
    solana_program::{
        instruction::Instruction,
        nonce::state::State,
        system_program,
        sysvar::{recent_blockhashes, rent},
    },
    system_program::{create_nonce_account, transfer, CreateNonceAccount, Transfer},
};

/// Accounts required by the `thread_create` instruction.
#[derive(Accounts)]
#[instruction(amount: u64, id: ThreadId, trigger: Trigger, initial_instruction: Option<SerializableInstruction>, priority_fee: Option<u64>)]
pub struct ThreadCreate<'info> {
    /// CHECK: the authority (owner) of the thread. Allows for program
    /// ownership
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

    /// The initial fiber account (created if initial_instruction is Some)
    #[account(
        init_if_needed,
        seeds = [
            SEED_THREAD_FIBER,
            thread.key().as_ref(),
            &[0], // Always use index 0 for initial fiber
        ],
        bump,
        payer = payer,
        space = 8 + FiberState::INIT_SPACE
    )]
    pub fiber: Option<Account<'info, FiberState>>,

    /// CHECK: Nonce account that must be passed in as a signer
    #[account(mut)]
    pub nonce_account: Option<Signer<'info>>,

    /// CHECK: Recent blockhashes sysvar required for nonce account operations
    #[account(address = recent_blockhashes::ID)]
    pub recent_blockhashes: Option<AccountInfo<'info>>,

    /// CHECK: Rent sysvar required for nonce account operations
    #[account(address = rent::ID)]
    pub rent: Option<AccountInfo<'info>>,

    #[account(address = system_program::ID)]
    pub system_program: Program<'info, System>,
}

pub fn thread_create(
    ctx: Context<ThreadCreate>,
    amount: u64,
    id: ThreadId,
    trigger: Trigger,
    initial_instruction: Option<SerializableInstruction>,
    priority_fee: Option<u64>,
) -> Result<()> {
    let authority: &Signer = &ctx.accounts.authority;
    let payer: &Signer = &ctx.accounts.payer;
    let thread: &mut Account<Thread> = &mut ctx.accounts.thread;
    let system_program: &Program<System> = &ctx.accounts.system_program;

    // Check if all required accounts for nonce creation are provided
    let create_durable_thread = ctx.accounts.nonce_account.is_some()
        && ctx.accounts.recent_blockhashes.is_some()
        && ctx.accounts.rent.is_some();

    if create_durable_thread {
        // All required accounts are present, create the nonce account
        let nonce_account = ctx.accounts.nonce_account.as_ref().unwrap();
        let recent_blockhashes = ctx.accounts.recent_blockhashes.as_ref().unwrap();
        let rent_program = ctx.accounts.rent.as_ref().unwrap();

        let rent: Rent = Rent::get()?;
        let nonce_account_size: usize = State::size();
        let nonce_lamports: u64 = rent.minimum_balance(nonce_account_size);

        create_nonce_account(
            CpiContext::new(
                system_program.to_account_info(),
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
    thread.paused = false;
    thread.trigger = trigger.clone();

    // Initialize trigger_context based on trigger type
    // Use created_at as initial prev value for proper fee calculation on first execution
    thread.trigger_context = match trigger {
        Trigger::Account { .. } => TriggerContext::Account { hash: 0 },
        Trigger::Cron { schedule, .. } => {
            let next = next_timestamp(current_timestamp, schedule).unwrap_or(current_timestamp);
            TriggerContext::Timestamp {
                prev: current_timestamp, // Use creation time as initial prev
                next,
            }
        }
        Trigger::Now => TriggerContext::Timestamp {
            prev: current_timestamp, // Use creation time as initial prev
            next: current_timestamp,
        },
        Trigger::Slot { slot } => TriggerContext::Block {
            prev: clock.slot, // Use current slot as initial prev
            next: slot,
        },
        Trigger::Epoch { epoch } => TriggerContext::Block {
            prev: clock.epoch, // Use current epoch as initial prev
            next: epoch,
        },
        Trigger::Interval { seconds, .. } => TriggerContext::Timestamp {
            prev: current_timestamp, // Use creation time as initial prev
            next: current_timestamp.saturating_add(seconds),
        },
        Trigger::Timestamp { unix_ts } => TriggerContext::Timestamp {
            prev: current_timestamp, // Use creation time as initial prev
            next: unix_ts,
        },
    };

    // Handle optional initial instruction
    if let Some(instruction) = initial_instruction {
        // Create initial fiber (index 0)
        if let Some(fiber_account) = &mut ctx.accounts.fiber {
            // Convert to regular Instruction
            let instruction: Instruction = instruction.into();

            // Use thread's PDA seeds for signer seeds
            let signer_seeds = vec![vec![
                SEED_THREAD.to_vec(),
                thread.authority.to_bytes().to_vec(),
                thread.id.clone(),
            ]];

            // Compile the instruction
            let compiled = compile_instruction(instruction, signer_seeds)?;
            let compiled_bytes = compiled.try_to_vec()?;

            // Initialize the fiber
            fiber_account.thread = thread.key();
            fiber_account.index = 0;
            fiber_account.compiled_instruction = compiled_bytes;
            fiber_account.priority_fee = priority_fee.unwrap_or(0);

            // Add fiber to thread's fiber mapping
            thread.fibers = vec![0];
        }
    } else {
        // No initial instruction, create empty thread
        thread.fibers = Vec::new();
    }

    thread.exec_index = 0;
    thread.exec_count = 0; // Initialize execution counter
    thread.last_executor = Pubkey::default(); // Initialize with default for load balancing

    // Transfer SOL from payer to the thread.
    transfer(
        CpiContext::new(
            system_program.to_account_info(),
            Transfer {
                from: payer.to_account_info(),
                to: thread.to_account_info(),
            },
        ),
        amount,
    )?;

    Ok(())
}
