use litesvm::LiteSVM;
use solana_sdk::{
    clock::Clock,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};

use super::accounts::config_pda;
use super::instructions::build_init_config;

/// Program ID for antegen_thread_program — sourced from the crate's declare_id!
pub const PROGRAM_ID: Pubkey = antegen_thread_program::ID;

/// Fiber program ID — sourced from the thread program re-export.
pub const FIBER_PROGRAM_ID: Pubkey = antegen_thread_program::fiber::ID;

/// Program ID for antegen_reentrance_test — sourced from the crate's declare_id!
pub const TEST_PROCESSOR_ID: Pubkey = antegen_reentrance_test::ID;

/// Compiled program bytes
const PROGRAM_BYTES: &[u8] = include_bytes!("../../../../target/deploy/antegen_thread_program.so");
const FIBER_PROGRAM_BYTES: &[u8] =
    include_bytes!("../../../../target/deploy/antegen_fiber_program.so");
const TEST_PROCESSOR_BYTES: &[u8] =
    include_bytes!("../../../../target/deploy/antegen_reentrance_test.so");

/// Default airdrop amount (10 SOL)
pub const DEFAULT_AIRDROP: u64 = 10_000_000_000;

/// Creates a test environment with program loaded, admin and payer funded, config initialized.
/// Returns (svm, admin, payer).
pub fn create_test_env() -> (LiteSVM, Keypair, Keypair) {
    let mut svm = LiteSVM::new();

    // Load all programs
    svm.add_program(PROGRAM_ID, PROGRAM_BYTES)
        .expect("Failed to load thread program");
    svm.add_program(FIBER_PROGRAM_ID, FIBER_PROGRAM_BYTES)
        .expect("Failed to load fiber program");
    svm.add_program(TEST_PROCESSOR_ID, TEST_PROCESSOR_BYTES)
        .expect("Failed to load test processor");

    // Create and fund admin
    let admin = Keypair::new();
    svm.airdrop(&admin.pubkey(), DEFAULT_AIRDROP).unwrap();

    // Create and fund payer
    let payer = Keypair::new();
    svm.airdrop(&payer.pubkey(), DEFAULT_AIRDROP).unwrap();

    // Initialize config
    let (config_pubkey, _) = config_pda();
    let ix = build_init_config(&admin.pubkey(), &config_pubkey);
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&admin.pubkey()), &[&admin], blockhash);
    svm.send_transaction(tx)
        .expect("config_init should succeed");

    (svm, admin, payer)
}

/// Advance the clock by the given number of seconds.
pub fn advance_clock(svm: &mut LiteSVM, seconds: i64) {
    let mut clock = svm.get_sysvar::<Clock>();
    clock.unix_timestamp += seconds;
    svm.set_sysvar(&clock);
}
