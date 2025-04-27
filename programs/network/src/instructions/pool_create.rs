use {
    crate::{errors::*, state::*},
    anchor_lang::{prelude::*, solana_program::system_program},
};

#[derive(Accounts)]
pub struct PoolCreate<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        init_if_needed,
        seeds = [
            SEED_POOL,
            registry.total_pools.to_be_bytes().as_ref(),
        ],
        payer = payer,
        space = 8 + Pool::INIT_SPACE,
        bump,
    )]
    pub pool: Account<'info, Pool>,

    #[account(
        mut,
        address = Registry::pubkey(),
        constraint = !registry.locked @ AntegenNetworkError::RegistryLocked
    )]
    pub registry: Box<Account<'info, Registry>>,

    #[account(address = system_program::ID)]
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<PoolCreate>) -> Result<()> {
    let pool: &mut Account<Pool> = &mut ctx.accounts.pool;
    let registry: &mut Box<Account<Registry>> = &mut ctx.accounts.registry;

    // Initialize the pool account.
    pool.init(registry.total_pools)?;
    pool.bump = ctx.bumps.pool;

    // Increment the registry's pool counter.
    registry.total_pools = registry
        .total_pools
        .checked_add(1)
        .ok_or(error!(AntegenNetworkError::PoolOverflow))?;
    Ok(())
}
