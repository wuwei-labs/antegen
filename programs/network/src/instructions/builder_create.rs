use {
    crate::{errors::*, state::*},
    anchor_lang::{
        prelude::*,
        solana_program::system_program,
    }
};

#[derive(Accounts)]
pub struct BuilderCreate<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(constraint = signatory.key().ne(&authority.key()) @ AntegenNetworkError::InvalidSignatory)]
    pub signatory: Signer<'info>,

    #[account(
        init,
        seeds = [
            SEED_BUILDER,
            registry.total_builders.saturating_add(1).to_be_bytes().as_ref(),
        ],
        payer = authority,
        space = 8 + Builder::INIT_SPACE,
        bump,
    )]
    pub builder: Account<'info, Builder>,

    #[account(
        mut, 
        address = Registry::pubkey(),
        constraint = !registry.locked @ AntegenNetworkError::RegistryLocked
    )]
    pub registry: Account<'info, Registry>,

    #[account(address = system_program::ID)]
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<BuilderCreate>) -> Result<()> {
    let authority: &mut Signer = &mut ctx.accounts.authority;
    let registry: &mut Account<Registry> = &mut ctx.accounts.registry;
    let signatory: &mut Signer = &mut ctx.accounts.signatory;
    let builder: &mut Account<Builder> = &mut ctx.accounts.builder;

    let builder_id: u32 = registry.total_builders.saturating_add(1);
    // Initialize the builder accounts.
    builder.init(authority, builder_id, signatory)?;
    builder.bump = ctx.bumps.builder;

    // Update the registry's builder counter.
    registry.total_builders = builder_id;
    Ok(())
}
