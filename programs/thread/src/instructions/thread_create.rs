use crate::{
    state::{Trigger, TriggerContext},
    utils::next_timestamp,
    *,
};
use anchor_lang::{
    prelude::*,
    solana_program::{
        nonce::state::State,
        system_program,
        sysvar::{recent_blockhashes, rent},
    },
    system_program::{create_nonce_account, transfer, CreateNonceAccount, Transfer},
};

/// Accounts required by the `thread_create` instruction.
#[derive(Accounts)]
#[instruction(amount: u64, id: ThreadId, trigger: Trigger)]
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
        // No nonce account, use system_program::ID as sentinel value
        thread.nonce_account = system_program::ID;
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
    thread.trigger_context = match trigger {
        Trigger::Account { .. } => TriggerContext::Account { hash: 0 },
        Trigger::Cron { schedule, .. } => {
            let next = next_timestamp(current_timestamp, schedule).unwrap_or(current_timestamp);
            TriggerContext::Timestamp { prev: 0, next }
        }
        Trigger::Now => TriggerContext::Timestamp {
            prev: 0,
            next: current_timestamp,
        },
        Trigger::Slot { slot } => TriggerContext::Block {
            prev: 0,
            next: slot,
        },
        Trigger::Epoch { epoch } => TriggerContext::Block {
            prev: 0,
            next: epoch,
        },
        Trigger::Interval { seconds, .. } => TriggerContext::Timestamp {
            prev: 0,
            next: current_timestamp.saturating_add(seconds),
        },
        Trigger::Timestamp { unix_ts } => TriggerContext::Timestamp {
            prev: 0,
            next: unix_ts,
        },
    };

    // Handle optional initial instruction
    thread.fibers = Vec::new(); // No fibers initially, use fiber_create to add them

    thread.exec_index = 0;

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
