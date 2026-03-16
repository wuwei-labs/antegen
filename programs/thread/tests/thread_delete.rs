use solana_sdk::{pubkey::Pubkey, signature::{Keypair, Signer}, transaction::Transaction};

mod common;
use common::*;

fn create_thread(
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

#[test]
fn test_thread_delete_admin_success() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = create_thread(&mut svm, &authority, &payer, "td-1");
    let (config_pubkey, _) = config_pda();

    let ix = build_delete_thread(&admin.pubkey(), &config_pubkey, &thread_pubkey);
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&admin.pubkey()),
        &[&admin],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    assert!(!account_exists(&svm, &thread_pubkey));
}

#[test]
fn test_thread_delete_non_admin_fails() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = create_thread(&mut svm, &authority, &payer, "td-nonadm");
    let (config_pubkey, _) = config_pda();

    // authority is not admin
    let ix = build_delete_thread(&authority.pubkey(), &config_pubkey, &thread_pubkey);
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&authority.pubkey()),
        &[&authority],
        blockhash,
    );
    let result = svm.send_transaction(tx);
    assert!(result.is_err());
}

#[test]
fn test_thread_delete_skips_fiber_checks() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = create_thread(&mut svm, &authority, &payer, "td-fibers");

    // Add a fiber
    let (fiber_pubkey, _) = fiber_pda(&thread_pubkey, 0);
    let memo_ix = make_memo_instruction("test", None);
    let serializable = make_serializable_instruction(&memo_ix);
    let ix = build_create_fiber(
        &authority.pubkey(),
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
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    // Admin delete skips fiber checks
    let (config_pubkey, _) = config_pda();
    let ix = build_delete_thread(&admin.pubkey(), &config_pubkey, &thread_pubkey);
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&admin.pubkey()),
        &[&admin],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();
    assert!(!account_exists(&svm, &thread_pubkey));
}

#[test]
fn test_thread_delete_returns_rent_to_admin() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = create_thread(&mut svm, &authority, &payer, "td-rent");
    let (config_pubkey, _) = config_pda();

    let admin_before = get_balance(&svm, &admin.pubkey());
    let thread_balance = get_balance(&svm, &thread_pubkey);

    let ix = build_delete_thread(&admin.pubkey(), &config_pubkey, &thread_pubkey);
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&admin.pubkey()),
        &[&admin],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    let admin_after = get_balance(&svm, &admin.pubkey());
    // Admin should gain the thread's balance minus tx fee
    let gained = admin_after as i64 - admin_before as i64;
    assert!(gained > 0);
    // Should be close to thread_balance (minus tx fee ~5000)
    assert!(gained as u64 > thread_balance.saturating_sub(10_000));
}
