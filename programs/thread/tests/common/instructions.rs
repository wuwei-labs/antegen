use anchor_lang::{InstructionData, ToAccountMetas};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use super::setup::{FIBER_PROGRAM_ID, PROGRAM_ID};

// Re-export program types used by tests
pub use antegen_thread_program::instructions::config_update::ConfigUpdateParams;
pub use antegen_thread_program::instructions::thread_update::ThreadUpdateParams;
use antegen_thread_program::state::{SerializableAccountMeta, SerializableInstruction};
pub use antegen_thread_program::state::{Signal, Trigger};
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

pub fn build_update_config(
    admin: &Pubkey,
    config: &Pubkey,
    params: ConfigUpdateParams,
) -> Instruction {
    Instruction {
        program_id: PROGRAM_ID,
        accounts: antegen_thread_program::accounts::ConfigUpdate {
            admin: *admin,
            config: *config,
        }
        .to_account_metas(None),
        data: antegen_thread_program::instruction::UpdateConfig { params }.data(),
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
    instruction: Option<SerializableInstruction>,
    priority_fee: Option<u64>,
    fiber: Option<Pubkey>,
) -> Instruction {
    let fiber_program = fiber.map(|_| FIBER_PROGRAM_ID);

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
            fiber,
            fiber_program,
        }
        .to_account_metas(None),
        data: antegen_thread_program::instruction::CreateThread {
            amount,
            id,
            trigger,
            paused: None,
            instruction,
            priority_fee,
        }
        .data(),
    }
}

pub fn build_update_thread(
    authority: &Pubkey,
    thread: &Pubkey,
    params: ThreadUpdateParams,
) -> Instruction {
    Instruction {
        program_id: PROGRAM_ID,
        accounts: antegen_thread_program::accounts::ThreadUpdate {
            authority: *authority,
            thread: *thread,
        }
        .to_account_metas(None),
        data: antegen_thread_program::instruction::UpdateThread { params }.data(),
    }
}

pub fn build_withdraw_thread(
    authority: &Pubkey,
    pay_to: &Pubkey,
    thread: &Pubkey,
    amount: u64,
) -> Instruction {
    Instruction {
        program_id: PROGRAM_ID,
        accounts: antegen_thread_program::accounts::ThreadWithdraw {
            authority: *authority,
            pay_to: *pay_to,
            thread: *thread,
        }
        .to_account_metas(None),
        data: antegen_thread_program::instruction::WithdrawThread { amount }.data(),
    }
}

pub fn build_close_thread(
    authority: &Pubkey,
    close_to: &Pubkey,
    thread: &Pubkey,
    fiber_accounts: &[Pubkey],
) -> Instruction {
    let fiber_program = if fiber_accounts.is_empty() {
        None
    } else {
        Some(FIBER_PROGRAM_ID)
    };

    let mut accounts = antegen_thread_program::accounts::ThreadClose {
        authority: *authority,
        close_to: *close_to,
        thread: *thread,
        fiber_program,
    }
    .to_account_metas(None);

    // Add fiber accounts as remaining_accounts (writable, not signer)
    for fiber in fiber_accounts {
        accounts.push(AccountMeta::new(*fiber, false));
    }

    Instruction {
        program_id: PROGRAM_ID,
        accounts,
        data: antegen_thread_program::instruction::CloseThread.data(),
    }
}

pub fn build_delete_thread(admin: &Pubkey, config: &Pubkey, thread: &Pubkey) -> Instruction {
    Instruction {
        program_id: PROGRAM_ID,
        accounts: antegen_thread_program::accounts::ThreadDelete {
            admin: *admin,
            config: *config,
            thread: *thread,
        }
        .to_account_metas(None),
        data: antegen_thread_program::instruction::DeleteThread.data(),
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

pub fn build_close_fiber(
    authority: &Pubkey,
    thread: &Pubkey,
    fiber: &Pubkey,
    fiber_index: u8,
) -> Instruction {
    Instruction {
        program_id: PROGRAM_ID,
        accounts: antegen_thread_program::accounts::FiberClose {
            authority: *authority,
            thread: *thread,
            fiber: *fiber,
            fiber_program: FIBER_PROGRAM_ID,
        }
        .to_account_metas(None),
        data: antegen_thread_program::instruction::CloseFiber { fiber_index }.data(),
    }
}

pub fn build_update_fiber(
    authority: &Pubkey,
    thread: &Pubkey,
    fiber: &Pubkey,
    instruction: SerializableInstruction,
    priority_fee: Option<u64>,
) -> Instruction {
    Instruction {
        program_id: PROGRAM_ID,
        accounts: antegen_thread_program::accounts::FiberUpdate {
            authority: *authority,
            thread: *thread,
            fiber: *fiber,
            fiber_program: FIBER_PROGRAM_ID,
        }
        .to_account_metas(None),
        data: antegen_thread_program::instruction::UpdateFiber {
            instruction,
            priority_fee,
        }
        .data(),
    }
}

pub fn build_swap_fiber(
    authority: &Pubkey,
    thread: &Pubkey,
    target: &Pubkey,
    source: &Pubkey,
    source_fiber_index: u8,
) -> Instruction {
    Instruction {
        program_id: PROGRAM_ID,
        accounts: antegen_thread_program::accounts::FiberSwap {
            authority: *authority,
            thread: *thread,
            target: *target,
            source: *source,
            fiber_program: FIBER_PROGRAM_ID,
        }
        .to_account_metas(None),
        data: antegen_thread_program::instruction::SwapFiber { source_fiber_index }.data(),
    }
}

// ============================================================================
// Exec / Error / Memo Instructions
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

pub fn build_error_thread(
    executor: &Pubkey,
    thread: &Pubkey,
    config: &Pubkey,
    admin: &Pubkey,
    error_code: u32,
    error_message: &str,
    remaining_accounts: &[AccountMeta],
) -> Instruction {
    let mut accounts = antegen_thread_program::accounts::ThreadError {
        executor: *executor,
        thread: *thread,
        config: *config,
        admin: *admin,
        system_program: solana_system_interface::program::ID,
    }
    .to_account_metas(None);

    accounts.extend_from_slice(remaining_accounts);

    Instruction {
        program_id: PROGRAM_ID,
        accounts,
        data: antegen_thread_program::instruction::ErrorThread {
            error_code,
            error_message: error_message.to_string(),
        }
        .data(),
    }
}

pub fn build_thread_memo(signer: &Pubkey, memo: &str, signal: Option<Signal>) -> Instruction {
    Instruction {
        program_id: PROGRAM_ID,
        accounts: antegen_thread_program::accounts::ThreadMemo { signer: *signer }
            .to_account_metas(None),
        data: antegen_thread_program::instruction::ThreadMemo {
            memo: memo.to_string(),
            signal,
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
/// Uses PAYER_PUBKEY as placeholder signer that gets replaced during execution.
pub fn make_memo_instruction(memo: &str, signal: Option<Signal>) -> Instruction {
    let payer_pubkey = solana_sdk::pubkey!("AntegenPayer1111111111111111111111111111111");
    build_thread_memo(&payer_pubkey, memo, signal)
}
