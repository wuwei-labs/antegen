use anchor_lang::{prelude::*, AnchorDeserialize};

pub const SEED_CONFIG: &[u8] = b"config";

/**
 * Config
 */

#[account]
#[derive(Debug)]
pub struct Config {
    pub admin: Pubkey,
    pub epoch_thread: Pubkey,
    pub hasher_thread: Pubkey
}

impl Config {
    pub fn pubkey() -> Pubkey {
        Pubkey::find_program_address(&[SEED_CONFIG], &crate::ID).0
    }
}

/**
 * ConfigSettings
 */

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct ConfigSettings {
    pub admin: Pubkey,
    pub epoch_thread: Pubkey,
    pub hasher_thread: Pubkey
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
        self.admin = admin;
        Ok(())
    }

    fn update(&mut self, settings: ConfigSettings) -> Result<()> {
        self.admin = settings.admin;
        self.epoch_thread = settings.epoch_thread;
        self.hasher_thread = settings.hasher_thread;
        Ok(())
    }
}
