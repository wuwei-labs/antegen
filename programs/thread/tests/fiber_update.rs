use solana_sdk::{pubkey::Pubkey, signature::{Keypair, Signer}, transaction::Transaction};

mod common;
use common::*;

fn setup_thread_with_fiber_account(
    svm: &mut litesvm::LiteSVM,
    authority: &Keypair,
    payer: &Keypair,
    id: &str,
) -> (Pubkey, Pubkey) {
    // Create thread without initial instruction
    let thread_id = ThreadId::Bytes(id.as_bytes().to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), id.as_bytes());
    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        10_000_000, // enough to fund 1 fiber
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

    // Create fiber at index 0
    let (fiber_pubkey, _) = fiber_pda(&thread_pubkey, 0);
    let memo_ix = make_memo_instruction("original", None);
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
    svm.send_transaction(tx).unwrap();

    (thread_pubkey, fiber_pubkey)
}

#[test]
fn test_fiber_update_success() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (thread_pubkey, fiber_pubkey) =
        setup_thread_with_fiber_account(&mut svm, &authority, &payer, "fu-1");

    let fiber_before = deserialize_fiber(&svm, &fiber_pubkey);
    let old_compiled = fiber_before.compiled_instruction.clone();

    // Update with new instruction
    let new_memo = make_memo_instruction("updated", None);
    let serializable = make_serializable_instruction(&new_memo);
    let ix = build_update_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
        0,
        serializable,
        None,
        false,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    let fiber_after = deserialize_fiber(&svm, &fiber_pubkey);
    assert_ne!(fiber_after.compiled_instruction, old_compiled);
}

#[test]
fn test_fiber_update_resets_stats() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (thread_pubkey, fiber_pubkey) =
        setup_thread_with_fiber_account(&mut svm, &authority, &payer, "fu-reset");

    // Update fiber
    let new_memo = make_memo_instruction("reset-test", None);
    let serializable = make_serializable_instruction(&new_memo);
    let ix = build_update_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
        0,
        serializable,
        None,
        false,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    let fiber = deserialize_fiber(&svm, &fiber_pubkey);
    assert_eq!(fiber.last_executed, 0);
    assert_eq!(fiber.exec_count, 0);
}

#[test]
fn test_fiber_update_authority_check() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    let bad_authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&bad_authority.pubkey(), DEFAULT_AIRDROP)
        .unwrap();

    let (thread_pubkey, fiber_pubkey) =
        setup_thread_with_fiber_account(&mut svm, &authority, &payer, "fu-auth");

    let new_memo = make_memo_instruction("bad-update", None);
    let serializable = make_serializable_instruction(&new_memo);
    let ix = build_update_fiber(
        &bad_authority.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
        0,
        serializable,
        None,
        false,
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
fn test_fiber_update_wrong_thread() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (_thread1, _fiber1) =
        setup_thread_with_fiber_account(&mut svm, &authority, &payer, "fu-t1");
    let (thread2, _fiber2) =
        setup_thread_with_fiber_account(&mut svm, &authority, &payer, "fu-t2");

    // Try to update fiber at index 0 but with thread2 — PDA seeds won't match
    let (fiber1_pda, _) = fiber_pda(&_thread1, 0);
    let new_memo = make_memo_instruction("wrong-thread", None);
    let serializable = make_serializable_instruction(&new_memo);
    let ix = build_update_fiber(
        &authority.pubkey(),
        &thread2,
        &fiber1_pda,
        0,
        serializable,
        None,
        false,
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
fn test_fiber_update_prevents_delete_thread() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (thread_pubkey, fiber_pubkey) =
        setup_thread_with_fiber_account(&mut svm, &authority, &payer, "fu-del");
    let (config_pubkey, _) = config_pda();

    let delete_ix = build_delete_thread(&authority.pubkey(), &config_pubkey, &thread_pubkey);
    let serializable = make_serializable_instruction(&delete_ix);
    let ix = build_update_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
        0,
        serializable,
        None,
        false,
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
