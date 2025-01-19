use std::mem::size_of;

use anchor_lang::{
    prelude::*,
    solana_program::system_program,
    system_program::{transfer, Transfer}
};
use antegen_utils::thread::{Trigger, SerializableInstruction};

use crate::state::*;

/// The minimum exec fee that may be set on a thread.
const MINIMUM_FEE: u64 = 1000;

/// Accounts required by the `thread_create` instruction.
#[derive(Accounts)]
#[instruction(amount: u64, id: Vec<u8>, instructions: Vec<SerializableInstruction>,  trigger: Trigger)]
pub struct ThreadCreate<'info> {
    /// The authority (owner) of the thread.
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
        init,
        seeds = [
            SEED_THREAD,
            authority.key().as_ref(),
            id.as_slice(),
        ],
        bump,
        payer= payer,
        space = 8 +                              // discriminator
                size_of::<Thread>() +            // base struct
                id.len() +                       // id length
                4 + (instructions.len() * size_of::<SerializableInstruction>()) + // vec length prefix + items
                size_of::<Trigger>() +           // trigger enum
                NEXT_INSTRUCTION_SIZE            // next instruction
    )]
    pub thread: Account<'info, Thread>,
}

pub fn handler(ctx: Context<ThreadCreate>, amount: u64, id: Vec<u8>, instructions: Vec<SerializableInstruction>, trigger: Trigger) -> Result<()> {
    // Get accounts
    let authority = &ctx.accounts.authority;
    let payer = &ctx.accounts.payer;
    let system_program = &ctx.accounts.system_program;
    let thread = &mut ctx.accounts.thread;

    // Initialize the thread
    thread.authority = authority.key();
    thread.bump = ctx.bumps.thread;
    thread.created_at = Clock::get().unwrap().into();
    thread.exec_context = None;
    thread.fee = MINIMUM_FEE;
    thread.id = id;
    thread.instructions = instructions;
    thread.name = String::from_utf8_lossy(&thread.id).to_string();
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

    Ok(())
}
