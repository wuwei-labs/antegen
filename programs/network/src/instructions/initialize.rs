use {
    crate::{state::*, ANTEGEN_SQUADS},
    anchor_lang::{prelude::*, solana_program::system_program},
    std::mem::size_of,
};

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        init,
        seeds = [SEED_CONFIG],
        payer = payer,
        space = 8 + size_of::<Config>(),
        bump,
    )]
    pub config: Account<'info, Config>,

    #[account(
        init,
        seeds = [SEED_REGISTRY],
        payer = payer,
        space = 8 + size_of::<Registry>(),
        bump,
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
    let payer: &Signer = &ctx.accounts.payer;
    let admin: Pubkey = if cfg!(feature = "mainnet") {
        ANTEGEN_SQUADS
    } else {
        payer.key()
    };

    let config: &mut Account<Config> = &mut ctx.accounts.config;
    let registry: &mut Account<Registry> = &mut ctx.accounts.registry;
    let snapshot: &mut Account<Snapshot> = &mut ctx.accounts.snapshot;

    // Initialize accounts.
    config.init(admin)?;
    registry.init()?;
    snapshot.init(0)?;

    Ok(())
}
