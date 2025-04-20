use {crate::state::*, anchor_lang::prelude::*};

#[derive(Accounts)]
pub struct RegistryUnlock<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        has_one = admin,
        seeds = [SEED_CONFIG],
        bump = config.bump,
    )]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        seeds = [SEED_REGISTRY],
        bump = registry.bump,
    )]
    pub registry: Account<'info, Registry>,
}

pub fn handler(ctx: Context<RegistryUnlock>) -> Result<()> {
    let registry: &mut Account<Registry> = &mut ctx.accounts.registry;
    registry.locked = false;
    Ok(())
}
