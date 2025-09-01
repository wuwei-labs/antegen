pub use antegen_thread_program::errors;
pub use antegen_thread_program::program::ThreadProgram;
pub use antegen_thread_program::ThreadId;
pub use antegen_thread_program::ID;

#[cfg(feature = "metrics")]
pub mod metrics;

pub mod rpc;
pub mod types;

// Re-export types needed by other crates
pub use types::{ProcessorMessage, TransactionMessage, DurableTransactionMessage};

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

/// Convenience functions for working with fibers
pub mod fiber {
    use super::state::SerializableInstruction;
    use anchor_lang::solana_program::instruction::Instruction;
    
    /// Convert a standard Solana instruction to a SerializableInstruction for use in fibers
    /// 
    /// # Example
    /// ```ignore
    /// use antegen_sdk::fiber;
    /// use solana_sdk::system_instruction;
    /// 
    /// let transfer = system_instruction::transfer(&from, &to, amount);
    /// let serializable = fiber::to_serializable(transfer);
    /// ```
    pub fn to_serializable(instruction: Instruction) -> SerializableInstruction {
        instruction.into()
    }
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
        antegen_thread_program::cpi::create_fiber(ctx, index, instruction, signer_seeds)
    }

    pub fn fiber_delete<'info>(
        ctx: CpiContext<'_, '_, '_, 'info, FiberDelete<'info>>,
        index: u8,
    ) -> Result<()> {
        antegen_thread_program::cpi::delete_fiber(ctx, index)
    }

    pub fn thread_create<'info>(
        ctx: CpiContext<'_, '_, '_, 'info, ThreadCreate<'info>>,
        amount: u64,
        id: ThreadId,
        trigger: Trigger,
    ) -> Result<()> {
        antegen_thread_program::cpi::create_thread(ctx, amount, id, trigger)
    }

    pub fn thread_delete<'info>(
        ctx: CpiContext<'_, '_, '_, 'info, ThreadDelete<'info>>,
    ) -> Result<()> {
        antegen_thread_program::cpi::delete_thread(ctx)
    }

    pub fn thread_toggle<'info>(
        ctx: CpiContext<'_, '_, '_, 'info, ThreadToggle<'info>>,
    ) -> Result<()> {
        antegen_thread_program::cpi::toggle_thread(ctx)
    }

    pub fn thread_update<'info>(
        ctx: CpiContext<'_, '_, '_, 'info, ThreadUpdate<'info>>,
        new_trigger: Option<Trigger>,
    ) -> Result<()> {
        antegen_thread_program::cpi::update_thread(ctx, new_trigger)
    }

    pub fn thread_withdraw<'info>(
        ctx: CpiContext<'_, '_, '_, 'info, ThreadWithdraw<'info>>,
        amount: u64,
    ) -> Result<()> {
        antegen_thread_program::cpi::withdraw_thread(ctx, amount)
    }

    pub fn config_init<'info>(ctx: CpiContext<'_, '_, '_, 'info, ConfigInit<'info>>) -> Result<()> {
        antegen_thread_program::cpi::init_config(ctx)
    }

    pub fn config_update<'info>(
        ctx: CpiContext<'_, '_, '_, 'info, ConfigUpdate<'info>>,
        params: ConfigUpdateParams,
    ) -> Result<()> {
        antegen_thread_program::cpi::update_config(ctx, params)
    }

    pub fn thread_exec<'info>(ctx: CpiContext<'_, '_, '_, 'info, ThreadExec<'info>>, forgo_commission: bool) -> Result<()> {
        antegen_thread_program::cpi::exec_thread(ctx, forgo_commission)
    }
}
