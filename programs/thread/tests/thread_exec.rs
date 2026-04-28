use solana_sdk::{
    instruction::AccountMeta,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};

mod common;
use common::*;

/// Create a thread with an external fiber (memo instruction at index 0).
fn setup_exec_thread(
    svm: &mut litesvm::LiteSVM,
    authority: &Keypair,
    payer: &Keypair,
    _admin: &Pubkey,
    id: &str,
    trigger: Trigger,
    memo: &str,
    signal: Option<Signal>,
) -> (Pubkey, Pubkey) {
    let thread_id = ThreadId::Bytes(id.as_bytes().to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), id.as_bytes());

    // Create the thread (no fibers)
    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        10_000_000, // enough to pay fees
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
    svm.send_transaction(tx)
        .expect("create_thread should succeed");

    // Create fiber at index 0 with memo instruction
    let (fiber_pubkey, _) = fiber_pda(&thread_pubkey, 0);
    let memo_ix = make_memo_instruction(memo, signal);
    let serializable = make_serializable_instruction(&memo_ix);
    let ix = build_create_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
        0,
        serializable,
        0,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[payer, authority],
        blockhash,
    );
    svm.send_transaction(tx)
        .expect("create_fiber should succeed");

    (thread_pubkey, fiber_pubkey)
}

/// Build remaining accounts for exec based on the thread's compiled instruction.
/// For a memo instruction with the payer placeholder as signer, remaining accounts = [program_id, executor].
fn build_remaining_accounts(executor: &Pubkey) -> Vec<AccountMeta> {
    vec![
        AccountMeta::new_readonly(PROGRAM_ID, false), // program account for CPI
        AccountMeta::new_readonly(*executor, false),   // executor replaces PAYER_PUBKEY
    ]
}

#[test]
fn test_exec_thread_immediate_trigger() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (config_pubkey, _) = config_pda();
    let (thread_pubkey, fiber_pubkey) = setup_exec_thread(
        &mut svm,
        &authority,
        &payer,
        &admin.pubkey(),
        "exec-imm",
        Trigger::Immediate { jitter: 0 },
        "exec-test",
        None,
    );

    let remaining = build_remaining_accounts(&executor.pubkey());
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        false,
        0,
        &remaining,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[&executor],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.exec_count, 1);
    assert_eq!(thread.last_executor, executor.pubkey());
}

#[test]
fn test_exec_thread_paused_fails() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (config_pubkey, _) = config_pda();
    let (thread_pubkey, fiber_pubkey) = setup_exec_thread(
        &mut svm,
        &authority,
        &payer,
        &admin.pubkey(),
        "exec-paused",
        Trigger::Immediate { jitter: 0 },
        "test",
        None,
    );

    // Pause the thread
    let update_ix = build_update_thread(
        &authority.pubkey(),
        &thread_pubkey,
        ThreadUpdateParams {
            paused: Some(true),
            ..Default::default()
        },
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[update_ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    // Try to exec
    let remaining = build_remaining_accounts(&executor.pubkey());
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        false,
        0,
        &remaining,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[&executor],
        blockhash,
    );
    let result = svm.send_transaction(tx);
    assert!(result.is_err());
}

#[test]
fn test_exec_thread_global_pause_fails() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (config_pubkey, _) = config_pda();
    let (thread_pubkey, fiber_pubkey) = setup_exec_thread(
        &mut svm,
        &authority,
        &payer,
        &admin.pubkey(),
        "exec-gpause",
        Trigger::Immediate { jitter: 0 },
        "test",
        None,
    );

    // Global pause
    let update_ix = build_update_config(
        &admin.pubkey(),
        &config_pubkey,
        ConfigUpdateParams {
            paused: Some(true),
            ..Default::default()
        },
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[update_ix],
        Some(&admin.pubkey()),
        &[&admin],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    // Try to exec
    let remaining = build_remaining_accounts(&executor.pubkey());
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        false,
        0,
        &remaining,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[&executor],
        blockhash,
    );
    let result = svm.send_transaction(tx);
    assert!(result.is_err());
}

