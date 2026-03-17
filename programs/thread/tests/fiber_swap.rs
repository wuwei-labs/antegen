use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};

mod common;
use common::*;

/// Setup: create thread + two fibers (index 0 and 1).
fn setup_thread_with_two_fibers(
    svm: &mut litesvm::LiteSVM,
    authority: &Keypair,
    payer: &Keypair,
    id: &str,
    source_priority_fee: u64,
) -> (Pubkey, Pubkey, Pubkey) {
    let thread_id = ThreadId::Bytes(id.as_bytes().to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), id.as_bytes());
    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        1_000_000,
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

    // Create fiber at index 0 (will be target)
    let (fiber_0, _) = fiber_pda(&thread_pubkey, 0);
    let memo_ix_0 = make_memo_instruction("target-fiber", None);
    let ser_0 = make_serializable_instruction(&memo_ix_0);
    let ix = build_create_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber_0,
        0,
        ser_0,
        0,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[payer, authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    // Create fiber at index 1 (will be source)
    let (fiber_1, _) = fiber_pda(&thread_pubkey, 1);
    let memo_ix_1 = make_memo_instruction("source-fiber", None);
    let ser_1 = make_serializable_instruction(&memo_ix_1);
    let ix = build_create_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber_1,
        1,
        ser_1,
        source_priority_fee,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[payer, authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    (thread_pubkey, fiber_0, fiber_1)
}

#[test]
fn test_fiber_swap_basic() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (thread_pubkey, target, source) =
        setup_thread_with_two_fibers(&mut svm, &authority, &payer, "fswap-basic", 0);

    // Get source's compiled instruction before swap
    let source_fiber = deserialize_fiber(&svm, &source);
    let source_compiled = source_fiber.compiled_instruction.clone();

    let ix = build_swap_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &target,
        &source,
        1, // source_fiber_index
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    // Target should now have source's compiled instruction
    let target_fiber = deserialize_fiber(&svm, &target);
    assert_eq!(target_fiber.compiled_instruction, source_compiled);
}

#[test]
fn test_fiber_swap_priority_fee_copied() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (thread_pubkey, target, source) =
        setup_thread_with_two_fibers(&mut svm, &authority, &payer, "fswap-pf", 99999);

    let ix = build_swap_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &target,
        &source,
        1,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    let target_fiber = deserialize_fiber(&svm, &target);
    assert_eq!(target_fiber.priority_fee, 99999);
}

#[test]
fn test_fiber_swap_target_stats_reset() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (thread_pubkey, target, source) =
        setup_thread_with_two_fibers(&mut svm, &authority, &payer, "fswap-stats", 0);

    let ix = build_swap_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &target,
        &source,
        1,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    let target_fiber = deserialize_fiber(&svm, &target);
    assert_eq!(target_fiber.last_executed, 0);
    assert_eq!(target_fiber.exec_count, 0);
}

#[test]
fn test_fiber_swap_source_closed() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (thread_pubkey, target, source) =
        setup_thread_with_two_fibers(&mut svm, &authority, &payer, "fswap-close", 0);

    let ix = build_swap_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &target,
        &source,
        1,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    // Source should be closed
    assert!(!account_exists(&svm, &source));
}

#[test]
fn test_fiber_swap_source_rent_returned() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (thread_pubkey, target, source) =
        setup_thread_with_two_fibers(&mut svm, &authority, &payer, "fswap-rent", 0);

    let thread_balance_before = get_balance(&svm, &thread_pubkey);

    let ix = build_swap_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &target,
        &source,
        1,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    // Thread should have received source's rent back
    let thread_balance_after = get_balance(&svm, &thread_pubkey);
    assert!(thread_balance_after > thread_balance_before);
}

#[test]
fn test_fiber_swap_wrong_authority() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    let bad_authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&bad_authority.pubkey(), DEFAULT_AIRDROP)
        .unwrap();

    let (thread_pubkey, target, source) =
        setup_thread_with_two_fibers(&mut svm, &authority, &payer, "fswap-auth", 0);

    let ix = build_swap_fiber(
        &bad_authority.pubkey(),
        &thread_pubkey,
        &target,
        &source,
        1,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &bad_authority],
        blockhash,
    );
    let result = svm.send_transaction(tx);
    assert!(result.is_err());
}

#[test]
fn test_fiber_swap_fiber_ids_updated() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (thread_pubkey, target, source) =
        setup_thread_with_two_fibers(&mut svm, &authority, &payer, "fswap-ids", 0);

    // Before swap: fiber_ids = [0, 1]
    let thread_before = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread_before.fiber_ids, vec![0, 1]);

    let ix = build_swap_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &target,
        &source,
        1,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    // After swap: source index 1 removed, target index 0 remains
    let thread_after = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread_after.fiber_ids, vec![0]);
    assert!(!thread_after.fiber_ids.contains(&1));
}
