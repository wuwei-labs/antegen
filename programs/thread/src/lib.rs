pub mod constants;
pub mod errors;
pub mod instructions;
pub mod state;
pub mod utils;

/// Fiber program re-exports for direct CPI access.
/// Use when your program is executed via `thread_exec` and needs to
/// manage fibers directly (cannot CPI back to thread program due to reentrancy).
pub mod fiber {
    pub use antegen_fiber_program::cpi;
    pub use antegen_fiber_program::program::AntegenFiber;
    pub use antegen_fiber_program::state::{
        decompile_instruction,
        CompiledInstructionV0,
        Fiber,
        FiberState,
        FiberVersionedState,
        // Re-exported because it appears in this program's instruction
        // signatures: anything constructing `CreateFiber`/`UpdateFiber` needs it.
        Trailing,
    };
    pub use antegen_fiber_program::ID;
}

pub use crate::program::AntegenThread;
pub use constants::*;
use instructions::*;
use state::*;

use anchor_lang::prelude::*;
use state::{SerializableInstruction, Trigger};

declare_id!("AgTv5w1UvUb6zeqkThwMrztGu9hpepBu8YLghuR4dpSx");

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

    pub fn is_empty(&self) -> bool {
        match self {
            ThreadId::Bytes(bytes) => bytes.is_empty(),
            ThreadId::Pubkey(_) => false,
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
pub mod antegen_thread {
    use super::*;

    /// Initialize the global thread configuration.
    pub fn init_config(ctx: Context<ConfigInit>) -> Result<()> {
        config_init(ctx)
    }

    /// Update the global thread configuration.
    pub fn update_config(ctx: Context<ConfigUpdate>, params: ConfigUpdateParams) -> Result<()> {
        config_update(ctx, params)
    }

    /// Creates a fiber (instruction) for a thread via CPI to Fiber Program.
    /// `lookup_tables` is capped at 4 per fiber (Solana v0 transaction limit).
    pub fn create_fiber(
        ctx: Context<FiberCreate>,
        fiber_index: u8,
        instruction: SerializableInstruction,
        priority_fee: u64,
        lookup_tables: antegen_fiber_program::state::Trailing<Vec<Pubkey>>,
    ) -> Result<()> {
        fiber_create(
            ctx,
            fiber_index,
            instruction,
            priority_fee,
            lookup_tables.into_inner(),
        )
    }

    /// Closes a fiber from a thread via CPI to Fiber Program.
    pub fn close_fiber(ctx: Context<FiberClose>, fiber_index: u8) -> Result<()> {
        fiber_close(ctx, fiber_index)
    }

    /// Updates a fiber's instruction via CPI to Fiber Program.
    /// Initializes the fiber if it doesn't exist (thread PDA pays rent).
    /// If `track` is true, adds the fiber_index to thread.fiber_ids.
    /// Pass `None` for `instruction` to wipe the compiled instruction (idle).
    /// Pass `None` for `lookup_tables` to leave them unchanged; `Some(vec)`
    /// atomically replaces. Legacy fibers reject non-empty lookup_tables.
    pub fn update_fiber(
        ctx: Context<FiberUpdate>,
        fiber_index: u8,
        instruction: Option<SerializableInstruction>,
        priority_fee: Option<u64>,
        track: bool,
        lookup_tables: antegen_fiber_program::state::Trailing<Option<Vec<Pubkey>>>,
    ) -> Result<()> {
        fiber_update(
            ctx,
            fiber_index,
            instruction,
            priority_fee,
            track,
            lookup_tables.into_inner(),
        )
    }

    /// Swaps source fiber's instruction into target fiber, closes source.
    /// Target keeps its PDA/index, source is deleted.
    pub fn swap_fiber(ctx: Context<FiberSwap>, source_fiber_index: u8) -> Result<()> {
        instructions::fiber_swap::fiber_swap(ctx, source_fiber_index)
    }

    /// Creates a new transaction thread.
    /// Optionally creates fiber index 0 if `instruction` is provided.
    /// `lookup_tables` is forwarded to fiber_0 when one is created;
    /// it is ignored when `instruction` is `None`.
    pub fn create_thread(
        ctx: Context<ThreadCreate>,
        amount: u64,
        id: ThreadId,
        trigger: Trigger,
        paused: Option<bool>,
        instruction: Option<SerializableInstruction>,
        priority_fee: Option<u64>,
        lookup_tables: antegen_fiber_program::state::Trailing<Vec<Pubkey>>,
    ) -> Result<()> {
        thread_create(
            ctx,
            amount,
            id,
            trigger,
            paused,
            instruction,
            priority_fee,
            lookup_tables.into_inner(),
        )
    }

    /// Closes an existing thread account and returns the lamports to the owner.
    /// Requires authority (owner) or thread itself to sign.
    /// External fiber accounts should be passed via remaining_accounts.
    pub fn close_thread<'info>(ctx: Context<'info, ThreadClose<'info>>) -> Result<()> {
        thread_close(ctx)
    }

    /// Executes a thread fiber with trigger validation and fee distribution.
    /// Respects builder claim priority windows from registry configuration.
    pub fn exec_thread<'info>(
        ctx: Context<'info, ThreadExec<'info>>,
        forgo_commission: bool,
        fiber_cursor: u8,
    ) -> Result<()> {
        thread_exec(ctx, forgo_commission, fiber_cursor)
    }

    /// Allows an owner to update the thread's properties (paused state, trigger).
    pub fn update_thread(ctx: Context<ThreadUpdate>, params: ThreadUpdateParams) -> Result<()> {
        thread_update(ctx, params)
    }

    /// Allows an owner to withdraw from a thread's lamport balance.
    pub fn withdraw_thread(ctx: Context<ThreadWithdraw>, amount: u64) -> Result<()> {
        thread_withdraw(ctx, amount)
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

    /// Deletes a thread - admin only, skips all checks.
    /// Used for cleaning up stuck/broken threads.
    pub fn delete_thread(ctx: Context<ThreadDelete>) -> Result<()> {
        thread_delete(ctx)
    }
}

#[cfg(test)]
mod wire_compat_tests {
    use super::*;
    use anchor_lang::AnchorDeserialize;

    /// Instruction data exactly as a caller built against the 5.0.12 IDL sends
    /// it: everything through `track`, and then nothing.
    ///
    /// The mainnet contract program CPIs into `update_fiber` and was compiled
    /// against that surface. Deploying a program that cannot decode this would
    /// fail every one of its fiber operations until it was rebuilt and
    /// redeployed in lockstep.
    #[test]
    fn update_fiber_decodes_data_without_the_appended_argument() {
        let mut old_format = Vec::new();
        7u8.serialize(&mut old_format).unwrap(); // fiber_index
        None::<SerializableInstruction>
            .serialize(&mut old_format)
            .unwrap(); // instruction
        Some(42u64).serialize(&mut old_format).unwrap(); // priority_fee
        true.serialize(&mut old_format).unwrap(); // track
                                                  // no lookup_tables — the caller does not know the argument exists

        let ix = instruction::UpdateFiber::deserialize(&mut &old_format[..])
            .expect("data from a pre-lookup_tables caller must still decode");

        assert_eq!(ix.fiber_index, 7);
        assert_eq!(ix.priority_fee, Some(42));
        assert!(ix.track);
        assert_eq!(
            ix.lookup_tables.into_inner(),
            None,
            "an absent argument means no lookup tables, as it did before it existed"
        );
    }

    /// The new format must still decode, or the program cannot read its own
    /// clients.
    #[test]
    fn update_fiber_decodes_data_with_the_appended_argument() {
        let tables = vec![Pubkey::new_unique(), Pubkey::new_unique()];
        let mut new_format = Vec::new();
        7u8.serialize(&mut new_format).unwrap();
        None::<SerializableInstruction>
            .serialize(&mut new_format)
            .unwrap();
        Some(42u64).serialize(&mut new_format).unwrap();
        true.serialize(&mut new_format).unwrap();
        Some(tables.clone()).serialize(&mut new_format).unwrap();

        let ix = instruction::UpdateFiber::deserialize(&mut &new_format[..]).unwrap();
        assert_eq!(ix.lookup_tables.into_inner(), Some(tables));
    }

    #[test]
    fn create_fiber_decodes_both_wire_formats() {
        let ix_data = SerializableInstruction {
            program_id: Pubkey::new_unique(),
            accounts: vec![],
            data: vec![],
        };

        let mut old_format = Vec::new();
        3u8.serialize(&mut old_format).unwrap();
        ix_data.serialize(&mut old_format).unwrap();
        0u64.serialize(&mut old_format).unwrap();

        let old = instruction::CreateFiber::deserialize(&mut &old_format[..]).unwrap();
        assert!(old.lookup_tables.into_inner().is_empty());

        let tables = vec![Pubkey::new_unique()];
        let mut new_format = old_format.clone();
        tables.serialize(&mut new_format).unwrap();

        let new = instruction::CreateFiber::deserialize(&mut &new_format[..]).unwrap();
        assert_eq!(new.lookup_tables.into_inner(), tables);
    }
}
