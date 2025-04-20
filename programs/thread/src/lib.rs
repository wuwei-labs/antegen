//! This program allows users to create transaction threads on Solana. Threads are dynamic, long-running
//! transaction threads that can persist across blocks and even run indefinitely. Developers can use threads
//! to schedule transactions and automate smart-contracts without relying on centralized infrastructure.
pub mod errors;
mod instructions;
pub mod state;

use anchor_lang::prelude::*;
use antegen_utils::thread::{SerializableInstruction, Trigger};
use instructions::*;
use state::*;

declare_id!("AgThdyi1P5RkVeZD2rQahTvs8HePJoGFFxKtvok5s2J1");

pub const TRANSACTION_BASE_FEE_REIMBURSEMENT: u64 = 5_000;
pub const THREAD_MINIMUM_FEE: u64 = 1_000;

#[derive(AnchorSerialize, AnchorDeserialize)]
pub enum ThreadId {
    Bytes(Vec<u8>),
    Pubkey(Pubkey),
}

impl AsRef<[u8]> for ThreadId {
    fn as_ref(&self) -> &[u8] {
        match self {
            ThreadId::Bytes(bytes) => bytes.as_ref(),
            ThreadId::Pubkey(pubkey) => pubkey.as_ref(),
        }
    }
}

impl ThreadId {
    pub fn len(&self) -> usize {
        match self {
            ThreadId::Bytes(bytes) => bytes.len(),
            ThreadId::Pubkey(_) => 32,
        }
    }
}

impl From<String> for ThreadId {
    fn from(s: String) -> Self {
        ThreadId::Bytes(s.into_bytes())
    }
}

impl From<&str> for ThreadId {
    fn from(s: &str) -> Self {
        ThreadId::Bytes(s.as_bytes().to_vec())
    }
}

impl From<Pubkey> for ThreadId {
    fn from(pubkey: Pubkey) -> Self {
        ThreadId::Pubkey(pubkey)
    }
}

#[program]
pub mod thread_program {
    use super::*;

    /// Advance nonce account for thread.
    pub fn thread_claim(ctx: Context<ThreadClaim>, hash: String) -> Result<()> {
        thread_claim::handler(ctx, hash)
    }

    /// Executes the next instruction on thread.
    pub fn thread_exec(ctx: Context<ThreadExec>) -> Result<()> {
        thread_exec::handler(ctx)
    }

    /// Creates a new transaction thread.
    pub fn thread_create(
        ctx: Context<ThreadCreate>,
        amount: u64,
        id: ThreadId,
        instructions: Vec<SerializableInstruction>,
        trigger: Trigger,
    ) -> Result<()> {
        thread_create::handler(ctx, amount, id, instructions, trigger)
    }

    /// Closes an existing thread account and returns the lamports to the owner.
    pub fn thread_delete(ctx: Context<ThreadDelete>) -> Result<()> {
        thread_delete::handler(ctx)
    }

    /// Kicks off a thread if its trigger condition is active.
    pub fn thread_kickoff(ctx: Context<ThreadKickoff>) -> Result<()> {
        thread_kickoff::handler(ctx)
    }

    /// Pauses an active thread.
    pub fn thread_pause(ctx: Context<ThreadPause>) -> Result<()> {
        thread_pause::handler(ctx)
    }

    /// Resumes a paused thread.
    pub fn thread_resume(ctx: Context<ThreadResume>) -> Result<()> {
        thread_resume::handler(ctx)
    }

    /// Resets a thread's next instruction.
    pub fn thread_reset(ctx: Context<ThreadReset>) -> Result<()> {
        thread_reset::handler(ctx)
    }

    /// Allows an owner to update the mutable properties of a thread.
    pub fn thread_update(ctx: Context<ThreadUpdate>, settings: ThreadSettings) -> Result<()> {
        thread_update::handler(ctx, settings)
    }

    /// Allows an owner to withdraw from a thread's lamport balance.
    pub fn thread_withdraw(ctx: Context<ThreadWithdraw>, amount: u64) -> Result<()> {
        thread_withdraw::handler(ctx, amount)
    }
}
