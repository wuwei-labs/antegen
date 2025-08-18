pub use antegen_thread_program::errors;
pub use antegen_thread_program::program::ThreadProgram;
pub use antegen_thread_program::ThreadId;
pub use antegen_thread_program::ID;

// Re-export instruction builders and data
pub mod instruction {
    pub use antegen_thread_program::instruction::*;
}

// Re-export account structs
pub mod accounts {
    pub use antegen_thread_program::accounts::*;
}

pub mod seeds {
    pub use antegen_thread_program::{SEED_CONFIG, SEED_NONCE, SEED_THREAD, SEED_THREAD_FIBER};
}

pub mod state {
    pub use antegen_thread_program::state::{
        compile_instruction, decompile_instruction, CompiledInstructionData, CompiledInstructionV0,
        FiberState, SerializableAccountMeta, SerializableInstruction, Thread, ThreadConfig,
        ThreadResponse, Trigger, TriggerContext, PAYER_PUBKEY,
    };
    // ConfigUpdateParams is in the instructions module, not state
    pub use antegen_thread_program::instructions::ConfigUpdateParams;
}

pub mod cpi {
    use anchor_lang::prelude::{CpiContext, Result};
    use antegen_thread_program::instructions::ConfigUpdateParams;
    use antegen_thread_program::state::{SerializableInstruction, Trigger};

    pub use antegen_thread_program::cpi::accounts::*;
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

    pub fn config_init<'info>(ctx: CpiContext<'_, '_, '_, 'info, ConfigInit<'info>>) -> Result<()> {
        antegen_thread_program::cpi::config_init(ctx)
    }

    pub fn config_update<'info>(
        ctx: CpiContext<'_, '_, '_, 'info, ConfigUpdate<'info>>,
        params: ConfigUpdateParams,
    ) -> Result<()> {
        antegen_thread_program::cpi::config_update(ctx, params)
    }

    pub fn thread_claim<'info>(
        ctx: CpiContext<'_, '_, '_, 'info, ThreadClaim<'info>>,
    ) -> Result<()> {
        antegen_thread_program::cpi::thread_claim(ctx)
    }

    pub fn thread_exec<'info>(ctx: CpiContext<'_, '_, '_, 'info, ThreadExec<'info>>) -> Result<()> {
        antegen_thread_program::cpi::thread_exec(ctx)
    }

    pub fn thread_kickoff<'info>(
        ctx: CpiContext<'_, '_, '_, 'info, ThreadKickoff<'info>>,
    ) -> Result<()> {
        antegen_thread_program::cpi::thread_kickoff(ctx)
    }
}
