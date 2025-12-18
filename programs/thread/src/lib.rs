pub mod constants;
pub mod errors;
pub mod instructions;
pub mod state;
pub mod utils;

pub use constants::*;
use instructions::*;
use state::*;

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::Instruction;
use state::{SerializableInstruction, Trigger};

declare_id!("AgV3xRAdyTe1wW4gTW2oAnzHiAGofsxC7jBVGGkzUQbY");

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

    pub fn to_name(&self) -> String {
        match self {
            ThreadId::Bytes(bytes) => String::from_utf8_lossy(bytes).to_string(),
            ThreadId::Pubkey(pubkey) => pubkey.to_string(),
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

impl From<ThreadId> for Vec<u8> {
    fn from(id: ThreadId) -> Vec<u8> {
        match id {
            ThreadId::Bytes(bytes) => bytes,
            ThreadId::Pubkey(pubkey) => pubkey.to_bytes().to_vec(),
        }
    }
}

#[program]
pub mod thread_program {
    use super::*;

    /// Initialize the global thread configuration.
    pub fn init_config(ctx: Context<ConfigInit>) -> Result<()> {
        config_init(ctx)
    }

    /// Update the global thread configuration.
    pub fn update_config(ctx: Context<ConfigUpdate>, params: ConfigUpdateParams) -> Result<()> {
        config_update(ctx, params)
    }

    /// Creates a fiber (instruction) for a thread.
    pub fn create_fiber(
        ctx: Context<FiberCreate>,
        fiber_index: u8,
        instruction: SerializableInstruction,
        signer_seeds: Vec<Vec<Vec<u8>>>,
        priority_fee: u64,
    ) -> Result<()> {
        let instruction: Instruction = instruction.into();
        fiber_create(ctx, fiber_index, instruction, signer_seeds, priority_fee)
    }

    /// Deletes a fiber from a thread.
    pub fn delete_fiber(ctx: Context<FiberDelete>, fiber_index: u8) -> Result<()> {
        fiber_delete(ctx, fiber_index)
    }

    /// Updates a fiber's instruction and resets execution stats.
    pub fn update_fiber(
        ctx: Context<FiberUpdate>,
        instruction: SerializableInstruction,
    ) -> Result<()> {
        let instruction: Instruction = instruction.into();
        fiber_update(ctx, instruction)
    }

    /// Creates a new transaction thread.
    /// Optionally creates an initial fiber if instruction is provided.
    pub fn create_thread(
        ctx: Context<ThreadCreate>,
        amount: u64,
        id: ThreadId,
        trigger: Trigger,
        initial_instruction: Option<SerializableInstruction>,
        priority_fee: Option<u64>,
    ) -> Result<()> {
        thread_create(ctx, amount, id, trigger, initial_instruction, priority_fee)
    }

    /// Closes an existing thread account and returns the lamports to the owner.
    /// Requires authority (owner) or thread itself to sign.
    /// External fiber accounts should be passed via remaining_accounts.
    pub fn delete_thread(ctx: Context<ThreadDelete>) -> Result<()> {
        thread_delete(ctx)
    }

    /// Executes a thread fiber with trigger validation and fee distribution.
    /// Respects builder claim priority windows from registry configuration.
    pub fn exec_thread(
        ctx: Context<ThreadExec>,
        forgo_commission: bool,
        fiber_cursor: u8,
    ) -> Result<()> {
        thread_exec(ctx, forgo_commission, fiber_cursor)
    }

    /// Toggles a thread's pause state.
    pub fn toggle_thread(ctx: Context<ThreadToggle>) -> Result<()> {
        thread_toggle(ctx)
    }

    /// Allows an owner to update the thread's trigger.
    pub fn update_thread(ctx: Context<ThreadUpdate>, new_trigger: Option<Trigger>) -> Result<()> {
        thread_update(ctx, new_trigger)
    }

    /// Allows an owner to withdraw from a thread's lamport balance.
    pub fn withdraw_thread(ctx: Context<ThreadWithdraw>, amount: u64) -> Result<()> {
        thread_withdraw(ctx, amount)
    }

    /// Reports an error for a thread that failed to execute.
    pub fn error_thread(
        ctx: Context<ThreadError>,
        error_code: u32,
        error_message: String,
    ) -> Result<()> {
        thread_error(ctx, error_code, error_message)
    }

    /// Memo instruction that logs a message (replacement for spl-memo).
    /// Used for tracking thread fiber execution in logs without external dependencies.
    /// Optionally emits a signal for testing signal behaviors.
    pub fn thread_memo(
        ctx: Context<ThreadMemo>,
        memo: String,
        signal: Option<Signal>,
    ) -> Result<Signal> {
        instructions::thread_memo::thread_memo(ctx, memo, signal)
    }

    /// Force deletes a thread - admin only, skips all checks.
    /// Used for cleaning up stuck/broken threads during development.
    pub fn force_delete_thread(ctx: Context<ThreadForceDelete>) -> Result<()> {
        thread_force_delete(ctx)
    }
}
