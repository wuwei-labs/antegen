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
    let registry: &mut Account<Registry> = &mut ctx.accounts.registry;

    // Initialize registry with admin set to payer
    registry.init(payer.key())?;
    registry.bump = ctx.bumps.registry;
    
    Ok(())
}
