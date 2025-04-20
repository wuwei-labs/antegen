use {
    crate::state::*,
    anchor_lang::{prelude::*, solana_program::system_program},
};

#[derive(Accounts)]
pub struct RegistryReset<'info> {
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

    #[account(address = system_program::ID)]
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<RegistryReset>) -> Result<()> {
    let registry: &mut Account<Registry> = &mut ctx.accounts.registry;

    // Reset accounts to their initial state
    registry.reset()?;
    Ok(())
}
