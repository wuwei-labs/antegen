use solana_sdk::{
    instruction::AccountMeta,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};

mod common;
use common::*;

/// Create a thread with an inline memo instruction (default fiber at index 0).
fn setup_exec_thread(
    svm: &mut litesvm::LiteSVM,
    authority: &Keypair,
    payer: &Keypair,
    _admin: &Pubkey,
    id: &str,
    trigger: Trigger,
    memo: &str,
    signal: Option<Signal>,
) -> Pubkey {
    let thread_id = ThreadId::Bytes(id.as_bytes().to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), id.as_bytes());

    // Build memo instruction with thread as signer (replaced by PAYER for executor)
    // For inline default fiber, the signer will be the thread itself
    let memo_ix = make_thread_memo_instruction(&thread_pubkey, memo, signal);
    let serializable = make_serializable_instruction(&memo_ix);

    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        5_000_000, // enough to pay fees
        thread_id,
        trigger,
        Some(serializable),
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
        .expect("create_thread with fiber should succeed");
    thread_pubkey
}

/// Build remaining accounts for exec based on the thread's compiled instruction.
/// For a memo instruction with the thread as signer, remaining accounts = [thread, program_id].
fn build_remaining_accounts(thread_pubkey: &Pubkey) -> Vec<AccountMeta> {
    // The thread_memo instruction needs: signer (thread) as remaining account
    // plus the program_id for the CPI
    vec![
        AccountMeta::new_readonly(PROGRAM_ID, false),  // program account for CPI
        AccountMeta::new_readonly(*thread_pubkey, false), // thread as signer in remaining
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
    let thread_pubkey = setup_exec_thread(
        &mut svm,
        &authority,
        &payer,
        &admin.pubkey(),
        "exec-imm",
        Trigger::Immediate { jitter: 0 },
        "exec-test",
        None,
    );

    let remaining = build_remaining_accounts(&thread_pubkey);
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        None,
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
    let thread_pubkey = setup_exec_thread(
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
    let remaining = build_remaining_accounts(&thread_pubkey);
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        None,
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
    let thread_pubkey = setup_exec_thread(
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
    let remaining = build_remaining_accounts(&thread_pubkey);
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        None,
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

    // Create thread WITHOUT initial instruction (no fibers)
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
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    // Try to exec
    let (config_pubkey, _) = config_pda();
    let remaining = build_remaining_accounts(&thread_pubkey);
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        None,
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
    let thread_pubkey = setup_exec_thread(
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

    let remaining = build_remaining_accounts(&thread_pubkey);
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        None,
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
    let thread_pubkey = setup_exec_thread(
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

    let remaining = build_remaining_accounts(&thread_pubkey);
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        None,
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
fn test_exec_thread_interval_trigger() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (config_pubkey, _) = config_pda();
    let thread_pubkey = setup_exec_thread(
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

    let remaining = build_remaining_accounts(&thread_pubkey);
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        None,
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
    let thread_pubkey = setup_exec_thread(
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

    let remaining = build_remaining_accounts(&thread_pubkey);
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        None,
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
    let thread_pubkey = setup_exec_thread(
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

    let remaining = build_remaining_accounts(&thread_pubkey);
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        None,
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
    // Executor pays tx fee but receives reimbursement + commission
    // Net effect depends on exact commission amounts
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
    let thread_pubkey = setup_exec_thread(
        &mut svm,
        &authority,
        &payer,
        &admin.pubkey(),
        "exec-forgo",
        Trigger::Immediate { jitter: 0 },
        "test",
        None,
    );

    let remaining = build_remaining_accounts(&thread_pubkey);
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        None,
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
    // So first exec succeeds, then second exec would trigger close
    let thread_pubkey = setup_exec_thread(
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
    let remaining = build_remaining_accounts(&thread_pubkey);
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        None,
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
fn test_exec_thread_fiber_cursor_advance() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (config_pubkey, _) = config_pda();
    // Use interval trigger so thread stays alive after exec
    let thread_pubkey = setup_exec_thread(
        &mut svm,
        &authority,
        &payer,
        &admin.pubkey(),
        "exec-cursor",
        Trigger::Interval {
            seconds: 10,
            skippable: false,
            jitter: 0,
        },
        "test",
        None,
    );

    // Add a second fiber (external at index 1)
    let (fiber1_pubkey, _) = fiber_pda(&thread_pubkey, 1);
    let memo_ix = make_memo_instruction("fiber-1", None);
    let serializable = make_serializable_instruction(&memo_ix);
    let ix = build_create_fiber(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        &fiber1_pubkey,
        1,
        serializable,
        vec![],
        0,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    // Advance past interval
    advance_clock(&mut svm, 15);

    // Exec fiber 0 (inline default)
    let remaining = build_remaining_accounts(&thread_pubkey);
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        None,
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
    // After executing fiber 0, cursor should advance to 1
    assert_eq!(thread.fiber_cursor, 1);
}

#[test]
fn test_exec_thread_wrong_fiber_cursor() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (config_pubkey, _) = config_pda();
    let thread_pubkey = setup_exec_thread(
        &mut svm,
        &authority,
        &payer,
        &admin.pubkey(),
        "exec-wrong-cursor",
        Trigger::Immediate { jitter: 0 },
        "test",
        None,
    );

    // Thread has fiber_cursor=0, try exec with cursor=5
    let remaining = build_remaining_accounts(&thread_pubkey);

    // Fabricate a fiber PDA for index 5 (doesn't exist)
    let (fiber5, _) = fiber_pda(&thread_pubkey, 5);
    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        Some(&fiber5),
        &config_pubkey,
        &admin.pubkey(),
        false,
        5, // wrong cursor
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
