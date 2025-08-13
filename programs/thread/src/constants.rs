use anchor_lang::prelude::*;

#[constant]
pub const SEED_THREAD: &[u8] = b"thread";
pub const SEED_THREAD_FIBER: &[u8] = b"thread_fiber";
pub const SEED_NONCE: &[u8] = b"thread_nonce";

pub const TRANSACTION_BASE_FEE_REIMBURSEMENT: u64 = 5_000;
pub const THREAD_MINIMUM_FEE: u64 = 1_000;
