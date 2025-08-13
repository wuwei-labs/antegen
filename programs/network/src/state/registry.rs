use crate::*;
use anchor_lang::{prelude::*, AnchorDeserialize};
pub const SEED_REGISTRY: &[u8] = b"registry";

/// Registry - Global state for the Antegen network
#[account]
#[derive(Debug, InitSpace)]
pub struct Registry {
    pub version: u64,
    pub bump: u8,
    pub admin: Pubkey,
    pub commission_fee: u64,                  // Base fee in lamports
    pub builder_commission_bps: u64,          // 8500 (85%)
    pub submitter_commission_bps: u64,        // 500 (5%)
    pub core_team_bps: u64,                   // 1000 (10%)
    pub total_builders: u32,
}

#[derive(Accounts)]
pub struct MigrateRegistry<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [SEED_REGISTRY],
        constraint = registry.admin == authority.key(),
        realloc = 8 + Registry::INIT_SPACE,
        realloc::payer = authority,
        realloc::zero = false,
        bump = registry.bump,
    )]
    pub registry: Account<'info, Registry>,

    pub system_program: Program<'info, System>,
}

impl Registry {
    pub fn pubkey() -> Pubkey {
        Pubkey::find_program_address(&[SEED_REGISTRY], &crate::ID).0
    }
}

/**
 * RegistryAccount
 */
pub trait RegistryAccount {
    fn init(&mut self, admin: Pubkey) -> Result<()>;
    fn update_admin(&mut self, new_admin: Pubkey) -> Result<()>;
}

impl RegistryAccount for Account<'_, Registry> {
    fn init(&mut self, admin: Pubkey) -> Result<()> {
        self.version = CURRENT_REGISTRY_VERSION;
        self.admin = admin;
        self.commission_fee = 1000; // 1000 lamports
        self.builder_commission_bps = 8500;   // 85%
        self.submitter_commission_bps = 500;  // 5%
        self.core_team_bps = 1000;           // 10%
        self.total_builders = 0;
        Ok(())
    }

    fn update_admin(&mut self, new_admin: Pubkey) -> Result<()> {
        self.admin = new_admin;
        Ok(())
    }
}
