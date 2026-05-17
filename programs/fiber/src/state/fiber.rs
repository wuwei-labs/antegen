use crate::constants::*;
use crate::errors::AntegenFiberError;
use crate::state::instruction::*;
use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::Instruction;

/// Current version stamped onto newly written `FiberVersionedState` accounts.
pub const CURRENT_FIBER_VERSION: u8 = 1;

/// Per-fiber hard cap on lookup tables — matches Solana's v0-tx ALT cap.
pub const MAX_LOOKUP_TABLES_PER_FIBER: usize = 4;

/// Trait for processing fiber instructions
pub trait FiberInstructionProcessor {
    /// Get the decompiled instruction from the fiber's compiled data,
    /// replacing PAYER_PUBKEY with the provided executor
    fn get_instruction(&self, executor: &Pubkey) -> Result<Instruction>;
}

/// Legacy fiber state — predates ALT support. Still readable on mainnet,
/// never re-allocated. New writes always produce `FiberVersionedState`.
#[account]
#[derive(Debug, InitSpace)]
pub struct FiberState {
    /// The thread this fiber belongs to
    pub thread: Pubkey,
    /// The compiled instruction data
    #[max_len(1024)]
    pub compiled_instruction: Vec<u8>,
    /// When this fiber was last executed
    pub last_executed: i64,
    /// Total number of executions
    pub exec_count: u64,
    /// Priority fee in microlamports for compute unit price (0 = no priority fee)
    pub priority_fee: u64,
}

impl FiberState {
    /// Derive the pubkey of a fiber account.
    pub fn pubkey(thread: Pubkey, fiber_index: u8) -> Pubkey {
        Pubkey::find_program_address(
            &[SEED_THREAD_FIBER, thread.as_ref(), &[fiber_index]],
            &crate::ID,
        )
        .0
    }
}

impl FiberInstructionProcessor for FiberState {
    fn get_instruction(&self, executor: &Pubkey) -> Result<Instruction> {
        decompile_with_payer(&self.compiled_instruction, executor)
    }
}

/// Versioned fiber state. Carries `version` as the first field so future
/// migrations have a leading gate, and trailing `lookup_tables` so v0 message
/// compilation can attach an ALT union without runtime account churn.
#[account]
#[derive(Debug, InitSpace)]
pub struct FiberVersionedState {
    /// State version. Currently 1.
    pub version: u8,
    /// The thread this fiber belongs to
    pub thread: Pubkey,
    /// The compiled instruction data
    #[max_len(1024)]
    pub compiled_instruction: Vec<u8>,
    /// When this fiber was last executed
    pub last_executed: i64,
    /// Total number of executions
    pub exec_count: u64,
    /// Priority fee in microlamports for compute unit price (0 = no priority fee)
    pub priority_fee: u64,
    /// Address Lookup Tables consumed by this fiber. Capped at 4 (Solana v0 limit).
    #[max_len(4)]
    pub lookup_tables: Vec<Pubkey>,
}

impl FiberVersionedState {
    pub fn pubkey(thread: Pubkey, fiber_index: u8) -> Pubkey {
        FiberState::pubkey(thread, fiber_index)
    }
}

impl FiberInstructionProcessor for FiberVersionedState {
    fn get_instruction(&self, executor: &Pubkey) -> Result<Instruction> {
        decompile_with_payer(&self.compiled_instruction, executor)
    }
}

/// Discriminator-tagged read view over either fiber shape on disk.
///
/// Implements [`AccountDeserialize`] so callers use the same
/// `try_deserialize(&mut buf)` shape as any other Anchor account type — the
/// trait impl peeks the leading 8-byte discriminator and routes to the
/// matching state struct's deserializer.
#[derive(Debug)]
pub enum Fiber {
    Legacy(FiberState),
    V1(FiberVersionedState),
}

impl anchor_lang::AccountDeserialize for Fiber {
    fn try_deserialize(buf: &mut &[u8]) -> Result<Self> {
        Self::try_deserialize_unchecked(buf)
    }

    fn try_deserialize_unchecked(buf: &mut &[u8]) -> Result<Self> {
        if buf.len() < 8 {
            return Err(error!(AntegenFiberError::InvalidFiberData));
        }
        let disc = &buf[..8];
        if disc == FiberVersionedState::DISCRIMINATOR {
            let state = FiberVersionedState::try_deserialize(buf)?;
            Ok(Self::V1(state))
        } else if disc == FiberState::DISCRIMINATOR {
            let state = FiberState::try_deserialize(buf)?;
            Ok(Self::Legacy(state))
        } else {
            Err(error!(AntegenFiberError::InvalidFiberData))
        }
    }
}

impl Fiber {
    pub fn is_legacy(&self) -> bool {
        matches!(self, Self::Legacy(_))
    }

    pub fn thread(&self) -> Pubkey {
        match self {
            Self::Legacy(s) => s.thread,
            Self::V1(s) => s.thread,
        }
    }

    pub fn compiled_instruction(&self) -> &[u8] {
        match self {
            Self::Legacy(s) => &s.compiled_instruction,
            Self::V1(s) => &s.compiled_instruction,
        }
    }

    pub fn priority_fee(&self) -> u64 {
        match self {
            Self::Legacy(s) => s.priority_fee,
            Self::V1(s) => s.priority_fee,
        }
    }

    pub fn lookup_tables(&self) -> &[Pubkey] {
        match self {
            Self::Legacy(_) => &[],
            Self::V1(s) => &s.lookup_tables,
        }
    }
}

impl FiberInstructionProcessor for Fiber {
    fn get_instruction(&self, executor: &Pubkey) -> Result<Instruction> {
        match self {
            Self::Legacy(s) => s.get_instruction(executor),
            Self::V1(s) => s.get_instruction(executor),
        }
    }
}

fn decompile_with_payer(compiled_instruction: &[u8], executor: &Pubkey) -> Result<Instruction> {
    let compiled = CompiledInstructionV0::try_from_slice(compiled_instruction)?;
    let mut instruction = decompile_instruction(&compiled)?;

    for acc in instruction.accounts.iter_mut() {
        if acc.pubkey.eq(&PAYER_PUBKEY) {
            acc.pubkey = *executor;
        }
    }

    Ok(instruction)
}
