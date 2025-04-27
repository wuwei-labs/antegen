use {crate::state::*, anchor_lang::prelude::*};

#[derive(Accounts)]
pub struct BuilderRemove<'info> {
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
        bump = pool.bump,
    )]
    pub pool: Account<'info, Pool>,

    #[account(mut)]
    pub signatory: Signer<'info>,
}

pub fn handler(ctx: Context<BuilderRemove>) -> Result<()> {
    let pool: &mut Account<Pool> = &mut ctx.accounts.pool;
    let builder: &mut Account<Builder> = &mut ctx.accounts.builder;

    // Remove the builder from a pool.
    builder.pool = 0;
    pool.remove_builder(builder.key())?;
    Ok(())
}
