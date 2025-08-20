use anchor_lang::prelude::*;

/// Global configuration for the thread program
#[account]
#[derive(Debug, InitSpace)]
pub struct ThreadConfig {
    /// Version for future upgrades
    pub version: u64,
    /// Bump seed for PDA
    pub bump: u8,
    /// Admin who can update configuration
    pub admin: Pubkey,
    /// Global pause flag for all threads
    pub paused: bool,
    /// Base commission fee in lamports (when executed on time)
    pub commission_fee: u64,
    /// Fee percentage for executor (9000 = 90%)
    pub executor_fee_bps: u64,
    /// Core team fee percentage (1000 = 10%)
    pub core_team_bps: u64,
    /// Grace period in seconds where full commission applies
    pub grace_period_seconds: i64,
    /// Decay period in seconds after grace (commission decays to 0)
    pub fee_decay_seconds: i64,
}

impl ThreadConfig {
    pub fn pubkey() -> Pubkey {
        Pubkey::find_program_address(&[crate::SEED_CONFIG], &crate::ID).0
    }

    pub fn space() -> usize {
        8 + Self::INIT_SPACE
    }
}
