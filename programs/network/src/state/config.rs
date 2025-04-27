use crate::*;
use anchor_lang::{prelude::*, AnchorDeserialize};
pub const SEED_CONFIG: &[u8] = b"config";

/**
 * Config
 */
#[account]
#[derive(Debug, InitSpace)]
pub struct Config {
    pub version: u64,
    pub bump: u8,
    pub admin: Pubkey,
}

#[derive(Accounts)]
pub struct MigrateConfig<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [SEED_CONFIG],
        constraint = config.admin == authority.key(),
        realloc = 8 + Config::INIT_SPACE,
        realloc::payer = authority,
        realloc::zero = false,
        bump = config.bump,
    )]
    pub config: Account<'info, Config>,
    pub system_program: Program<'info, System>,
}

impl Config {
    pub fn pubkey() -> Pubkey {
        Pubkey::find_program_address(&[SEED_CONFIG], &crate::ID).0
    }
}

/**
 * ConfigSettings
 */

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct ConfigSettings {
    pub admin: Pubkey,
}

/**
 * ConfigAccount
 */

pub trait ConfigAccount {
    fn init(&mut self, admin: Pubkey) -> Result<()>;
    fn update(&mut self, settings: ConfigSettings) -> Result<()>;
}

impl ConfigAccount for Account<'_, Config> {
    fn init(&mut self, admin: Pubkey) -> Result<()> {
        self.version = CURRENT_CONFIG_VERSION;
        self.admin = admin;
        Ok(())
    }

    fn update(&mut self, settings: ConfigSettings) -> Result<()> {
        self.version = CURRENT_CONFIG_VERSION;
        self.admin = settings.admin;
        Ok(())
    }
}
