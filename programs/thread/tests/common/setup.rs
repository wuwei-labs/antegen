use antegen_thread_program as thread_program;
use litesvm::LiteSVM;
use solana_sdk::{
    clock::Clock,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};

use super::accounts::config_pda;
use super::instructions::build_init_config;

pub const PROGRAM_ID: Pubkey = thread_program::ID;
pub const FIBER_PROGRAM_ID: Pubkey = thread_program::fiber::ID;

/// Compiled program bytes
const PROGRAM_BYTES: &[u8] = include_bytes!("../../../../target/deploy/antegen_thread_program.so");
const FIBER_PROGRAM_BYTES: &[u8] =
    include_bytes!("../../../../target/deploy/antegen_fiber_program.so");

/// Default airdrop amount (10 SOL)
pub const DEFAULT_AIRDROP: u64 = 10_000_000_000;

/// Creates a test environment with program loaded, admin and payer funded, config initialized.
/// Returns (svm, admin, payer).
pub fn create_test_env() -> (LiteSVM, Keypair, Keypair) {
    let (mut svm, admin, payer) = create_test_env_no_config();

    // Initialize config
    let (config_pubkey, _) = config_pda();
    let ix = build_init_config(&admin.pubkey(), &config_pubkey);
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&admin.pubkey()), &[&admin], blockhash);
    svm.send_transaction(tx)
        .expect("config_init should succeed");

    (svm, admin, payer)
}

/// Creates a test environment without initializing config.
/// Returns (svm, admin, payer).
fn create_test_env_no_config() -> (LiteSVM, Keypair, Keypair) {
    let mut svm = LiteSVM::new();

    // Load all programs
    svm.add_program(PROGRAM_ID, PROGRAM_BYTES)
        .expect("Failed to load thread program");
    svm.add_program(FIBER_PROGRAM_ID, FIBER_PROGRAM_BYTES)
        .expect("Failed to load fiber program");

    // Create and fund admin
    let admin = Keypair::new();
    svm.airdrop(&admin.pubkey(), DEFAULT_AIRDROP).unwrap();

    // Create and fund payer
    let payer = Keypair::new();
    svm.airdrop(&payer.pubkey(), DEFAULT_AIRDROP).unwrap();

    (svm, admin, payer)
}

/// Advance the clock by the given number of seconds.
pub fn advance_clock(svm: &mut LiteSVM, seconds: i64) {
    let mut clock = svm.get_sysvar::<Clock>();
    clock.unix_timestamp += seconds;
    svm.set_sysvar(&clock);
}

/// Warp the slot to a specific value.
pub fn warp_to_slot(svm: &mut LiteSVM, slot: u64) {
    svm.warp_to_slot(slot);
}

/// Get current clock from the SVM.
pub fn get_clock(svm: &LiteSVM) -> Clock {
    svm.get_sysvar::<Clock>()
}
