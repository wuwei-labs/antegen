use crate::errors::AntegenNetworkError;
use crate::*;
use anchor_lang::{prelude::*, AnchorDeserialize};

pub const SEED_POOL: &[u8] = b"pool";
pub const POOL_SIZE_MAX: u8 = 10;

/**
 * Pool
 */

#[account]
#[derive(Debug, InitSpace)]
pub struct Pool {
    pub version: u64,
    pub bump: u8,
    pub id: u8,
    pub locked: bool,
    #[max_len(POOL_SIZE_MAX)]
    pub builders: Vec<Pubkey>,
}

#[derive(Accounts)]
#[instruction(pool_id: u8)]
pub struct MigratePool<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [
            SEED_POOL,
            pool_id.to_be_bytes().as_ref(),
        ],
        constraint = config.admin == authority.key(),
        realloc = 8 + Pool::INIT_SPACE,
        realloc::payer = authority,
        realloc::zero = false,
        bump,
    )]
    pub pool: Account<'info, Pool>,

    #[account(
        seeds = [SEED_CONFIG],
        bump = config.bump,
    )]
    pub config: Account<'info, Config>,
    pub system_program: Program<'info, System>,
}

impl Pool {
    pub fn pubkey(id: u8) -> Pubkey {
        Pubkey::find_program_address(&[SEED_POOL, id.to_be_bytes().as_ref()], &crate::ID).0
    }
}

/**
 * PoolAccount
 */
pub trait PoolAccount {
    fn pubkey(&self) -> Pubkey;
    fn init(&mut self, id: u8) -> Result<()>;
    fn add_builder(&mut self, builder: Pubkey) -> Result<()>;
    fn remove_builder(&mut self, builder: Pubkey) -> Result<()>;
}

impl PoolAccount for Account<'_, Pool> {
    fn pubkey(&self) -> Pubkey {
        Pool::pubkey(self.id)
    }

    fn init(&mut self, id: u8) -> Result<()> {
        self.id = id;
        self.builders = Vec::new();
        Ok(())
    }

    fn add_builder(&mut self, builder: Pubkey) -> Result<()> {
        // Check if the builder is already in the pool
        require!(
            !self.builders.contains(&builder),
            AntegenNetworkError::BuilderInPool
        );

        // Check if there's space available based on capacity
        require!(
            self.builders.len() < POOL_SIZE_MAX as usize,
            AntegenNetworkError::PoolFull
        );

        // Push new builder into the pool.
        self.builders.push(builder);
        Ok(())
    }

    fn remove_builder(&mut self, builder: Pubkey) -> Result<()> {
        // Check if the builder exists in the pool first
        require!(
            self.builders.contains(&builder),
            AntegenNetworkError::BuilderNotInPool
        );

        self.builders.retain(|w| w != &builder);
        Ok(())
    }
}
