use std::mem::size_of;

use anchor_lang::{
    prelude::*,
    solana_program::system_program,
    system_program::{transfer, Transfer}
};
use antegen_utils::thread::{Trigger, SerializableInstruction};

use crate::{state::*, ThreadId};

/// The minimum exec fee that may be set on a thread.
const MINIMUM_FEE: u64 = 1000;

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

    /// The Solana system program.
    #[account(address = system_program::ID)]
    pub system_program: Program<'info, System>,

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
}

pub fn handler(
    ctx: Context<ThreadCreate>,
    amount: u64,
    id: ThreadId,
    instructions: Vec<SerializableInstruction>,
    trigger: Trigger
) -> Result<()> {
    let id_bytes: Vec<u8> = match &id {
        ThreadId::Bytes(bytes) => bytes.clone(),
        ThreadId::Pubkey(pubkey) => pubkey.to_bytes().to_vec(),
    };

    // Get accounts
    let authority: &Signer = &ctx.accounts.authority;
    let payer: &Signer = &ctx.accounts.payer;
    let system_program: &Program<System> = &ctx.accounts.system_program;
    let thread: &mut Account<Thread> = &mut ctx.accounts.thread;

    // Initialize the thread with the current version
    thread.version = CURRENT_THREAD_VERSION;
    thread.authority = authority.key();
    thread.bump = ctx.bumps.thread;
    thread.created_at = Clock::get().unwrap().into();
    thread.exec_context = None;
    thread.fee = MINIMUM_FEE;
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

    // Transfer SOL from payer to the thread.
    transfer(
        CpiContext::new(
            system_program.to_account_info(),
            Transfer {
                from: payer.to_account_info(),
                to: thread.to_account_info(),
            },
        ),
        amount
    )?;

    msg!("Thread created with version {}", CURRENT_THREAD_VERSION);
    Ok(())
}
