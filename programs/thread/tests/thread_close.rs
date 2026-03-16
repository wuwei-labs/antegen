use solana_sdk::{pubkey::Pubkey, signature::{Keypair, Signer}, transaction::Transaction};

mod common;
use common::*;

fn create_thread_no_fiber(
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
    thread_pubkey
}

fn create_thread_with_default_fiber(
    svm: &mut litesvm::LiteSVM,
    authority: &Keypair,
    payer: &Keypair,
    id: &str,
) -> Pubkey {
    let thread_id = ThreadId::Bytes(id.as_bytes().to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), id.as_bytes());
    let memo_ix = make_memo_instruction("fiber-0", None);
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

fn add_external_fiber(
    svm: &mut litesvm::LiteSVM,
    authority: &Keypair,
    payer: &Keypair,
    thread: &Pubkey,
    index: u8,
) -> Pubkey {
    let (fiber_pubkey, _) = fiber_pda(thread, index);
    let memo_ix = make_memo_instruction(&format!("fiber-{}", index), None);
    let serializable = make_serializable_instruction(&memo_ix);
    let ix = build_create_fiber(
        &authority.pubkey(),
        &payer.pubkey(),
        thread,
        &fiber_pubkey,
        index,
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
    fiber_pubkey
}

#[test]
fn test_thread_close_no_fibers() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = create_thread_no_fiber(&mut svm, &authority, &payer, "tc-empty");
    let close_to_before = get_balance(&svm, &authority.pubkey());

    let ix = build_close_thread(&authority.pubkey(), &authority.pubkey(), &thread_pubkey, &[]);
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    assert!(!account_exists(&svm, &thread_pubkey));
    let close_to_after = get_balance(&svm, &authority.pubkey());
    assert!(close_to_after > close_to_before);
}

#[test]
fn test_thread_close_with_default_fiber_only() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey =
        create_thread_with_default_fiber(&mut svm, &authority, &payer, "tc-def");

    // Close with no remaining accounts (inline fiber only)
    let ix = build_close_thread(&authority.pubkey(), &authority.pubkey(), &thread_pubkey, &[]);
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    assert!(!account_exists(&svm, &thread_pubkey));
}

#[test]
fn test_thread_close_with_external_fibers() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = create_thread_no_fiber(&mut svm, &authority, &payer, "tc-ext");
    let fiber0 = add_external_fiber(&mut svm, &authority, &payer, &thread_pubkey, 0);
    let fiber1 = add_external_fiber(&mut svm, &authority, &payer, &thread_pubkey, 1);

    let ix = build_close_thread(
        &authority.pubkey(),
        &authority.pubkey(),
        &thread_pubkey,
        &[fiber0, fiber1],
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    assert!(!account_exists(&svm, &thread_pubkey));
    assert!(!account_exists(&svm, &fiber0));
    assert!(!account_exists(&svm, &fiber1));
}

#[test]
fn test_thread_close_missing_fiber_accounts() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = create_thread_no_fiber(&mut svm, &authority, &payer, "tc-miss");
    let _fiber0 = add_external_fiber(&mut svm, &authority, &payer, &thread_pubkey, 0);
    let _fiber1 = add_external_fiber(&mut svm, &authority, &payer, &thread_pubkey, 1);

    // Only provide fiber0, missing fiber1
    let (fiber0, _) = fiber_pda(&thread_pubkey, 0);
    let ix = build_close_thread(
        &authority.pubkey(),
        &authority.pubkey(),
        &thread_pubkey,
        &[fiber0], // missing fiber1
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
fn test_thread_close_wrong_fiber_thread() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    // Create two threads
    let thread1 = create_thread_no_fiber(&mut svm, &authority, &payer, "tc-wf1");
    let _fiber_t1 = add_external_fiber(&mut svm, &authority, &payer, &thread1, 0);

    let thread2 = create_thread_no_fiber(&mut svm, &authority, &payer, "tc-wf2");
    let fiber_t2 = add_external_fiber(&mut svm, &authority, &payer, &thread2, 0);

    // Try closing thread1 with thread2's fiber
    let ix = build_close_thread(
        &authority.pubkey(),
        &authority.pubkey(),
        &thread1,
        &[fiber_t2], // wrong thread's fiber
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
fn test_thread_close_authority_check() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    let bad_authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&bad_authority.pubkey(), DEFAULT_AIRDROP)
        .unwrap();

    let thread_pubkey = create_thread_no_fiber(&mut svm, &authority, &payer, "tc-auth");

    let ix = build_close_thread(
        &bad_authority.pubkey(),
        &bad_authority.pubkey(),
        &thread_pubkey,
        &[],
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
fn test_thread_close_returns_all_lamports() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    let close_to = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&close_to.pubkey(), 1_000_000).unwrap();

    let thread_pubkey = create_thread_no_fiber(&mut svm, &authority, &payer, "tc-lam");
    let fiber0 = add_external_fiber(&mut svm, &authority, &payer, &thread_pubkey, 0);

    let thread_balance = get_balance(&svm, &thread_pubkey);
    let fiber_balance = get_balance(&svm, &fiber0);
    let close_to_before = get_balance(&svm, &close_to.pubkey());

    let ix = build_close_thread(
        &authority.pubkey(),
        &close_to.pubkey(),
        &thread_pubkey,
        &[fiber0],
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    let close_to_after = get_balance(&svm, &close_to.pubkey());
    // close_to should receive thread + fiber lamports
    assert_eq!(
        close_to_after - close_to_before,
        thread_balance + fiber_balance
    );
}
