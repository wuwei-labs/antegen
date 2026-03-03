use solana_sdk::{pubkey::Pubkey, signature::{Keypair, Signer}, transaction::Transaction};

mod common;
use common::*;

/// Create thread with interval trigger ready for error reporting tests.
fn setup_error_thread(
    svm: &mut litesvm::LiteSVM,
    authority: &Keypair,
    payer: &Keypair,
    id: &str,
) -> Pubkey {
    let thread_id = ThreadId::Bytes(id.as_bytes().to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), id.as_bytes());
    let memo_ix = make_thread_memo_instruction(&thread_pubkey, "error-test", None);
    let serializable = make_serializable_instruction(&memo_ix);
    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        5_000_000,
        thread_id,
        Trigger::Interval {
            seconds: 10,
            skippable: false,
            jitter: 0,
        },
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
    svm.send_transaction(tx).unwrap();
    thread_pubkey
}

fn send_error_thread(
    svm: &mut litesvm::LiteSVM,
    executor: &Keypair,
    thread: &Pubkey,
    config: &Pubkey,
    admin: &Pubkey,
    error_code: u32,
    error_message: &str,
) -> Result<(), litesvm::types::FailedTransactionMetadata> {
    let ix = build_error_thread(
        &executor.pubkey(),
        thread,
        config,
        admin,
        error_code,
        error_message,
        &[],
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[executor],
        blockhash,
    );
    svm.send_transaction(tx).map(|_| ())
}

#[test]
fn test_error_thread_success() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (config_pubkey, _) = config_pda();
    let thread_pubkey = setup_error_thread(&mut svm, &authority, &payer, "err-1");

    // Advance past interval + grace + decay (10 + 5 + 295 = 310 seconds)
    advance_clock(&mut svm, 320);

    let executor_before = get_balance(&svm, &executor.pubkey());

    send_error_thread(
        &mut svm,
        &executor,
        &thread_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        1000,
        "test error",
    )
    .unwrap();

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert!(thread.last_error_time.is_some());

    // Executor should have received reimbursement
    let executor_after = get_balance(&svm, &executor.pubkey());
    // Net gain after tx fee should still be positive (10000 reimbursement - ~5000 fee)
    assert!(executor_after as i64 - executor_before as i64 > 0);
}

#[test]
fn test_error_thread_not_last_executor() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    let other_executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&other_executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (config_pubkey, _) = config_pda();
    let thread_pubkey = setup_error_thread(&mut svm, &authority, &payer, "err-notlast");

    // Execute once with executor to set last_executor
    advance_clock(&mut svm, 15);
    let remaining = vec![
        solana_sdk::instruction::AccountMeta::new_readonly(PROGRAM_ID, false),
        solana_sdk::instruction::AccountMeta::new_readonly(thread_pubkey, false),
    ];
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

    // Advance past grace + decay
    advance_clock(&mut svm, 320);

    // other_executor tries to report error - should fail (not last executor)
    let result = send_error_thread(
        &mut svm,
        &other_executor,
        &thread_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        1000,
        "bad error",
    );
    assert!(result.is_err());
}

#[test]
fn test_error_thread_already_reported() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (config_pubkey, _) = config_pda();
    let thread_pubkey = setup_error_thread(&mut svm, &authority, &payer, "err-dup");

    // Advance past threshold
    advance_clock(&mut svm, 320);

    // First report succeeds
    send_error_thread(
        &mut svm,
        &executor,
        &thread_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        1000,
        "first error",
    )
    .unwrap();

    // Second report fails
    let result = send_error_thread(
        &mut svm,
        &executor,
        &thread_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        1001,
        "second error",
    );
    assert!(result.is_err());
}

#[test]
fn test_error_thread_not_overdue() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (config_pubkey, _) = config_pda();
    let thread_pubkey = setup_error_thread(&mut svm, &authority, &payer, "err-notdue");

    // Only advance past interval but NOT past grace+decay threshold
    advance_clock(&mut svm, 15); // 15s - interval is 10s, but threshold is 300s

    let result = send_error_thread(
        &mut svm,
        &executor,
        &thread_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        1000,
        "too early",
    );
    assert!(result.is_err());
}

#[test]
fn test_error_thread_paused_fails() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (config_pubkey, _) = config_pda();
    let thread_pubkey = setup_error_thread(&mut svm, &authority, &payer, "err-paused");

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

    advance_clock(&mut svm, 320);

    let result = send_error_thread(
        &mut svm,
        &executor,
        &thread_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        1000,
        "paused error",
    );
    assert!(result.is_err());
}

#[test]
fn test_error_thread_reimbursement_capped() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (config_pubkey, _) = config_pda();

    // Create thread with minimal funding
    let thread_id = ThreadId::Bytes(b"err-cap".to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), b"err-cap");
    let memo_ix = make_thread_memo_instruction(&thread_pubkey, "test", None);
    let serializable = make_serializable_instruction(&memo_ix);
    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        100, // minimal extra funding
        thread_id,
        Trigger::Interval {
            seconds: 10,
            skippable: false,
            jitter: 0,
        },
        Some(serializable),
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

    advance_clock(&mut svm, 320);

    // Should still succeed - reimbursement capped at available lamports
    send_error_thread(
        &mut svm,
        &executor,
        &thread_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        1000,
        "capped",
    )
    .unwrap();
}

#[test]
fn test_error_thread_default_executor_allowed() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (config_pubkey, _) = config_pda();
    let thread_pubkey = setup_error_thread(&mut svm, &authority, &payer, "err-default");

    // Thread was just created - last_executor is Pubkey::default()
    // Any executor should be allowed to report
    advance_clock(&mut svm, 320);

    send_error_thread(
        &mut svm,
        &executor,
        &thread_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        1000,
        "default executor",
    )
    .unwrap();
}