#[test]
fn test_exec_thread_no_fibers_fails() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    // Create thread WITHOUT any fibers
    let thread_id = ThreadId::Bytes(b"exec-nofiber".to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), b"exec-nofiber");
    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        5_000_000,
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

    // Try to exec - should fail because no fibers (constraint check)
    let (config_pubkey, _) = config_pda();
    // Use a dummy fiber pubkey (doesn't exist)
    let (dummy_fiber, _) = fiber_pda(&thread_pubkey, 0);
    let remaining = build_remaining_accounts(&executor.pubkey());
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        &dummy_fiber,
        &config_pubkey,
        &admin.pubkey(),
        false,
        0,
        &remaining,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[&executor],
        blockhash,
    );
    let result = svm.send_transaction(tx);
    assert!(result.is_err());
}

#[test]
fn test_exec_thread_timestamp_not_ready() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let future_ts = get_clock(&svm).unix_timestamp + 3600; // 1 hour from now
    let (config_pubkey, _) = config_pda();
    let (thread_pubkey, fiber_pubkey) = setup_exec_thread(
        &mut svm,
        &authority,
        &payer,
        &admin.pubkey(),
        "exec-notready",
        Trigger::Timestamp {
            unix_ts: future_ts,
            jitter: 0,
        },
        "test",
        None,
    );

    let remaining = build_remaining_accounts(&executor.pubkey());
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        false,
        0,
        &remaining,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[&executor],
        blockhash,
    );
    let result = svm.send_transaction(tx);
    assert!(result.is_err());
}

#[test]
fn test_exec_thread_timestamp_ready() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let clock = get_clock(&svm);
    let target_ts = clock.unix_timestamp + 10;
    let (config_pubkey, _) = config_pda();
    let (thread_pubkey, fiber_pubkey) = setup_exec_thread(
        &mut svm,
        &authority,
        &payer,
        &admin.pubkey(),
        "exec-ts-ready",
        Trigger::Timestamp {
            unix_ts: target_ts,
            jitter: 0,
        },
        "test",
        None,
    );

    // Advance clock past the timestamp
    advance_clock(&mut svm, 20);

    let remaining = build_remaining_accounts(&executor.pubkey());
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        false,
        0,
        &remaining,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[&executor],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.exec_count, 1);
    assert!(thread.paused, "Timestamp thread should auto-pause after firing");
    match thread.schedule {
        antegen_thread_program::state::Schedule::Timed { next, .. } => {
            assert_eq!(next, target_ts, "schedule.next should be the original unix_ts after timestamp fires");
        }
        _ => panic!("Expected Timed schedule"),
    }
}

#[test]
fn test_exec_thread_interval_trigger() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (config_pubkey, _) = config_pda();
    let (thread_pubkey, fiber_pubkey) = setup_exec_thread(
        &mut svm,
        &authority,
        &payer,
        &admin.pubkey(),
        "exec-int",
        Trigger::Interval {
            seconds: 30,
            skippable: false,
            jitter: 0,
        },
        "test",
        None,
    );

    // Advance past interval
    advance_clock(&mut svm, 35);

    let remaining = build_remaining_accounts(&executor.pubkey());
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        false,
        0,
        &remaining,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[&executor],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.exec_count, 1);
}

#[test]
fn test_exec_thread_slot_trigger() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let clock = get_clock(&svm);
    let target_slot = clock.slot + 10;
    let (config_pubkey, _) = config_pda();
    let (thread_pubkey, fiber_pubkey) = setup_exec_thread(
        &mut svm,
        &authority,
        &payer,
        &admin.pubkey(),
        "exec-slot",
        Trigger::Slot { slot: target_slot },
        "test",
        None,
    );

    // Warp to target slot
    warp_to_slot(&mut svm, target_slot + 1);

    let remaining = build_remaining_accounts(&executor.pubkey());
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        false,
        0,
        &remaining,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[&executor],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.exec_count, 1);
}

