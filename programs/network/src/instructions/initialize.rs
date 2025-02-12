use {
    crate::{state::*, ANTEGEN_SQUADS},
    anchor_lang::{prelude::*, solana_program::system_program},
    std::mem::size_of,
};

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        mut,
        address = if cfg!(feature = "mainnet") {
            ANTEGEN_SQUADS
        } else {
            admin.key()
        }
    )]
    pub admin: Signer<'info>,

    #[account(
        init,
        seeds = [SEED_CONFIG],
        bump,
        payer = admin,
        space = 8 + size_of::<Config>(),
    )]
    pub config: Account<'info, Config>,

    #[account(
        init,
        seeds = [SEED_REGISTRY],
        bump,
        payer = admin,
        space = 8 + size_of::<Registry>(),
    )]
    pub registry: Account<'info, Registry>,

    #[account(
        init,
        payer = admin,
        space = 8 + std::mem::size_of::<RegistryFee>(),
        seeds = [SEED_REGISTRY_FEE, registry.key().as_ref()],
        bump
    )]
    pub registry_fee: Account<'info, RegistryFee>,

    #[account(
        init,
        seeds = [
            SEED_SNAPSHOT,
            (0 as u64).to_be_bytes().as_ref(),
        ],
        bump,
        payer = admin,
        space = 8 + size_of::<Snapshot>(),
    )]
    pub snapshot: Account<'info, Snapshot>,

    #[account(address = system_program::ID)]
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<Initialize>) -> Result<()> {
    // Get accounts
    let config = &mut ctx.accounts.config;
    let registry = &mut ctx.accounts.registry;
    let registry_fee = &mut ctx.accounts.registry_fee;
    let snapshot = &mut ctx.accounts.snapshot;

    // Initialize accounts.
    config.init(ctx.accounts.admin.key())?;
    registry.init()?;
    registry_fee.init(registry.key())?;
    snapshot.init(0)?;

    Ok(())
}
