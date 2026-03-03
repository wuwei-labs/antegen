use solana_sdk::{signature::Signer, transaction::Transaction};

mod common;
use common::*;

#[test]
fn test_config_init_success() {
    let (svm, admin, _payer) = create_test_env();
    let (config_pubkey, _) = config_pda();

    let config = deserialize_config(&svm, &config_pubkey);
    assert_eq!(config.version, 1);
    assert_eq!(config.admin, admin.pubkey());
    assert!(!config.paused);
    assert_eq!(config.commission_fee, 1000);
    assert_eq!(config.executor_fee_bps, 9000);
    assert_eq!(config.core_team_bps, 1000);
    assert_eq!(config.grace_period_seconds, 5);
    assert_eq!(config.fee_decay_seconds, 295);
}

#[test]
fn test_config_init_sets_correct_bump() {
    let (svm, _admin, _payer) = create_test_env();
    let (config_pubkey, expected_bump) = config_pda();

    let config = deserialize_config(&svm, &config_pubkey);
    assert_eq!(config.bump, expected_bump);
}

#[test]
fn test_config_init_already_exists() {
    let (mut svm, admin, _payer) = create_test_env();
    let (config_pubkey, _) = config_pda();

    // Try to init again - should fail
    let ix = build_init_config(&admin.pubkey(), &config_pubkey);
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&admin.pubkey()), &[&admin], blockhash);
    let result = svm.send_transaction(tx);
    assert!(result.is_err(), "Double init should fail");
}

#[test]
fn test_config_pda_derivation() {
    let (config_pubkey, _) = config_pda();
    let expected = antegen_thread_program::state::ThreadConfig::pubkey();
    assert_eq!(config_pubkey, expected);
}
