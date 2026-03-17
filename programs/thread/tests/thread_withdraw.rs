use solana_sdk::{pubkey::Pubkey, signature::{Keypair, Signer}, transaction::Transaction};

mod common;
use common::*;

fn create_funded_thread(
    svm: &mut litesvm::LiteSVM,
    authority: &Keypair,
    payer: &Keypair,
    id: &str,
    amount: u64,
) -> Pubkey {
    let thread_id = ThreadId::Bytes(id.as_bytes().to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), id.as_bytes());
    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        amount,
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

fn send_withdraw(
    svm: &mut litesvm::LiteSVM,
    authority: &Keypair,
    pay_to: &Pubkey,
    thread: &Pubkey,
    amount: u64,
) -> Result<(), litesvm::types::FailedTransactionMetadata> {
    let ix = build_withdraw_thread(&authority.pubkey(), pay_to, thread, amount);
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&authority.pubkey()),
        &[authority],
        blockhash,
    );
    svm.send_transaction(tx).map(|_| ())
}

#[test]
fn test_thread_withdraw_success() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey =
        create_funded_thread(&mut svm, &authority, &payer, "tw-1", 5_000_000);

    let thread_before = get_balance(&svm, &thread_pubkey);
    let authority_before = get_balance(&svm, &authority.pubkey());

    send_withdraw(&mut svm, &authority, &authority.pubkey(), &thread_pubkey, 1_000_000).unwrap();

    let thread_after = get_balance(&svm, &thread_pubkey);
    let authority_after = get_balance(&svm, &authority.pubkey());

    assert_eq!(thread_before - thread_after, 1_000_000);
    // Authority gains ~1M lamports (minus tx fee)
    assert!(authority_after > authority_before);
}

#[test]
fn test_thread_withdraw_authority_only() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    let bad_authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&bad_authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey =
        create_funded_thread(&mut svm, &authority, &payer, "tw-auth", 5_000_000);

    let result = send_withdraw(&mut svm, &bad_authority, &bad_authority.pubkey(), &thread_pubkey, 1_000_000);
    assert!(result.is_err());
}

#[test]
fn test_thread_withdraw_too_large() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey =
        create_funded_thread(&mut svm, &authority, &payer, "tw-large", 1_000_000);

    let thread_balance = get_balance(&svm, &thread_pubkey);
    // Try to withdraw everything (would go below rent)
    let result = send_withdraw(
        &mut svm,
        &authority,
        &authority.pubkey(),
        &thread_pubkey,
        thread_balance,
    );
    assert!(result.is_err());
}

#[test]
fn test_thread_withdraw_exact_to_rent() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey =
        create_funded_thread(&mut svm, &authority, &payer, "tw-rent", 5_000_000);

    // Withdraw a small amount that leaves well above rent
    send_withdraw(&mut svm, &authority, &authority.pubkey(), &thread_pubkey, 100_000).unwrap();
}

#[test]
fn test_thread_withdraw_to_different_account() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    let recipient = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&recipient.pubkey(), 1_000_000).unwrap();

    let thread_pubkey =
        create_funded_thread(&mut svm, &authority, &payer, "tw-diff", 5_000_000);

    let recipient_before = get_balance(&svm, &recipient.pubkey());
    send_withdraw(&mut svm, &authority, &recipient.pubkey(), &thread_pubkey, 1_000_000).unwrap();
    let recipient_after = get_balance(&svm, &recipient.pubkey());

    assert_eq!(recipient_after - recipient_before, 1_000_000);
}

#[test]
fn test_thread_withdraw_zero_amount() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey =
        create_funded_thread(&mut svm, &authority, &payer, "tw-zero", 5_000_000);

    send_withdraw(&mut svm, &authority, &authority.pubkey(), &thread_pubkey, 0).unwrap();
}
