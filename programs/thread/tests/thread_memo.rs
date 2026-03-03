use solana_sdk::{signature::{Keypair, Signer}, transaction::Transaction};

mod common;
use common::*;

/// Thread memo can be called directly (not only via CPI).
/// The signer just needs to sign the transaction.

fn send_memo(
    svm: &mut litesvm::LiteSVM,
    signer: &Keypair,
    memo: &str,
    signal: Option<Signal>,
) -> Result<(), litesvm::types::FailedTransactionMetadata> {
    let ix = build_thread_memo(&signer.pubkey(), memo, signal);
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&signer.pubkey()),
        &[signer],
        blockhash,
    );
    svm.send_transaction(tx).map(|_| ())
}

#[test]
fn test_thread_memo_basic() {
    let (mut svm, _admin, payer) = create_test_env();
    send_memo(&mut svm, &payer, "Hello, World!", None).unwrap();
}

#[test]
fn test_thread_memo_signal_none() {
    let (mut svm, _admin, payer) = create_test_env();
    send_memo(&mut svm, &payer, "signal-none", Some(Signal::None)).unwrap();
}

#[test]
fn test_thread_memo_signal_chain() {
    let (mut svm, _admin, payer) = create_test_env();
    send_memo(&mut svm, &payer, "signal-chain", Some(Signal::Chain)).unwrap();
}

#[test]
fn test_thread_memo_signal_close() {
    let (mut svm, _admin, payer) = create_test_env();
    send_memo(&mut svm, &payer, "signal-close", Some(Signal::Close)).unwrap();
}

#[test]
fn test_thread_memo_signal_repeat() {
    let (mut svm, _admin, payer) = create_test_env();
    send_memo(&mut svm, &payer, "signal-repeat", Some(Signal::Repeat)).unwrap();
}

#[test]
fn test_thread_memo_signal_next() {
    let (mut svm, _admin, payer) = create_test_env();
    send_memo(&mut svm, &payer, "signal-next", Some(Signal::Next { index: 3 })).unwrap();
}

#[test]
fn test_thread_memo_no_signal() {
    let (mut svm, _admin, payer) = create_test_env();
    // None signal should default to Signal::None
    send_memo(&mut svm, &payer, "no-signal", None).unwrap();
}

#[test]
fn test_thread_memo_empty_string() {
    let (mut svm, _admin, payer) = create_test_env();
    send_memo(&mut svm, &payer, "", None).unwrap();
}
