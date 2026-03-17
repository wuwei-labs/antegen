use solana_sdk::{pubkey::Pubkey, signature::{Keypair, Signer}, transaction::Transaction};

mod common;
use common::*;

fn create_thread_for_update(
    svm: &mut litesvm::LiteSVM,
    authority: &Keypair,
    payer: &Keypair,
    id: &str,
    trigger: Trigger,
) -> Pubkey {
    let thread_id = ThreadId::Bytes(id.as_bytes().to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), id.as_bytes());
    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        1_000_000,
        thread_id,
        trigger,
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

fn send_update(
    svm: &mut litesvm::LiteSVM,
    authority: &Keypair,
    payer: &Keypair,
    thread: &Pubkey,
    params: ThreadUpdateParams,
) -> Result<(), litesvm::types::FailedTransactionMetadata> {
    let ix = build_update_thread(&authority.pubkey(), thread, params);
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[payer, authority],
        blockhash,
    );
    svm.send_transaction(tx).map(|_| ())
}

#[test]
fn test_thread_update_pause() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey =
        create_thread_for_update(&mut svm, &authority, &payer, "tu-pause", Trigger::Immediate { jitter: 0 });
    send_update(&mut svm, &authority, &payer, &thread_pubkey, ThreadUpdateParams {
        paused: Some(true),
        ..Default::default()
    }).unwrap();

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert!(thread.paused);
}

#[test]
fn test_thread_update_unpause() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey =
        create_thread_for_update(&mut svm, &authority, &payer, "tu-unpause", Trigger::Immediate { jitter: 0 });

    // Pause
    send_update(&mut svm, &authority, &payer, &thread_pubkey, ThreadUpdateParams {
        paused: Some(true),
        ..Default::default()
    }).unwrap();
    assert!(deserialize_thread(&svm, &thread_pubkey).paused);

    // Unpause
    send_update(&mut svm, &authority, &payer, &thread_pubkey, ThreadUpdateParams {
        paused: Some(false),
        ..Default::default()
    }).unwrap();
    assert!(!deserialize_thread(&svm, &thread_pubkey).paused);
}

#[test]
fn test_thread_update_trigger_immediate() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey =
        create_thread_for_update(&mut svm, &authority, &payer, "tu-imm", Trigger::Interval { seconds: 60, skippable: false, jitter: 0 });
    send_update(&mut svm, &authority, &payer, &thread_pubkey, ThreadUpdateParams {
        trigger: Some(Trigger::Immediate { jitter: 0 }),
        ..Default::default()
    }).unwrap();

    let thread = deserialize_thread(&svm, &thread_pubkey);
    match thread.trigger {
        antegen_thread_program::state::Trigger::Immediate { .. } => {}
        _ => panic!("Expected Immediate trigger"),
    }
}

#[test]
fn test_thread_update_trigger_interval() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let clock = get_clock(&svm);
    let thread_pubkey =
        create_thread_for_update(&mut svm, &authority, &payer, "tu-int", Trigger::Immediate { jitter: 0 });
    send_update(&mut svm, &authority, &payer, &thread_pubkey, ThreadUpdateParams {
        trigger: Some(Trigger::Interval { seconds: 120, skippable: false, jitter: 0 }),
        ..Default::default()
    }).unwrap();

    let thread = deserialize_thread(&svm, &thread_pubkey);
    match thread.schedule {
        antegen_thread_program::state::Schedule::Timed { next, .. } => {
            assert!(next >= clock.unix_timestamp + 120);
        }
        _ => panic!("Expected Timed schedule"),
    }
}

#[test]
fn test_thread_update_trigger_timestamp() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let target_ts = 1800000000i64;
    let thread_pubkey =
        create_thread_for_update(&mut svm, &authority, &payer, "tu-ts", Trigger::Immediate { jitter: 0 });
    send_update(&mut svm, &authority, &payer, &thread_pubkey, ThreadUpdateParams {
        trigger: Some(Trigger::Timestamp { unix_ts: target_ts, jitter: 0 }),
        ..Default::default()
    }).unwrap();

    let thread = deserialize_thread(&svm, &thread_pubkey);
    match thread.schedule {
        antegen_thread_program::state::Schedule::Timed { next, .. } => {
            assert_eq!(next, target_ts);
        }
        _ => panic!("Expected Timed schedule"),
    }
}

#[test]
fn test_thread_update_trigger_cron() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let clock = get_clock(&svm);
    let thread_pubkey =
        create_thread_for_update(&mut svm, &authority, &payer, "tu-cron", Trigger::Immediate { jitter: 0 });
    send_update(&mut svm, &authority, &payer, &thread_pubkey, ThreadUpdateParams {
        trigger: Some(Trigger::Cron {
            schedule: "0 * * * * * *".to_string(),
            skippable: false,
            jitter: 0,
        }),
        ..Default::default()
    }).unwrap();

    let thread = deserialize_thread(&svm, &thread_pubkey);
    match thread.schedule {
        antegen_thread_program::state::Schedule::Timed { next, .. } => {
            assert!(next > clock.unix_timestamp);
        }
        _ => panic!("Expected Timed schedule"),
    }
}

#[test]
fn test_thread_update_trigger_slot() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey =
        create_thread_for_update(&mut svm, &authority, &payer, "tu-slot", Trigger::Immediate { jitter: 0 });
    send_update(&mut svm, &authority, &payer, &thread_pubkey, ThreadUpdateParams {
        trigger: Some(Trigger::Slot { slot: 1000 }),
        ..Default::default()
    }).unwrap();

    let thread = deserialize_thread(&svm, &thread_pubkey);
    match thread.schedule {
        antegen_thread_program::state::Schedule::Block { next, .. } => {
            assert_eq!(next, 1000);
        }
        _ => panic!("Expected Block schedule"),
    }
}

#[test]
fn test_thread_update_authority_only() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    let bad_authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&bad_authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey =
        create_thread_for_update(&mut svm, &authority, &payer, "tu-auth", Trigger::Immediate { jitter: 0 });
    let result = send_update(&mut svm, &bad_authority, &payer, &thread_pubkey, ThreadUpdateParams {
        paused: Some(true),
        ..Default::default()
    });
    assert!(result.is_err());
}

#[test]
fn test_thread_update_no_params() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey =
        create_thread_for_update(&mut svm, &authority, &payer, "tu-noop", Trigger::Immediate { jitter: 0 });
    let thread_before = deserialize_thread(&svm, &thread_pubkey);

    send_update(&mut svm, &authority, &payer, &thread_pubkey, ThreadUpdateParams::default()).unwrap();

    let thread_after = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread_before.paused, thread_after.paused);
}

#[test]
fn test_thread_update_both_params() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey =
        create_thread_for_update(&mut svm, &authority, &payer, "tu-both", Trigger::Immediate { jitter: 0 });
    send_update(&mut svm, &authority, &payer, &thread_pubkey, ThreadUpdateParams {
        paused: Some(true),
        trigger: Some(Trigger::Interval { seconds: 30, skippable: false, jitter: 0 }),
    }).unwrap();

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert!(thread.paused);
    match thread.trigger {
        antegen_thread_program::state::Trigger::Interval { seconds, .. } => {
            assert_eq!(seconds, 30);
        }
        _ => panic!("Expected Interval trigger"),
    }
}
