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
}

pub fn handler(ctx: Context<BuilderUpdate>, settings: BuilderSettings) -> Result<()> {
    let builder: &mut Account<Builder> = &mut ctx.accounts.builder;

    // Update the builder
    builder.update(settings)?;
    Ok(())
}
