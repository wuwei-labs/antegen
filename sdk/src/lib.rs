pub use antegen_thread_program::errors;
pub use antegen_thread_program::program::ThreadProgram;
pub use antegen_thread_program::ThreadId;
pub use antegen_thread_program::ID;

pub mod seeds {
    pub use antegen_thread_program::{SEED_THREAD, SEED_THREAD_FIBER};
}

pub mod state {
    pub use antegen_thread_program::state::{FiberState, Thread};
    pub use antegen_utils::thread::{
        CompiledInstructionV0, CompiledTransactionV0, SerializableAccountMeta,
        SerializableInstruction, ThreadResponse, Trigger, TriggerContext, PAYER_PUBKEY,
    };
}

pub mod cpi {
    use anchor_lang::prelude::{CpiContext, Result};
    use antegen_utils::thread::{SerializableInstruction, Trigger};

    pub use antegen_thread_program::cpi::accounts::{
        FiberCreate, FiberDelete, ThreadCreate, ThreadDelete, ThreadToggle, ThreadUpdate,
        ThreadWithdraw,
    };
    use antegen_thread_program::ThreadId;

    pub fn fiber_create<'info>(
        ctx: CpiContext<'_, '_, '_, 'info, FiberCreate<'info>>,
        index: u8,
        instruction: SerializableInstruction,
        signer_seeds: Vec<Vec<Vec<u8>>>,
    ) -> Result<()> {
        antegen_thread_program::cpi::fiber_create(ctx, index, instruction, signer_seeds)
    }

    pub fn fiber_delete<'info>(
        ctx: CpiContext<'_, '_, '_, 'info, FiberDelete<'info>>,
        index: u8,
    ) -> Result<()> {
        antegen_thread_program::cpi::fiber_delete(ctx, index)
    }

    pub fn thread_create<'info>(
        ctx: CpiContext<'_, '_, '_, 'info, ThreadCreate<'info>>,
        amount: u64,
        id: ThreadId,
        trigger: Trigger,
    ) -> Result<()> {
        antegen_thread_program::cpi::thread_create(ctx, amount, id, trigger)
    }

    pub fn thread_delete<'info>(
        ctx: CpiContext<'_, '_, '_, 'info, ThreadDelete<'info>>,
    ) -> Result<()> {
        antegen_thread_program::cpi::thread_delete(ctx)
    }

    pub fn thread_toggle<'info>(
        ctx: CpiContext<'_, '_, '_, 'info, ThreadToggle<'info>>,
    ) -> Result<()> {
        antegen_thread_program::cpi::thread_toggle(ctx)
    }

    pub fn thread_update<'info>(
        ctx: CpiContext<'_, '_, '_, 'info, ThreadUpdate<'info>>,
        new_trigger: Option<Trigger>,
    ) -> Result<()> {
        antegen_thread_program::cpi::thread_update(ctx, new_trigger)
    }

    pub fn thread_withdraw<'info>(
        ctx: CpiContext<'_, '_, '_, 'info, ThreadWithdraw<'info>>,
        amount: u64,
    ) -> Result<()> {
        antegen_thread_program::cpi::thread_withdraw(ctx, amount)
    }
}
