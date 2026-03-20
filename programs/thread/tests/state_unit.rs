use antegen_thread_program::{
    constants::*,
    state::{
        compile_instruction, decompile_instruction, CommissionCalculator,
        FiberState, PaymentProcessor, Schedule, Signal, Thread, ThreadConfig, Trigger,
        CURRENT_THREAD_VERSION, SEED_THREAD_FIBER,
    },
    utils::{calculate_jitter_offset, next_timestamp},
};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

mod common;
use common::setup::{FIBER_PROGRAM_ID, PROGRAM_ID};

// ============================================================================
// Thread::advance_to_next_fiber tests
// ============================================================================

fn make_thread(fiber_ids: Vec<u8>, fiber_cursor: u8) -> Thread {
    Thread {
        version: CURRENT_THREAD_VERSION,
        bump: 0,
        authority: Pubkey::new_unique(),
        id: b"test".to_vec(),
        name: "test".to_string(),
        created_at: 0,
        trigger: Trigger::Immediate { jitter: 0 },
        schedule: Schedule::Timed { prev: 0, next: 0 },
        fiber_ids,
        fiber_cursor,
        fiber_next_id: 0,
        fiber_signal: Signal::None,
        paused: false,
        exec_count: 0,
        last_executor: Pubkey::default(),
        nonce_account: PROGRAM_ID, // sentinel for no nonce
        last_nonce: String::new(),
        close_fiber: Vec::new(),
    }
}

#[test]
fn test_advance_to_next_fiber_sequential() {
    let mut thread = make_thread(vec![0, 1, 2], 0);
    thread.advance_to_next_fiber();
    assert_eq!(thread.fiber_cursor, 1);
    thread.advance_to_next_fiber();
    assert_eq!(thread.fiber_cursor, 2);
}

#[test]
fn test_advance_to_next_fiber_wrap() {
    let mut thread = make_thread(vec![0, 1, 2], 2);
    thread.advance_to_next_fiber();
    assert_eq!(thread.fiber_cursor, 0);
}

#[test]
fn test_advance_to_next_fiber_empty() {
    let mut thread = make_thread(vec![], 0);
    thread.advance_to_next_fiber();
    assert_eq!(thread.fiber_cursor, 0);
}

#[test]
fn test_advance_to_next_fiber_single() {
    let mut thread = make_thread(vec![0], 0);
    thread.advance_to_next_fiber();
    assert_eq!(thread.fiber_cursor, 0); // wraps to itself
}

#[test]
fn test_advance_to_next_fiber_cursor_not_found() {
    let mut thread = make_thread(vec![0, 2, 4], 3); // 3 not in list
    thread.advance_to_next_fiber();
    assert_eq!(thread.fiber_cursor, 0); // resets to first
}

// ============================================================================
// Thread::has_nonce_account tests
// ============================================================================

#[test]
fn test_has_nonce_account_true() {
    let mut thread = make_thread(vec![], 0);
    thread.nonce_account = Pubkey::new_unique(); // real nonce
    assert!(thread.has_nonce_account());
}

#[test]
fn test_has_nonce_account_false_system_program() {
    let mut thread = make_thread(vec![], 0);
    thread.nonce_account = solana_system_interface::program::ID;
    assert!(!thread.has_nonce_account());
}

#[test]
fn test_has_nonce_account_false_program_id() {
    let mut thread = make_thread(vec![], 0);
    thread.nonce_account = PROGRAM_ID; // sentinel
    assert!(!thread.has_nonce_account());
}

// ============================================================================
// Thread::validate_for_execution tests
// ============================================================================

#[test]
fn test_validate_no_fibers() {
    let thread = make_thread(vec![], 0);
    assert!(thread.validate_for_execution().is_err());
}

#[test]
fn test_validate_invalid_cursor() {
    let thread = make_thread(vec![0, 1], 5); // 5 not in list
    assert!(thread.validate_for_execution().is_err());
}

#[test]
fn test_validate_valid_cursor() {
    let thread = make_thread(vec![0], 0);
    assert!(thread.validate_for_execution().is_ok());
}

// ============================================================================
// Thread::is_ready tests
// ============================================================================

#[test]
fn test_is_ready_timed_past() {
    let mut thread = make_thread(vec![0], 0);
    thread.schedule = Schedule::Timed {
        prev: 100,
        next: 200,
    };
    assert!(thread.is_ready(0, 200)); // current >= next
    assert!(thread.is_ready(0, 300)); // well past
}

#[test]
fn test_is_ready_timed_future() {
    let mut thread = make_thread(vec![0], 0);
    thread.schedule = Schedule::Timed {
        prev: 100,
        next: 200,
    };
    assert!(!thread.is_ready(0, 100)); // before next
}

#[test]
fn test_is_ready_block_slot() {
    let mut thread = make_thread(vec![0], 0);
    thread.trigger = Trigger::Slot { slot: 100 };
    thread.schedule = Schedule::Block {
        prev: 50,
        next: 100,
    };
    assert!(thread.is_ready(100, 0)); // slot reached
    assert!(!thread.is_ready(99, 0)); // not yet
}

// ============================================================================
// compile_instruction / decompile_instruction roundtrip tests
// ============================================================================

