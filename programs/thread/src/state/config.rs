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
    /// Base commission fee in lamports
    pub commission_fee: u64,
    /// Fee percentage when observer executes their own claim (9000 = 90%)
    pub observer_fee_bps: u64,
    /// Fee percentage when external executor helps (500 = 5%)
    pub executor_helper_fee_bps: u64,
    /// Observer's share when different executor helps (8500 = 85%)
    pub observer_share_bps: u64,
    /// Core team fee percentage (1000 = 10%)
    pub core_team_bps: u64,
    /// Seconds that observer has priority to execute (default 120)
    pub priority_window: i64,
}

impl ThreadConfig {
    pub fn pubkey() -> Pubkey {
        Pubkey::find_program_address(&[crate::SEED_CONFIG], &crate::ID).0
    }

    pub fn space() -> usize {
        8 + Self::INIT_SPACE
    }
}
