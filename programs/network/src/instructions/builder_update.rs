use {crate::state::*, anchor_lang::prelude::*};

#[derive(Accounts)]
#[instruction(settings: BuilderSettings)]
pub struct BuilderUpdate<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        has_one = authority,
        seeds = [
            SEED_BUILDER,
            builder.id.to_be_bytes().as_ref(),
        ],
        bump = builder.bump,
    )]
    pub builder: Account<'info, Builder>,

    #[account(
        address = Registry::pubkey(),
    )]
    pub registry: Account<'info, Registry>,
}

pub fn handler(ctx: Context<BuilderUpdate>, settings: BuilderSettings) -> Result<()> {
    let builder: &mut Account<Builder> = &mut ctx.accounts.builder;
    let registry: &Account<Registry> = &ctx.accounts.registry;

    // Update the builder
    builder.update(settings, registry.builder_commission_bps)?;
    Ok(())
}
