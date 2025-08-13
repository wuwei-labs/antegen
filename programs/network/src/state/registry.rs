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
    pub locked: bool,
    pub total_builders: u32,
    pub total_repeaters: u32,
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
    fn reset(&mut self) -> Result<()>;
    fn update_admin(&mut self, new_admin: Pubkey) -> Result<()>;
}

impl RegistryAccount for Account<'_, Registry> {
    fn init(&mut self, admin: Pubkey) -> Result<()> {
        self.version = CURRENT_REGISTRY_VERSION;
        self.admin = admin;
        self.locked = false;
        self.total_builders = 0;
        self.total_repeaters = 0;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.locked = false;
        Ok(())
    }

    fn update_admin(&mut self, new_admin: Pubkey) -> Result<()> {
        self.admin = new_admin;
        Ok(())
    }
}