#[test]
fn test_exec_thread_fee_distribution() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (config_pubkey, _) = config_pda();
    let (thread_pubkey, fiber_pubkey) = setup_exec_thread(
        &mut svm,
        &authority,
        &payer,
        &admin.pubkey(),
        "exec-fee",
        Trigger::Immediate { jitter: 0 },
        "test",
        None,
    );

    let executor_before = get_balance(&svm, &executor.pubkey());
    let admin_before = get_balance(&svm, &admin.pubkey());

    let remaining = build_remaining_accounts(&executor.pubkey());
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        false,
        0,
        &remaining,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[&executor],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    let executor_after = get_balance(&svm, &executor.pubkey());
    let admin_after = get_balance(&svm, &admin.pubkey());

    // Admin should receive core team fee
    assert!(admin_after > admin_before);
    let _ = executor_before;
    let _ = executor_after;
}

#[test]
fn test_exec_thread_forgo_commission() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (config_pubkey, _) = config_pda();
    let (thread_pubkey, fiber_pubkey) = setup_exec_thread(
        &mut svm,
        &authority,
        &payer,
        &admin.pubkey(),
        "exec-forgo",
        Trigger::Immediate { jitter: 0 },
        "test",
        None,
    );

    let remaining = build_remaining_accounts(&executor.pubkey());
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        true, // forgo commission
        0,
        &remaining,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[&executor],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.exec_count, 1);
}

#[test]
fn test_exec_thread_signal_close() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (config_pubkey, _) = config_pda();

    // For Immediate trigger, after first exec the fiber_signal is set to Signal::Close
    let (thread_pubkey, fiber_pubkey) = setup_exec_thread(
        &mut svm,
        &authority,
        &payer,
        &admin.pubkey(),
        "exec-close",
        Trigger::Immediate { jitter: 0 },
        "close-test",
        None,
    );

    // First exec - succeeds, sets fiber_signal to Close (Immediate trigger auto-closes)
    let remaining = build_remaining_accounts(&executor.pubkey());
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        false,
        0,
        &remaining,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[&executor],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    // After exec with Immediate trigger, fiber_signal should be Close
    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(
        thread.fiber_signal,
        antegen_thread_program::state::Signal::Close
    );
}

#[test]
fn test_exec_thread_signal_update_pause() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (config_pubkey, _) = config_pda();
    // Use Interval trigger so thread stays alive after exec
    let (thread_pubkey, fiber_pubkey) = setup_exec_thread(
        &mut svm,
        &authority,
        &payer,
        &admin.pubkey(),
        "exec-sig-pause",
        Trigger::Interval {
            seconds: 10,
            skippable: false,
            jitter: 0,
        },
        "pause-test",
        Some(Signal::Update {
            paused: Some(true),
            trigger: None,
            index: None,
        }),
    );

    // Advance past interval
    advance_clock(&mut svm, 15);

    let remaining = build_remaining_accounts(&executor.pubkey());
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        false,
        0,
        &remaining,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[&executor],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.exec_count, 1);
    assert!(thread.paused, "Thread should be paused after Signal::Update with paused=true");
}

