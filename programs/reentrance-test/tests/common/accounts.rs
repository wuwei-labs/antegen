use anchor_lang::AccountDeserialize;
use antegen_thread_program::fiber::{Fiber, FiberVersionedState};
use litesvm::LiteSVM;
use solana_sdk::pubkey::Pubkey;

use super::setup::{FIBER_PROGRAM_ID, PROGRAM_ID};

// PDA seeds (must match program constants)
const SEED_CONFIG: &[u8] = b"thread_config";
const SEED_THREAD: &[u8] = b"thread";
const SEED_THREAD_FIBER: &[u8] = b"thread_fiber";

/// Derive the config PDA.
pub fn config_pda() -> (Pubkey, u8) {
    Pubkey::find_program_address(&[SEED_CONFIG], &PROGRAM_ID)
}

/// Derive a thread PDA.
pub fn thread_pda(authority: &Pubkey, id: &[u8]) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[SEED_THREAD, authority.as_ref(), id], &PROGRAM_ID)
}

/// Derive a fiber PDA (owned by Fiber Program).
pub fn fiber_pda(thread: &Pubkey, index: u8) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[SEED_THREAD_FIBER, thread.as_ref(), &[index]],
        &FIBER_PROGRAM_ID,
    )
}

/// Deserialize a Thread account from the SVM.
pub fn deserialize_thread(svm: &LiteSVM, pubkey: &Pubkey) -> antegen_thread_program::state::Thread {
    let account = svm.get_account(pubkey).expect("Thread account not found");
    antegen_thread_program::state::Thread::try_deserialize(&mut account.data.as_slice())
        .expect("Failed to deserialize Thread")
}

/// Deserialize a fiber account, accepting either legacy or V1 shape and
/// projecting to V1 fields (legacy → version=0, lookup_tables=[]).
pub fn deserialize_fiber(
    svm: &LiteSVM,
    pubkey: &Pubkey,
) -> FiberVersionedState {
    let account = svm.get_account(pubkey).expect("Fiber account not found");
    let read = Fiber::try_deserialize(&mut account.data.as_slice())
        .expect("Failed to deserialize fiber account");
    match read {
        Fiber::Legacy(s) => FiberVersionedState {
            version: 0,
            thread: s.thread,
            compiled_instruction: s.compiled_instruction,
            last_executed: s.last_executed,
            exec_count: s.exec_count,
            priority_fee: s.priority_fee,
            lookup_tables: Vec::new(),
        },
        Fiber::V1(s) => s,
    }
}

/// Check if an account exists and has non-zero data.
pub fn account_exists(svm: &LiteSVM, pubkey: &Pubkey) -> bool {
    svm.get_account(pubkey)
        .map(|a| !a.data.is_empty() && a.lamports > 0)
        .unwrap_or(false)
}
