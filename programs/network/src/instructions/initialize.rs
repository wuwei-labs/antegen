use {
    crate::state::*,
    anchor_lang::{prelude::*, solana_program::system_program},
};

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        init,
        seeds = [SEED_CONFIG],
        payer = payer,
        space = 8 + Config::INIT_SPACE,
        bump,
    )]
    pub config: Account<'info, Config>,

    #[account(
        init,
        seeds = [SEED_REGISTRY],
        payer = payer,
        space = 8 + Registry::INIT_SPACE,
        bump,
    )]
    pub registry: Account<'info, Registry>,

    #[account(address = system_program::ID)]
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<Initialize>) -> Result<()> {
    let payer: &Signer = &ctx.accounts.payer;
    let config: &mut Account<Config> = &mut ctx.accounts.config;
    let registry: &mut Account<Registry> = &mut ctx.accounts.registry;

    // Initialize accounts.
    config.init(payer.key())?;
    config.bump = ctx.bumps.config;

    registry.init()?;
    registry.bump = ctx.bumps.registry;
    Ok(())
}
