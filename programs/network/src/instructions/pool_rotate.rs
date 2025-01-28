use {
    crate::{errors::*, state::*},
    anchor_lang::prelude::*,
};

#[derive(Accounts)]
pub struct PoolRotate<'info> {
    #[account(address = Config::pubkey())]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        seeds = [
            SEED_POOL,
            pool.id.to_be_bytes().as_ref(),
        ],
        bump,
    )]
    pub pool: Account<'info, Pool>,

    #[account(mut)]
    pub signatory: Signer<'info>,

    #[account(
        address = worker.pubkey(),
        has_one = signatory
    )]
    pub worker: Account<'info, Worker>,
}

pub fn handler(ctx: Context<PoolRotate>) -> Result<()> {
    // Get accounts
    let pool = &mut ctx.accounts.pool;
    let worker = &ctx.accounts.worker;

    // Verify the pool has excess space.
    require!(
        pool.workers.len() < (pool.size as usize),
        AntegenError::PoolFull
    );

    // Verify the worker is not already in the pool.
    require!(
        !pool.workers.contains(&worker.key()),
        AntegenError::AlreadyInPool
    );

    // Rotate the worker into the pool.
    pool.rotate(worker.key())?;

    Ok(())
}
