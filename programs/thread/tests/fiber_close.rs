use solana_sdk::{pubkey::Pubkey, signature::{Keypair, Signer}, transaction::Transaction};

mod common;
use common::*;

/// Setup: create thread + account-based fiber.
fn setup_thread_external(
    svm: &mut litesvm::LiteSVM,
    authority: &Keypair,
    payer: &Keypair,
    id: &str,
) -> (Pubkey, Pubkey) {
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

    // Create external fiber at index 0
    let (fiber_pubkey, _) = fiber_pda(&thread_pubkey, 0);
    let memo_ix = make_memo_instruction("external", None);
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
fn test_fiber_close_external_fiber() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (thread_pubkey, fiber_pubkey) =
        setup_thread_external(&mut svm, &authority, &payer, "fclose-ext");

    let ix = build_close_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
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

    // Fiber account should be closed
    assert!(!account_exists(&svm, &fiber_pubkey));
    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert!(!thread.fiber_ids.contains(&0));
}

#[test]
fn test_fiber_close_authority_check() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    let bad_authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&bad_authority.pubkey(), DEFAULT_AIRDROP)
        .unwrap();

    let (thread_pubkey, fiber_pubkey) =
        setup_thread_external(&mut svm, &authority, &payer, "fclose-auth");

    let ix = build_close_fiber(
        &bad_authority.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
        0,
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
fn test_fiber_close_rent_returned() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (thread_pubkey, fiber_pubkey) =
        setup_thread_external(&mut svm, &authority, &payer, "fclose-rent");

    // Rent returns to thread PDA (via Fiber Program close = thread)
    let thread_before = get_balance(&svm, &thread_pubkey);

    let ix = build_close_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
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

    // Thread should receive fiber rent back
    let thread_after = get_balance(&svm, &thread_pubkey);
    assert!(thread_after > thread_before);
}

#[test]
fn test_fiber_close_last_fiber_resets_cursor() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (thread_pubkey, fiber_pubkey) =
        setup_thread_external(&mut svm, &authority, &payer, "fclose-last");

    let ix = build_close_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
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

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert!(thread.fiber_ids.is_empty());
    assert_eq!(thread.fiber_cursor, 0);
}

#[test]
fn test_fiber_close_middle_of_sequence() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    // Create thread with no initial instruction
    let thread_id = ThreadId::Bytes(b"fclose-mid".to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), b"fclose-mid");
    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        30_000_000, // enough to fund 3 fibers
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

    // Create 3 fibers: 0, 1, 2
    let mut fiber_pubkeys = vec![];
    for i in 0..3u8 {
        let (fiber_pubkey, _) = fiber_pda(&thread_pubkey, i);
        let memo_ix = make_memo_instruction(&format!("fiber-{}", i), None);
        let serializable = make_serializable_instruction(&memo_ix);
        let ix = build_create_fiber(
            &authority.pubkey(),
            &thread_pubkey,
            &fiber_pubkey,
            i,
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
        svm.send_transaction(tx).unwrap();
        fiber_pubkeys.push(fiber_pubkey);
    }

    // Close fiber at index 1 (middle)
    let ix = build_close_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber_pubkeys[1],
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

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.fiber_ids, vec![0, 2]);
}

#[test]
fn test_fiber_close_advances_cursor() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    // Create thread with no initial instruction
    let thread_id = ThreadId::Bytes(b"fclose-adv".to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), b"fclose-adv");
    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        20_000_000, // enough to fund 2 fibers
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

    // Create 2 fibers: 0, 1
    for i in 0..2u8 {
        let (fiber_pubkey, _) = fiber_pda(&thread_pubkey, i);
        let memo_ix = make_memo_instruction(&format!("fiber-{}", i), None);
        let serializable = make_serializable_instruction(&memo_ix);
        let ix = build_create_fiber(
            &authority.pubkey(),
            &thread_pubkey,
            &fiber_pubkey,
            i,
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
        svm.send_transaction(tx).unwrap();
    }

    // fiber_cursor starts at 0. Close fiber 0 -> should advance to 1.
    let (fiber0_pubkey, _) = fiber_pda(&thread_pubkey, 0);
    let ix = build_close_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber0_pubkey,
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

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.fiber_ids, vec![1]);
    // Cursor should have advanced from 0 before removal
    assert_eq!(thread.fiber_cursor, 1);
}
