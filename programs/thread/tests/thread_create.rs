use borsh::BorshDeserialize;
use solana_sdk::{pubkey::Pubkey, signature::{Keypair, Signer}, transaction::Transaction};

mod common;
use common::*;

/// Helper to create a thread and return (thread_pubkey, bump).
fn create_thread_helper(
    svm: &mut litesvm::LiteSVM,
    authority: &Keypair,
    payer: &Keypair,
    id: &str,
    trigger: Trigger,
    amount: u64,
) -> (Pubkey, u8) {
    let thread_id = ThreadId::Bytes(id.as_bytes().to_vec());
    let (thread_pubkey, bump) = thread_pda(&authority.pubkey(), id.as_bytes());
    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        amount,
        thread_id,
        trigger,
        None,
        None,
        None,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[payer, authority],
        blockhash,
    );
    svm.send_transaction(tx).expect("create_thread should succeed");
    (thread_pubkey, bump)
}

#[test]
fn test_create_thread_immediate_trigger() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let clock = get_clock(&svm);
    let (thread_pubkey, _) = create_thread_helper(
        &mut svm,
        &authority,
        &payer,
        "imm-test",
        Trigger::Immediate { jitter: 0 },
        100_000,
    );

    let thread = deserialize_thread(&svm, &thread_pubkey);
    match thread.schedule {
        antegen_thread_program::state::Schedule::Timed { prev, next } => {
            assert!(prev >= clock.unix_timestamp);
            assert!(next >= clock.unix_timestamp);
        }
        _ => panic!("Expected Timed schedule for Immediate trigger"),
    }
}

#[test]
fn test_create_thread_timestamp_trigger() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let target_ts = 1700000000i64;
    let (thread_pubkey, _) = create_thread_helper(
        &mut svm,
        &authority,
        &payer,
        "ts-test",
        Trigger::Timestamp {
            unix_ts: target_ts,
            jitter: 0,
        },
        100_000,
    );

    let thread = deserialize_thread(&svm, &thread_pubkey);
    match thread.schedule {
        antegen_thread_program::state::Schedule::Timed { next, .. } => {
            assert_eq!(next, target_ts);
        }
        _ => panic!("Expected Timed schedule"),
    }
}

#[test]
fn test_create_thread_interval_trigger() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let clock = get_clock(&svm);
    let (thread_pubkey, _) = create_thread_helper(
        &mut svm,
        &authority,
        &payer,
        "int-test",
        Trigger::Interval {
            seconds: 60,
            skippable: false,
            jitter: 0,
        },
        100_000,
    );

    let thread = deserialize_thread(&svm, &thread_pubkey);
    match thread.schedule {
        antegen_thread_program::state::Schedule::Timed { next, .. } => {
            assert!(next >= clock.unix_timestamp + 60);
        }
        _ => panic!("Expected Timed schedule"),
    }
}

#[test]
fn test_create_thread_cron_trigger() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let clock = get_clock(&svm);
    let (thread_pubkey, _) = create_thread_helper(
        &mut svm,
        &authority,
        &payer,
        "cron-test",
        Trigger::Cron {
            schedule: "0 * * * * * *".to_string(),
            skippable: false,
            jitter: 0,
        },
        100_000,
    );

    let thread = deserialize_thread(&svm, &thread_pubkey);
    match thread.schedule {
        antegen_thread_program::state::Schedule::Timed { next, .. } => {
            assert!(next > clock.unix_timestamp);
        }
        _ => panic!("Expected Timed schedule"),
    }
}

#[test]
fn test_create_thread_slot_trigger() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let target_slot = 500u64;
    let (thread_pubkey, _) = create_thread_helper(
        &mut svm,
        &authority,
        &payer,
        "slot-test",
        Trigger::Slot { slot: target_slot },
        100_000,
    );

    let thread = deserialize_thread(&svm, &thread_pubkey);
    match thread.schedule {
        antegen_thread_program::state::Schedule::Block { next, .. } => {
            assert_eq!(next, target_slot);
        }
        _ => panic!("Expected Block schedule"),
    }
}