#[test]
fn test_exec_thread_signal_chain() {
    let (mut svm, admin, _payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    let payer = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&payer.pubkey(), DEFAULT_AIRDROP * 2).unwrap();

    let (config_pubkey, _) = config_pda();

    // Create thread with Interval trigger (so it stays alive)
    let thread_id = ThreadId::Bytes(b"exec-chain".to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), b"exec-chain");
    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        100_000_000, // extra for 2 fiber creations + rent
        thread_id,
        Trigger::Interval {
            seconds: 10,
            skippable: false,
            jitter: 0,
        },
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
    svm.send_transaction(tx).expect("create_thread should succeed");

    // Create fiber 0: returns Signal::Chain
    let (fiber0_pubkey, _) = fiber_pda(&thread_pubkey, 0);
    let chain_memo_ix = make_memo_instruction("chain-fiber", Some(Signal::Chain));
    let serializable = make_serializable_instruction(&chain_memo_ix);
    let ix = build_create_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber0_pubkey,
        0,
        serializable,
        0,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).expect("create_fiber_0 should succeed");

    // Create fiber 1: returns Signal::None (default memo)
    let (fiber1_pubkey, _) = fiber_pda(&thread_pubkey, 1);
    let none_memo_ix = make_memo_instruction("chained-fiber", None);
    let serializable = make_serializable_instruction(&none_memo_ix);
    let ix = build_create_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber1_pubkey,
        1,
        serializable,
        0,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).expect("create_fiber_1 should succeed");

    // Advance past interval
    advance_clock(&mut svm, 15);

    // Execute fiber 0 → should return Signal::Chain
    let remaining = build_remaining_accounts(&executor.pubkey());
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        &fiber0_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        false,
        0,
        &remaining,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[&executor],
        blockhash,
    );
    svm.send_transaction(tx).expect("exec fiber 0 should succeed");

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.exec_count, 1);
    assert_eq!(
        thread.fiber_signal,
        Signal::Chain,
        "fiber_signal should be Chain after fiber 0"
    );

    // Execute fiber 1 (chained) → should succeed without trigger validation
    let remaining = build_remaining_accounts(&executor.pubkey());
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        &fiber1_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        false,
        1, // fiber_cursor=1 for chained execution
        &remaining,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[&executor],
        blockhash,
    );
    svm.send_transaction(tx).expect("exec fiber 1 (chained) should succeed");

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.exec_count, 2, "Both fibers should have executed");
    assert_eq!(
        thread.fiber_signal,
        Signal::None,
        "fiber_signal should be None after chain completes"
    );
}

#[test]
fn test_exec_thread_signal_update_trigger() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (config_pubkey, _) = config_pda();
    let new_trigger = Trigger::Timestamp {
        unix_ts: 999_999_999,
        jitter: 0,
    };

    // Use Interval trigger, fiber will signal to change it to Timestamp
    let (thread_pubkey, fiber_pubkey) = setup_exec_thread(
        &mut svm,
        &authority,
        &payer,
        &admin.pubkey(),
        "exec-sig-trig",
        Trigger::Interval {
            seconds: 10,
            skippable: false,
            jitter: 0,
        },
        "trigger-test",
        Some(Signal::Update {
            paused: None,
            trigger: Some(new_trigger.clone()),
            index: None,
        }),
    );

    // Advance past interval
    advance_clock(&mut svm, 15);

    let remaining = build_remaining_accounts(&executor.pubkey());
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        false,
        0,
        &remaining,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[&executor],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.exec_count, 1);
    assert!(!thread.paused, "Thread should not be paused");
    assert_eq!(thread.trigger, new_trigger, "Trigger should be updated to Timestamp");
}

/// Test that exec works when the CPI target program_id also appears as a
/// regular account in the instruction (simulates Anchor's None-placeholder
/// collision where Option<UncheckedAccount>::None emits program_id as account).
#[test]
fn test_exec_with_program_id_in_accounts() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (config_pubkey, _) = config_pda();

    // Build a memo instruction where PROGRAM_ID appears as both the CPI target
    // AND as an extra readonly account (simulating Anchor's None placeholder).
    let payer_pubkey = solana_sdk::pubkey!("AntegenPayer1111111111111111111111111111111");
    let memo_ix = build_thread_memo(&payer_pubkey, "collision-test", None);

    // Add PROGRAM_ID as an extra readonly non-signer account
    let mut modified_accounts = memo_ix.accounts.clone();
    modified_accounts.push(AccountMeta::new_readonly(PROGRAM_ID, false));

    let collision_ix = solana_sdk::instruction::Instruction {
        program_id: memo_ix.program_id, // = PROGRAM_ID
        accounts: modified_accounts,
        data: memo_ix.data.clone(),
    };

    let serializable = make_serializable_instruction(&collision_ix);

    // Create thread + fiber with this collision instruction
    let thread_id = ThreadId::Bytes(b"exec-collision".to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), b"exec-collision");
    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        10_000_000,
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
    svm.send_transaction(tx).expect("create_thread should succeed");

    let (fiber_pubkey, _) = fiber_pda(&thread_pubkey, 0);
    let ix = build_create_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
        0,
        serializable,
        0,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).expect("create_fiber should succeed");

    // Build remaining accounts: PROGRAM_ID (CPI target + collision account) + executor
    let remaining = vec![
        AccountMeta::new_readonly(PROGRAM_ID, false),
        AccountMeta::new_readonly(executor.pubkey(), false),
    ];
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        false,
        0,
        &remaining,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[&executor],
        blockhash,
    );
    svm.send_transaction(tx).expect("exec with program_id-as-account should succeed");

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.exec_count, 1);
}

