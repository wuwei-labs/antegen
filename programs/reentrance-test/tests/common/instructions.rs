use anchor_lang::{InstructionData, ToAccountMetas};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use super::setup::{FIBER_PROGRAM_ID, PROGRAM_ID};

// Re-export program types used by tests
pub use antegen_thread_program::state::{Signal, Trigger};
use antegen_thread_program::state::{SerializableAccountMeta, SerializableInstruction};
pub use antegen_thread_program::ThreadId;

// ============================================================================
// Config Instructions
// ============================================================================

pub fn build_init_config(admin: &Pubkey, config: &Pubkey) -> Instruction {
    Instruction {
        program_id: PROGRAM_ID,
        accounts: antegen_thread_program::accounts::ConfigInit {
            admin: *admin,
            config: *config,
            system_program: solana_system_interface::program::ID,
        }
        .to_account_metas(None),
        data: antegen_thread_program::instruction::InitConfig.data(),
    }
}

// ============================================================================
// Thread Instructions
// ============================================================================

pub fn build_create_thread(
    authority: &Pubkey,
    payer: &Pubkey,
    thread: &Pubkey,
    amount: u64,
    id: ThreadId,
    trigger: Trigger,
) -> Instruction {
    Instruction {
        program_id: PROGRAM_ID,
        accounts: antegen_thread_program::accounts::ThreadCreate {
            authority: *authority,
            payer: *payer,
            thread: *thread,
            nonce_account: None,
            recent_blockhashes: None,
            rent: None,
            system_program: solana_system_interface::program::ID,
            fiber: None,
            fiber_program: None,
        }
        .to_account_metas(None),
        data: antegen_thread_program::instruction::CreateThread {
            amount,
            id,
            trigger,
            paused: None,
            instruction: None,
            priority_fee: None,
        }
        .data(),
    }
}

// ============================================================================
// Fiber Instructions (CPI wrappers on Thread Program)
// ============================================================================

pub fn build_create_fiber(
    authority: &Pubkey,
    thread: &Pubkey,
    fiber: &Pubkey,
    fiber_index: u8,
    instruction: SerializableInstruction,
    priority_fee: u64,
) -> Instruction {
    Instruction {
        program_id: PROGRAM_ID,
        accounts: antegen_thread_program::accounts::FiberCreate {
            authority: *authority,
            thread: *thread,
            fiber: *fiber,
            fiber_program: FIBER_PROGRAM_ID,
            system_program: solana_system_interface::program::ID,
        }
        .to_account_metas(None),
        data: antegen_thread_program::instruction::CreateFiber {
            fiber_index,
            instruction,
            priority_fee,
        }
        .data(),
    }
}

// ============================================================================
// Exec Instructions
// ============================================================================

pub fn build_exec_thread(
    executor: &Pubkey,
    thread: &Pubkey,
    fiber: &Pubkey,
    config: &Pubkey,
    admin: &Pubkey,
    forgo_commission: bool,
    fiber_cursor: u8,
    remaining_accounts: &[AccountMeta],
) -> Instruction {
    let mut accounts = antegen_thread_program::accounts::ThreadExec {
        executor: *executor,
        thread: *thread,
        fiber: *fiber,
        config: *config,
        admin: *admin,
        nonce_account: None,
        recent_blockhashes: None,
        system_program: solana_system_interface::program::ID,
    }
    .to_account_metas(None);

    // Remaining accounts for inner instruction CPI
    accounts.extend_from_slice(remaining_accounts);

    Instruction {
        program_id: PROGRAM_ID,
        accounts,
        data: antegen_thread_program::instruction::ExecThread {
            forgo_commission,
            fiber_cursor,
        }
        .data(),
    }
}

// ============================================================================
// Convenience helpers
// ============================================================================

/// Make a SerializableInstruction from a regular Instruction.
pub fn make_serializable_instruction(ix: &Instruction) -> SerializableInstruction {
    SerializableInstruction {
        program_id: ix.program_id,
        accounts: ix
            .accounts
            .iter()
            .map(|a| SerializableAccountMeta {
                pubkey: a.pubkey,
                is_signer: a.is_signer,
                is_writable: a.is_writable,
            })
            .collect(),
        data: ix.data.clone(),
    }
}

/// Build a memo instruction suitable for use as a fiber's inner instruction.
pub fn make_memo_instruction(memo: &str, signal: Option<Signal>) -> Instruction {
    let payer_pubkey = solana_sdk::pubkey!("AntegenPayer1111111111111111111111111111111");
    Instruction {
        program_id: PROGRAM_ID,
        accounts: antegen_thread_program::accounts::ThreadMemo {
            signer: payer_pubkey,
        }
        .to_account_metas(None),
        data: antegen_thread_program::instruction::ThreadMemo {
            memo: memo.to_string(),
            signal,
        }
        .data(),
    }
}
