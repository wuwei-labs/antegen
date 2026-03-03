use solana_sdk::{signature::{Keypair, Signer}, transaction::Transaction};

mod common;
use common::*;

fn send_update(
    svm: &mut litesvm::LiteSVM,
    admin: &Keypair,
    params: ConfigUpdateParams,
) -> Result<(), litesvm::types::FailedTransactionMetadata> {
    let (config_pubkey, _) = config_pda();
    let ix = build_update_config(&admin.pubkey(), &config_pubkey, params);
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&admin.pubkey()), &[admin], blockhash);
    svm.send_transaction(tx).map(|_| ())
}

#[test]
fn test_config_update_admin_only() {
    let (mut svm, _admin, payer) = create_test_env();
    // Non-admin tries to update
    let result = send_update(
        &mut svm,
        &payer, // payer is not admin
        ConfigUpdateParams {
            commission_fee: Some(2000),
            ..Default::default()
        },
    );
    assert!(result.is_err());
}

#[test]
fn test_config_update_commission_fee() {
    let (mut svm, admin, _payer) = create_test_env();
    send_update(
        &mut svm,
        &admin,
        ConfigUpdateParams {
            commission_fee: Some(5000),
            ..Default::default()
        },
    )
    .unwrap();

    let (config_pubkey, _) = config_pda();
    let config = deserialize_config(&svm, &config_pubkey);
    assert_eq!(config.commission_fee, 5000);
}

#[test]
fn test_config_update_executor_fee_bps() {
    let (mut svm, admin, _payer) = create_test_env();
    send_update(
        &mut svm,
        &admin,
        ConfigUpdateParams {
            executor_fee_bps: Some(8000),
            core_team_bps: Some(2000), // must sum to 10000
            ..Default::default()
        },
    )
    .unwrap();

    let (config_pubkey, _) = config_pda();
    let config = deserialize_config(&svm, &config_pubkey);
    assert_eq!(config.executor_fee_bps, 8000);
    assert_eq!(config.core_team_bps, 2000);
}

#[test]
fn test_config_update_invalid_fee_over_10000() {
    let (mut svm, admin, _payer) = create_test_env();
    let result = send_update(
        &mut svm,
        &admin,
        ConfigUpdateParams {
            executor_fee_bps: Some(10001),
            ..Default::default()
        },
    );
    assert!(result.is_err());
}

#[test]
fn test_config_update_total_fees_must_equal_10000() {
    let (mut svm, admin, _payer) = create_test_env();
    let result = send_update(
        &mut svm,
        &admin,
        ConfigUpdateParams {
            executor_fee_bps: Some(5000),
            // core_team_bps stays at 1000 -> total = 6000 != 10000
            ..Default::default()
        },
    );
    assert!(result.is_err());
}

#[test]
fn test_config_update_grace_period() {
    let (mut svm, admin, _payer) = create_test_env();
    send_update(
        &mut svm,
        &admin,
        ConfigUpdateParams {
            grace_period_seconds: Some(30),
            ..Default::default()
        },
    )
    .unwrap();

    let (config_pubkey, _) = config_pda();
    let config = deserialize_config(&svm, &config_pubkey);
    assert_eq!(config.grace_period_seconds, 30);
}

#[test]
fn test_config_update_grace_period_exceeds_max() {
    let (mut svm, admin, _payer) = create_test_env();
    let result = send_update(
        &mut svm,
        &admin,
        ConfigUpdateParams {
            grace_period_seconds: Some(61), // max is 60
            ..Default::default()
        },
    );
    assert!(result.is_err());
}

#[test]
fn test_config_update_fee_decay_exceeds_max() {
    let (mut svm, admin, _payer) = create_test_env();
    let result = send_update(
        &mut svm,
        &admin,
        ConfigUpdateParams {
            fee_decay_seconds: Some(601), // max is 600
            ..Default::default()
        },
    );
    assert!(result.is_err());
}

#[test]
fn test_config_update_negative_grace_period() {
    let (mut svm, admin, _payer) = create_test_env();
    let result = send_update(
        &mut svm,
        &admin,
        ConfigUpdateParams {
            grace_period_seconds: Some(-1),
            ..Default::default()
        },
    );
    assert!(result.is_err());
}

#[test]
fn test_config_update_admin_transfer() {
    let (mut svm, admin, _payer) = create_test_env();
    let new_admin = Keypair::new();
    svm.airdrop(&new_admin.pubkey(), 1_000_000_000).unwrap();

    // Transfer admin
    send_update(
        &mut svm,
        &admin,
        ConfigUpdateParams {
            admin: Some(new_admin.pubkey()),
            ..Default::default()
        },
    )
    .unwrap();

    // New admin can update
    send_update(
        &mut svm,
        &new_admin,
        ConfigUpdateParams {
            commission_fee: Some(2000),
            ..Default::default()
        },
    )
    .unwrap();

    // Old admin cannot
    let result = send_update(
        &mut svm,
        &admin,
        ConfigUpdateParams {
            commission_fee: Some(3000),
            ..Default::default()
        },
    );
    assert!(result.is_err());
}

#[test]
fn test_config_update_pause_state() {
    let (mut svm, admin, _payer) = create_test_env();
    send_update(
        &mut svm,
        &admin,
        ConfigUpdateParams {
            paused: Some(true),
            ..Default::default()
        },
    )
    .unwrap();

    let (config_pubkey, _) = config_pda();
    let config = deserialize_config(&svm, &config_pubkey);
    assert!(config.paused);
}

#[test]
fn test_config_update_multiple_params() {
    let (mut svm, admin, _payer) = create_test_env();
    send_update(
        &mut svm,
        &admin,
        ConfigUpdateParams {
            commission_fee: Some(3000),
            grace_period_seconds: Some(10),
            fee_decay_seconds: Some(100),
            ..Default::default()
        },
    )
    .unwrap();

    let (config_pubkey, _) = config_pda();
    let config = deserialize_config(&svm, &config_pubkey);
    assert_eq!(config.commission_fee, 3000);
    assert_eq!(config.grace_period_seconds, 10);
    assert_eq!(config.fee_decay_seconds, 100);
}
