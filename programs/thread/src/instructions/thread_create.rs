use std::mem::size_of;

use crate::{state::*, ThreadId, THREAD_MINIMUM_FEE};
use anchor_lang::{
    prelude::*,
    solana_program::{
        self, system_program,
        sysvar::{recent_blockhashes, rent},
    },
    system_program::{create_nonce_account, transfer, CreateNonceAccount, Transfer},
};
use antegen_utils::thread::{SerializableInstruction, Trigger};

/// Accounts required by the `thread_create` instruction.
#[derive(Accounts)]
#[instruction(amount: u64, id: ThreadId, instructions: Vec<SerializableInstruction>, trigger: Trigger)]
pub struct ThreadCreate<'info> {
    /// CHECK: the authority (owner) of the thread.
    #[account()]
    pub authority: Signer<'info>,

    /// The payer for account initializations.
    #[account(mut)]
    pub payer: Signer<'info>,

    /// The thread to be created.
    #[account(
        init_if_needed,
        seeds = [
            SEED_THREAD,
            authority.key().as_ref(),
            id.as_ref(),
        ],
        bump,
        payer = payer,
        space = 8 +                              // discriminator
                size_of::<Thread>() +            // base struct
                id.len() +                       // id length
                4 + (instructions.len() * size_of::<SerializableInstruction>()) + // vec length prefix + items
                size_of::<Trigger>() +           // trigger enum
                NEXT_INSTRUCTION_SIZE            // next instruction
    )]
    pub thread: Account<'info, Thread>,

    /// CHECK: Nonce account that must be passed in as a signer
    #[account(mut)]
    pub nonce_account: Signer<'info>,

    /// CHECK: Recent blockhashes sysvar required for nonce account operations
    #[account(address = recent_blockhashes::ID)]
    pub recent_blockhashes: AccountInfo<'info>,

    /// CHECK: Rent sysvar required for nonce account operations
    #[account(address = rent::ID)]
    pub rent: AccountInfo<'info>,

    #[account(address = system_program::ID)]
    pub system_program: Program<'info, System>,
}

pub fn handler(
    ctx: Context<ThreadCreate>,
    amount: u64,
    id: ThreadId,
    instructions: Vec<SerializableInstruction>,
    trigger: Trigger,
) -> Result<()> {
    let id_bytes: Vec<u8> = match &id {
        ThreadId::Bytes(bytes) => bytes.clone(),
        ThreadId::Pubkey(pubkey) => pubkey.to_bytes().to_vec(),
    };

    let authority: &Signer = &ctx.accounts.authority;
    let payer: &Signer = &ctx.accounts.payer;
    let nonce_account: &AccountInfo = &ctx.accounts.nonce_account;

    let thread: &mut Account<Thread> = &mut ctx.accounts.thread;
    let system_program: &Program<System> = &ctx.accounts.system_program;
    let recent_blockhashes: &AccountInfo = &ctx.accounts.recent_blockhashes;
    let rent_program: &AccountInfo = &ctx.accounts.rent;

    let rent: Rent = Rent::get()?;
    let nonce_account_size: usize = solana_program::nonce::state::State::size();
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

    // Initialize the thread
    thread.version = CURRENT_THREAD_VERSION;
    thread.authority = authority.key();
    thread.bump = ctx.bumps.thread;
    thread.created_at = Clock::get().unwrap().into();
    thread.exec_context = None;
    thread.fee = THREAD_MINIMUM_FEE;
    thread.id = id_bytes;
    thread.instructions = instructions;
    thread.name = match id {
        ThreadId::Bytes(bytes) => String::from_utf8_lossy(&bytes).to_string(),
        ThreadId::Pubkey(pubkey) => pubkey.to_string(),
    };
    thread.next_instruction = None;
    thread.paused = false;
    thread.rate_limit = u64::MAX;
    thread.trigger = trigger;
    thread.nonce_account = nonce_account.key();

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
