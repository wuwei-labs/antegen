use {
    crate::state::*,
    anchor_lang::prelude::*,
};

#[derive(Accounts)]
pub struct RegistryUpdate<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        mut,
        address = Registry::pubkey(),
        constraint = registry.admin == admin.key(),
    )]
    pub registry: Account<'info, Registry>,
}

pub fn handler(ctx: Context<RegistryUpdate>, new_admin: Pubkey) -> Result<()> {
    let registry: &mut Account<Registry> = &mut ctx.accounts.registry;
    
    // Update the admin
    registry.update_admin(new_admin)?;
    
    Ok(())
}