#[test]
fn test_create_thread_epoch_trigger() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let target_epoch = 10u64;
    let (thread_pubkey, _) = create_thread_helper(
        &mut svm,
        &authority,
        &payer,
        "epoch-test",
        Trigger::Epoch {
            epoch: target_epoch,
        },
        100_000,
    );

    let thread = deserialize_thread(&svm, &thread_pubkey);
    match thread.schedule {
        antegen_thread_program::state::Schedule::Block { next, .. } => {
            assert_eq!(next, target_epoch);
        }
        _ => panic!("Expected Block schedule"),
    }
}

#[test]
fn test_create_thread_account_trigger() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let monitored = Pubkey::new_unique();
    let (thread_pubkey, _) = create_thread_helper(
        &mut svm,
        &authority,
        &payer,
        "acct-test",
        Trigger::Account {
            address: monitored,
            offset: 0,
            size: 32,
        },
        100_000,
    );

    let thread = deserialize_thread(&svm, &thread_pubkey);
    match thread.schedule {
        antegen_thread_program::state::Schedule::OnChange { prev } => {
            assert_eq!(prev, 0);
        }
        _ => panic!("Expected OnChange schedule"),
    }
}

#[test]
fn test_create_thread_no_fibers() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (thread_pubkey, _) = create_thread_helper(
        &mut svm,
        &authority,
        &payer,
        "no-ix",
        Trigger::Immediate { jitter: 0 },
        100_000,
    );

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert!(thread.fiber_ids.is_empty());
    assert_eq!(thread.fiber_next_id, 0);
}

#[test]
fn test_create_thread_sol_transfer() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let payer_before = get_balance(&svm, &payer.pubkey());
    let amount = 1_000_000u64;

    let (thread_pubkey, _) = create_thread_helper(
        &mut svm,
        &authority,
        &payer,
        "sol-test",
        Trigger::Immediate { jitter: 0 },
        amount,
    );

    let payer_after = get_balance(&svm, &payer.pubkey());
    let thread_balance = get_balance(&svm, &thread_pubkey);
    // Payer spent at least `amount` (plus rent + tx fees)
    assert!(payer_before - payer_after >= amount);
    // Thread has at least the funded amount (plus rent from init)
    assert!(thread_balance >= amount);
}

#[test]
fn test_create_thread_id_bytes() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let id = "my-thread-id";
    let (thread_pubkey, _) = create_thread_helper(
        &mut svm,
        &authority,
        &payer,
        id,
        Trigger::Immediate { jitter: 0 },
        100_000,
    );

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.id, id.as_bytes());
}

#[test]
fn test_create_thread_id_pubkey() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let id_pubkey = Pubkey::new_unique();
    let thread_id = ThreadId::Pubkey(id_pubkey);
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), id_pubkey.as_ref());

    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        100_000,
        thread_id,
        Trigger::Immediate { jitter: 0 },
        None,
        None,
        None,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.id.len(), 32);
}

#[test]
fn test_create_thread_pda_derivation() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let id = "pda-test";
    let (thread_pubkey, _) = create_thread_helper(
        &mut svm,
        &authority,
        &payer,
        id,
        Trigger::Immediate { jitter: 0 },
        100_000,
    );

    let expected = antegen_thread_program::state::Thread::pubkey(authority.pubkey(), id.as_bytes());
    assert_eq!(thread_pubkey, expected);
}

#[test]
fn test_create_thread_initial_state() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (thread_pubkey, _) = create_thread_helper(
        &mut svm,
        &authority,
        &payer,
        "state-test",
        Trigger::Immediate { jitter: 0 },
        100_000,
    );

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.version, 1);
    assert!(!thread.paused);
    assert_eq!(thread.exec_count, 0);
    assert_eq!(thread.fiber_cursor, 0);
    assert_eq!(
        thread.fiber_signal,
        antegen_thread_program::state::Signal::None
    );
    assert_eq!(thread.last_executor, Pubkey::default());
}

