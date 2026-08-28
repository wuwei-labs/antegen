use anchor_lang::prelude::*;

pub const SEED_CONFIG: &[u8] = b"thread_config";
pub const SEED_THREAD: &[u8] = b"thread";
pub const SEED_NONCE: &[u8] = b"thread_nonce";

pub const TRANSACTION_BASE_FEE_REIMBURSEMENT: u64 = 5_000;
pub const THREAD_MINIMUM_FEE: u64 = 1_000;
pub const CLAIM_WINDOW_SECONDS: i64 = 30;

/// Sysvar addresses used by the durable-nonce path.
///
/// Pinned as constants rather than imported: `solana_program::sysvar`'s
/// re-exports for these two are deprecated in solana-program 3, and the
/// addresses themselves are protocol constants that cannot change.
pub const SYSVAR_RECENT_BLOCKHASHES: Pubkey =
    pubkey!("SysvarRecentB1ockHashes11111111111111111111");
pub const SYSVAR_RENT: Pubkey = pubkey!("SysvarRent111111111111111111111111111111111");
