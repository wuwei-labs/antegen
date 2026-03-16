use solana_sdk::{pubkey::Pubkey, signature::{Keypair, Signer}, transaction::Transaction};

mod common;
use common::*;

/// Setup: create thread with inline default fiber (index 0).
fn setup_thread_inline(
    svm: &mut litesvm::LiteSVM,
    authority: &Keypair,
    payer: &Keypair,
    id: &str,
) -> Pubkey {
    let thread_id = ThreadId::Bytes(id.as_bytes().to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), id.as_bytes());
    let memo_ix = make_memo_instruction("inline", None);
    let serializable = make_serializable_instruction(&memo_ix);
    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        1_000_000,
        thread_id,
        Trigger::Immediate { jitter: 0 },
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
        1_000_000,
        thread_id,
        Trigger::Immediate { jitter: 0 },
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
        &payer.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
        0,
        serializable,
        vec![],
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
        &payer.pubkey(),
        &thread_pubkey,
        Some(&fiber_pubkey),
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
fn test_fiber_close_default_fiber() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread_inline(&mut svm, &authority, &payer, "fclose-def");

    // Close inline fiber (no fiber account needed)
    let ix = build_close_fiber(
        &authority.pubkey(),
        &authority.pubkey(),
        &thread_pubkey,
        None,
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
    assert!(thread.default_fiber.is_none());
    assert!(thread.fiber_ids.is_empty());
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
        &payer.pubkey(),
        &thread_pubkey,
        Some(&fiber_pubkey),
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

    // payer is both tx fee payer and the original fiber rent payer (fiber.payer)
    let payer_before = get_balance(&svm, &payer.pubkey());

    let ix = build_close_fiber(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        Some(&fiber_pubkey),
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

    // Payer receives fiber rent back (minus tx fee), net should be positive
    let payer_after = get_balance(&svm, &payer.pubkey());
    assert!(payer_after > payer_before);
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
        &payer.pubkey(),
        &thread_pubkey,
        Some(&fiber_pubkey),
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
fn test_fiber_close_requires_account() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let (thread_pubkey, _fiber_pubkey) =
        setup_thread_external(&mut svm, &authority, &payer, "fclose-req");

    // Try to close account-based fiber without providing the account
    let ix = build_close_fiber(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        None, // Missing fiber account!
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
        1_000_000,
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

    // Create 3 fibers: 0, 1, 2
    let mut fiber_pubkeys = vec![];
    for i in 0..3u8 {
        let (fiber_pubkey, _) = fiber_pda(&thread_pubkey, i);
        let memo_ix = make_memo_instruction(&format!("fiber-{}", i), None);
        let serializable = make_serializable_instruction(&memo_ix);
        let ix = build_create_fiber(
            &authority.pubkey(),
            &payer.pubkey(),
            &thread_pubkey,
            &fiber_pubkey,
            i,
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
        fiber_pubkeys.push(fiber_pubkey);
    }

    // Close fiber at index 1 (middle)
    let ix = build_close_fiber(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        Some(&fiber_pubkeys[1]),
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
        1_000_000,
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

    // Create 2 fibers: 0, 1
    for i in 0..2u8 {
        let (fiber_pubkey, _) = fiber_pda(&thread_pubkey, i);
        let memo_ix = make_memo_instruction(&format!("fiber-{}", i), None);
        let serializable = make_serializable_instruction(&memo_ix);
        let ix = build_create_fiber(
            &authority.pubkey(),
            &payer.pubkey(),
            &thread_pubkey,
            &fiber_pubkey,
            i,
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
    }

    // fiber_cursor starts at 0. Close fiber 0 -> should advance to 1.
    let (fiber0_pubkey, _) = fiber_pda(&thread_pubkey, 0);
    let ix = build_close_fiber(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        Some(&fiber0_pubkey),
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
