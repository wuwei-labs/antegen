use {crate::state::*, anchor_lang::prelude::*};

#[derive(Accounts)]
pub struct RegistryUnlock<'info> {
    #[account(address = config.admin)]
    pub admin: Signer<'info>,
  
    #[account(
        address = Config::pubkey(),
        has_one = admin
    )]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        seeds = [SEED_REGISTRY],
        bump
    )]
    pub registry: Account<'info, Registry>,
}

pub fn handler(ctx: Context<RegistryUnlock>) -> Result<()> {
    let registry = &mut ctx.accounts.registry;
    registry.locked = false;
    Ok(())
}