#[test]
fn test_create_thread_without_nonce() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (thread_pubkey, _) = create_thread_helper(
        &mut svm,
        &authority,
        &payer,
        "no-nonce",
        Trigger::Immediate { jitter: 0 },
        100_000,
    );

    let thread = deserialize_thread(&svm, &thread_pubkey);
    // Sentinel value: program ID means no nonce
    assert_eq!(thread.nonce_account, PROGRAM_ID);
}

#[test]
fn test_create_thread_close_fiber_precompiled() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (thread_pubkey, _) = create_thread_helper(
        &mut svm,
        &authority,
        &payer,
        "close-fiber",
        Trigger::Immediate { jitter: 0 },
        100_000,
    );

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert!(!thread.close_fiber.is_empty());
    // Should be deserializable as CompiledInstructionV0
    let compiled =
        antegen_thread_program::state::CompiledInstructionV0::try_from_slice(&thread.close_fiber);
    assert!(compiled.is_ok());
}

#[test]
fn test_create_thread_interval_with_jitter() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let clock = get_clock(&svm);
    let (thread_pubkey, _) = create_thread_helper(
        &mut svm,
        &authority,
        &payer,
        "jitter-test",
        Trigger::Interval {
            seconds: 60,
            skippable: false,
            jitter: 30,
        },
        100_000,
    );

    let thread = deserialize_thread(&svm, &thread_pubkey);
    match thread.schedule {
        antegen_thread_program::state::Schedule::Timed { next, .. } => {
            // next = now + 60 + jitter_offset (0..30)
            assert!(next >= clock.unix_timestamp + 60);
            assert!(next <= clock.unix_timestamp + 60 + 30);
        }
        _ => panic!("Expected Timed schedule"),
    }
}

#[test]
fn test_create_thread_duplicate_id() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    // First create
    create_thread_helper(
        &mut svm,
        &authority,
        &payer,
        "dup-test",
        Trigger::Immediate { jitter: 0 },
        100_000,
    );

    // Second create with same id - should fail
    let thread_id = ThreadId::Bytes(b"dup-test".to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), b"dup-test");
    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        100_000,
        thread_id,
        Trigger::Immediate { jitter: 0 },
        None,
        None,
        None,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    let result = svm.send_transaction(tx);
    assert!(result.is_err(), "Duplicate thread ID should fail");
}

#[test]
fn test_create_thread_with_fiber() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let id = "with-fiber";
    let thread_id = ThreadId::Bytes(id.as_bytes().to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), id.as_bytes());
    let (fiber_pubkey, _) = fiber_pda(&thread_pubkey, 0);

    // Build a memo instruction to use as the fiber's inner instruction
    let memo_ix = make_memo_instruction("hello-fiber", None);
    let ser_ix = make_serializable_instruction(&memo_ix);

    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        10_000_000, // enough to fund 1 fiber
        thread_id,
        Trigger::Immediate { jitter: 0 },
        Some(ser_ix),
        Some(100),
        Some(fiber_pubkey),
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).expect("create_thread with fiber should succeed");

    // Verify thread state
    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.fiber_ids, vec![0]);
    assert_eq!(thread.fiber_next_id, 1);
    assert_eq!(thread.fiber_cursor, 0);

    // Verify fiber PDA exists and has correct data
    let fiber = deserialize_fiber(&svm, &fiber_pubkey);
    assert_eq!(fiber.thread, thread_pubkey);
    assert_eq!(fiber.priority_fee, 100);
    assert!(!fiber.compiled_instruction.is_empty());
}

#[test]
fn test_create_thread_with_fiber_no_accounts_fails() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let id = "fiber-noact";
    let thread_id = ThreadId::Bytes(id.as_bytes().to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), id.as_bytes());

    // Build a memo instruction but don't provide fiber/fiber_program accounts
    let memo_ix = make_memo_instruction("should-fail", None);
    let ser_ix = make_serializable_instruction(&memo_ix);

    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        1_000_000,
        thread_id,
        Trigger::Immediate { jitter: 0 },
        Some(ser_ix),
        None,
        None, // No fiber account
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    let result = svm.send_transaction(tx);
    assert!(result.is_err(), "Should fail when instruction provided but fiber accounts missing");
}
