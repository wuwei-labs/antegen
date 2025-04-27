use crate::*;
use anchor_lang::{prelude::*, AnchorDeserialize};
pub const SEED_REGISTRY: &[u8] = b"registry";

/// Registry
#[account]
#[derive(Debug, InitSpace)]
pub struct Registry {
    pub version: u64,
    pub bump: u8,
    pub locked: bool,
    pub total_pools: u8,
    pub total_builders: u32,
}

#[derive(Accounts)]
pub struct MigrateRegistry<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [SEED_REGISTRY],
        constraint = config.admin == authority.key(),
        realloc = 8 + Registry::INIT_SPACE,
        realloc::payer = authority,
        realloc::zero = false,
        bump = config.bump,
    )]
    pub builder: Account<'info, Registry>,

    #[account(
        seeds = [SEED_CONFIG],
        bump = config.bump,
      )]
    pub config: Account<'info, Config>,
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
    fn init(&mut self) -> Result<()>;
    fn reset(&mut self) -> Result<()>;
}

impl RegistryAccount for Account<'_, Registry> {
    fn init(&mut self) -> Result<()> {
        self.locked = false;
        self.total_builders = 0;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.locked = false;
        Ok(())
    }
}