/// Simulates the srsly rental_close inline-activation flow:
/// Timestamp fires -> fiber 0 chains -> fiber 1 returns Signal::Update { paused: false, trigger: Interval }
/// Thread should NOT be auto-paused because fiber 1 explicitly set paused: false.
#[test]
fn test_exec_timestamp_chain_then_update_unpause() {
    let (mut svm, admin, _payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    let payer = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&payer.pubkey(), DEFAULT_AIRDROP * 2).unwrap();

    let clock = get_clock(&svm);
    let target_ts = clock.unix_timestamp + 10;
    let (config_pubkey, _) = config_pda();

    // Create thread with Timestamp trigger (simulates contract_process switching to Timestamp)
    let thread_id = ThreadId::Bytes(b"ts-chain-unpause".to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), b"ts-chain-unpause");
    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        100_000_000,
        thread_id,
        Trigger::Timestamp {
            unix_ts: target_ts,
            jitter: 0,
        },
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
    svm.send_transaction(tx).expect("create_thread should succeed");

    // Fiber 0: returns Signal::Chain (simulates contract_process seeing expired rental)
    let (fiber0_pubkey, _) = fiber_pda(&thread_pubkey, 0);
    let chain_ix = make_memo_instruction("controller", Some(Signal::Chain));
    let serializable = make_serializable_instruction(&chain_ix);
    let ix = build_create_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber0_pubkey,
        0,
        serializable,
        0,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).expect("create_fiber_0 should succeed");

    // Fiber 1: returns Signal::Update { paused: false, trigger: Interval }
    // (simulates rental_close activating queued rental and restaging with cron/interval trigger)
    let new_trigger = Trigger::Interval {
        seconds: 60,
        skippable: false,
        jitter: 0,
    };
    let (fiber1_pubkey, _) = fiber_pda(&thread_pubkey, 1);
    let update_ix = make_memo_instruction(
        "close-activate",
        Some(Signal::Update {
            paused: Some(false),
            trigger: Some(new_trigger.clone()),
            index: Some(0),
        }),
    );
    let serializable = make_serializable_instruction(&update_ix);
    let ix = build_create_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber1_pubkey,
        1,
        serializable,
        0,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).expect("create_fiber_1 should succeed");

    // Advance past Timestamp target
    advance_clock(&mut svm, 15);

    // Execute fiber 0 -> Signal::Chain
    let remaining = build_remaining_accounts(&executor.pubkey());
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        &fiber0_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        false,
        0,
        &remaining,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[&executor],
        blockhash,
    );
    svm.send_transaction(tx).expect("exec fiber 0 should succeed");

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.fiber_signal, Signal::Chain);

    // Execute fiber 1 (chained) -> Signal::Update { paused: false, trigger: Interval }
    let remaining = build_remaining_accounts(&executor.pubkey());
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        &fiber1_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        false,
        1,
        &remaining,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[&executor],
        blockhash,
    );
    svm.send_transaction(tx).expect("exec fiber 1 (chained) should succeed");

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.exec_count, 2);
    assert!(
        !thread.paused,
        "Thread must NOT be auto-paused when chained fiber explicitly set paused: false"
    );
    assert_eq!(
        thread.trigger, new_trigger,
        "Trigger should be updated to Interval"
    );
    assert_eq!(thread.fiber_cursor, 0, "Cursor should be reset to 0");
}
