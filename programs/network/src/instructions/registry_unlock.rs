use {crate::state::*, anchor_lang::prelude::*};

#[derive(Accounts)]
pub struct RegistryUnlock<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        address = Config::pubkey(),
        has_one = admin
    )]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        address = Registry::pubkey()
    )]
    pub registry: Account<'info, Registry>,
}

pub fn handler(ctx: Context<RegistryUnlock>) -> Result<()> {
    let registry: &mut Account<Registry> = &mut ctx.accounts.registry;
    registry.locked = false;
    Ok(())
}