#[test]
fn test_compile_decompile_roundtrip() {
    let program_id = Pubkey::new_unique();
    let account1 = Pubkey::new_unique();
    let account2 = Pubkey::new_unique();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(account1, true),
            AccountMeta::new_readonly(account2, false),
        ],
        data: vec![1, 2, 3, 4],
    };

    let compiled = compile_instruction(ix.clone()).unwrap();
    let decompiled = decompile_instruction(&compiled).unwrap();

    assert_eq!(decompiled.program_id, ix.program_id);
    assert_eq!(decompiled.data, ix.data);
    assert_eq!(decompiled.accounts.len(), ix.accounts.len());
}

#[test]
fn test_compiled_account_sorting() {
    // Accounts should be sorted: rw_signers, ro_signers, rw, ro
    let program_id = Pubkey::new_unique();
    let ro_account = Pubkey::new_unique();
    let rw_signer = Pubkey::new_unique();
    let rw_account = Pubkey::new_unique();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(ro_account, false),
            AccountMeta::new(rw_signer, true),
            AccountMeta::new(rw_account, false),
        ],
        data: vec![],
    };

    let compiled = compile_instruction(ix).unwrap();

    // rw_signer should come first in the sorted account list
    assert_eq!(compiled.num_rw_signers, 1);
    assert_eq!(compiled.num_ro_signers, 0);
    assert_eq!(compiled.num_rw, 1); // rw_account (non-signer)
}

// ============================================================================
// CommissionCalculator tests
// ============================================================================

fn make_config() -> ThreadConfig {
    ThreadConfig {
        version: 1,
        bump: 0,
        admin: Pubkey::new_unique(),
        paused: false,
        commission_fee: 1000,
        executor_fee_bps: 9000,
        core_team_bps: 1000,
        grace_period_seconds: 5,
        fee_decay_seconds: 295,
    }
}

#[test]
fn test_commission_within_grace() {
    let config = make_config();
    let multiplier = config.calculate_commission_multiplier(3); // within 5s grace
    assert_eq!(multiplier, 1.0);
}

#[test]
fn test_commission_during_decay() {
    let config = make_config();
    // Halfway through decay: 5s grace + 147.5s into 295s decay
    let time = 5 + 147; // 152s total
    let multiplier = config.calculate_commission_multiplier(time);
    assert!(multiplier < 1.0 && multiplier > 0.0);
}

#[test]
fn test_commission_expired() {
    let config = make_config();
    // Past grace + decay: 5 + 295 = 300
    let multiplier = config.calculate_commission_multiplier(301);
    assert_eq!(multiplier, 0.0);
}

// ============================================================================
// PaymentProcessor tests
// ============================================================================

#[test]
fn test_payment_normal() {
    let config = make_config();
    let payments = config.calculate_payments(0, -5000, false);
    // Within grace, full commission
    assert_eq!(payments.fee_payer_reimbursement, 5000); // abs(-5000)
    assert!(payments.executor_commission > 0);
    assert!(payments.core_team_fee > 0);
}

#[test]
fn test_payment_forgo_commission() {
    let config = make_config();
    let payments = config.calculate_payments(0, -5000, true); // forgo
    assert_eq!(payments.executor_commission, 0);
    assert!(payments.core_team_fee > 0); // core team always gets paid
}

#[test]
fn test_payment_no_payment_positive_balance() {
    let config = make_config();
    // Positive balance change means inner instruction already paid
    let payments = config.calculate_payments(0, 5000, false);
    assert_eq!(payments.fee_payer_reimbursement, 0);
    assert_eq!(payments.executor_commission, 0);
    // Core team fee still calculated
    assert!(payments.core_team_fee > 0);
}

// ============================================================================
// calculate_jitter_offset tests
// ============================================================================

#[test]
fn test_jitter_zero() {
    let pubkey = Pubkey::new_unique();
    assert_eq!(calculate_jitter_offset(100, &pubkey, 0), 0);
}

#[test]
fn test_jitter_deterministic() {
    let pubkey = Pubkey::new_unique();
    let j1 = calculate_jitter_offset(100, &pubkey, 60);
    let j2 = calculate_jitter_offset(100, &pubkey, 60);
    assert_eq!(j1, j2); // same inputs -> same output
}

#[test]
fn test_jitter_bounded() {
    let pubkey = Pubkey::new_unique();
    let jitter = 60u64;
    let offset = calculate_jitter_offset(12345, &pubkey, jitter);
    assert!(offset >= 0);
    assert!((offset as u64) < jitter);
}

// ============================================================================
// next_timestamp cron test
// ============================================================================

#[test]
fn test_next_timestamp_cron() {
    let after = 1700000000i64; // some unix timestamp
    let schedule = "0 * * * * * *".to_string(); // every minute
    let result = next_timestamp(after, schedule);
    assert!(result.is_some());
    let next = result.unwrap();
    assert!(next > after);
}

// ============================================================================
// PDA derivation tests
// ============================================================================

#[test]
fn test_thread_pda_derivation() {
    let authority = Pubkey::new_unique();
    let id = b"test-thread";
    let (expected, _bump) =
        Pubkey::find_program_address(&[SEED_THREAD, authority.as_ref(), id], &PROGRAM_ID);
    let derived = Thread::pubkey(authority, id);
    assert_eq!(derived, expected);
}

#[test]
fn test_fiber_pda_derivation() {
    let thread = Pubkey::new_unique();
    let index = 3u8;
    // FiberState PDAs use the Fiber Program ID
    let (expected, _bump) =
        Pubkey::find_program_address(&[SEED_THREAD_FIBER, thread.as_ref(), &[index]], &FIBER_PROGRAM_ID);
    let derived = FiberState::pubkey(thread, index);
    assert_eq!(derived, expected);
}
