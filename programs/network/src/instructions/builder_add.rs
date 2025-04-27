use {
    crate::{errors::*, state::*},
    anchor_lang::prelude::*,
};

#[derive(Accounts)]
pub struct BuilderAdd<'info> {
    #[account(address = Config::pubkey())]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        has_one = signatory,
        address = builder.pubkey(),
    )]
    pub builder: Account<'info, Builder>,

    #[account(
        mut,
        seeds = [
            SEED_POOL,
            pool.id.to_be_bytes().as_ref(),
        ],
        constraint = !pool.locked @ AntegenNetworkError::PoolLocked,
        bump = pool.bump,
    )]
    pub pool: Account<'info, Pool>,

    #[account(mut)]
    pub signatory: Signer<'info>,
}

pub fn handler(ctx: Context<BuilderAdd>) -> Result<()> {
    let pool: &mut Account<Pool> = &mut ctx.accounts.pool;
    let builder: &mut Account<Builder> = &mut ctx.accounts.builder;

    // Add the builder to a pool.
    builder.pool = pool.id;
    pool.add_builder(builder.key())?;
    Ok(())
}
