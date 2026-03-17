use anchor_lang::prelude::*;

pub const SEED_THREAD_FIBER: &[u8] = b"thread_fiber";

/// Static pubkey for the payer placeholder - this is a placeholder address
/// "AntegenPayer1111111111111111111111111111111" in base58
pub const PAYER_PUBKEY: Pubkey = pubkey!("AntegenPayer1111111111111111111111111111111");
