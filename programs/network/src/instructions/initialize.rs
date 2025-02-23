use {
    crate::{state::*, ANTEGEN_SQUADS},
    anchor_lang::{prelude::*, solana_program::system_program},
    std::mem::size_of,
};

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    /// CHECK: This is the predefined SQUAD multisig that will be the admin
    #[account(
        address = if cfg!(feature = "mainnet") {
            ANTEGEN_SQUADS
        } else {
            payer.key()
        }
    )]
    pub admin: UncheckedAccount<'info>,

    #[account(
        init,
        seeds = [SEED_CONFIG],
        bump,
        payer = payer,
        space = 8 + size_of::<Config>(),
    )]
    pub config: Account<'info, Config>,

    #[account(
        init,
        seeds = [SEED_REGISTRY],
        bump,
        payer = payer,
        space = 8 + size_of::<Registry>(),
    )]
    pub registry: Account<'info, Registry>,

    #[account(
        init,
        seeds = [
            SEED_SNAPSHOT,
            (0 as u64).to_be_bytes().as_ref(),
        ],
        bump,
        payer = payer,
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
    let snapshot = &mut ctx.accounts.snapshot;

    // Initialize accounts.
    config.init(ctx.accounts.admin.key())?;
    registry.init()?;
    snapshot.init(0)?;

    Ok(())
}
