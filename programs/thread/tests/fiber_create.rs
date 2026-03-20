use solana_sdk::{pubkey::Pubkey, signature::{Keypair, Signer}, transaction::Transaction};

mod common;
use common::*;

/// Helper to create a thread with no fibers and return thread_pubkey.
fn setup_thread(
    svm: &mut litesvm::LiteSVM,
    authority: &Keypair,
    payer: &Keypair,
    id: &str,
) -> Pubkey {
    let thread_id = ThreadId::Bytes(id.as_bytes().to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), id.as_bytes());
    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        30_000_000, // enough to fund up to 3 fibers
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
        &[payer, authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();
    thread_pubkey
}

/// Helper to send create_fiber.
fn send_create_fiber(
    svm: &mut litesvm::LiteSVM,
    authority: &Keypair,
    payer: &Keypair,
    thread: &Pubkey,
    fiber_index: u8,
    priority_fee: u64,
) -> Result<Pubkey, litesvm::types::FailedTransactionMetadata> {
    let (fiber_pubkey, _) = fiber_pda(thread, fiber_index);
    let memo_ix = make_memo_instruction("fiber-test", None);
    let serializable = make_serializable_instruction(&memo_ix);

    let ix = build_create_fiber(
        &authority.pubkey(),
        thread,
        &fiber_pubkey,
        fiber_index,
        serializable,
        priority_fee,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[payer, authority],
        blockhash,
    );
    svm.send_transaction(tx).map(|_| fiber_pubkey)
}

#[test]
fn test_fiber_create_success() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-1");
    let fiber_pubkey = send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 0, 100)
        .unwrap();

    let fiber = deserialize_fiber(&svm, &fiber_pubkey);
    assert_eq!(fiber.thread, thread_pubkey);
    assert_eq!(fiber.priority_fee, 100);
    assert_eq!(fiber.last_executed, 0);
    assert_eq!(fiber.exec_count, 0);
    assert!(!fiber.compiled_instruction.is_empty());
}

#[test]
fn test_fiber_create_sequential_index() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-seq");

    send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 0, 0).unwrap();
    send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 1, 0).unwrap();
    send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 2, 0).unwrap();

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.fiber_ids, vec![0, 1, 2]);
    assert_eq!(thread.fiber_next_id, 3);
}

#[test]
fn test_fiber_create_wrong_index() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-wrong");

    // fiber_next_id is 0, try index 1
    let result = send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 1, 0);
    assert!(result.is_err());
}

#[test]
fn test_fiber_create_non_sequential() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-skip");
    send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 0, 0).unwrap();

    // Try to skip index 1 and go to 2
    let result = send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 2, 0);
    assert!(result.is_err());
}

#[test]
fn test_fiber_create_authority_check() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    let bad_authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&bad_authority.pubkey(), DEFAULT_AIRDROP)
        .unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-auth");

    // Use wrong authority
    let result = send_create_fiber(&mut svm, &bad_authority, &payer, &thread_pubkey, 0, 0);
    assert!(result.is_err());
}

#[test]
fn test_fiber_create_prevents_delete_thread() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-del");
    let (config_pubkey, _) = config_pda();

    // Build a delete_thread instruction as the fiber content
    let delete_ix = build_delete_thread(&authority.pubkey(), &config_pubkey, &thread_pubkey);
    let serializable = make_serializable_instruction(&delete_ix);

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
    let result = svm.send_transaction(tx);
    assert!(result.is_err());
}

#[test]
fn test_fiber_create_with_priority_fee() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-pf");
    let fiber_pubkey =
        send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 0, 50000).unwrap();

    let fiber = deserialize_fiber(&svm, &fiber_pubkey);
    assert_eq!(fiber.priority_fee, 50000);
}

#[test]
fn test_fiber_create_pda_derivation() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-pda");
    let fiber_pubkey =
        send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 0, 0).unwrap();

    let expected = antegen_fiber_program::state::FiberState::pubkey(thread_pubkey, 0);
    assert_eq!(fiber_pubkey, expected);
}

#[test]
fn test_fiber_create_updates_thread() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-upd");
    let thread_before = deserialize_thread(&svm, &thread_pubkey);
    assert!(thread_before.fiber_ids.is_empty());
    assert_eq!(thread_before.fiber_next_id, 0);

    send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 0, 0).unwrap();

    let thread_after = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread_after.fiber_ids, vec![0]);
    assert_eq!(thread_after.fiber_next_id, 1);
}

#[test]
fn test_fiber_create_compiled_roundtrip() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-rt");
    let fiber_pubkey =
        send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 0, 0).unwrap();

    let fiber = deserialize_fiber(&svm, &fiber_pubkey);
    // Compiled bytes should be deserializable
    let compiled = borsh::from_slice::<antegen_fiber_program::state::CompiledInstructionV0>(
        &fiber.compiled_instruction,
    );
    assert!(compiled.is_ok());
}
