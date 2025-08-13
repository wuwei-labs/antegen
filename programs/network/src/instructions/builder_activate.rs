use {
    crate::state::*,
    anchor_lang::prelude::*,
};

#[derive(Accounts)]
pub struct BuilderActivate<'info> {
    #[account(
        mut,
        has_one = signatory,
        address = builder.pubkey(),
    )]
    pub builder: Account<'info, Builder>,

    #[account(mut)]
    pub signatory: Signer<'info>,
}

pub fn handler(ctx: Context<BuilderActivate>) -> Result<()> {
    let builder: &mut Account<Builder> = &mut ctx.accounts.builder;

    // Activate the builder
    builder.is_active = true;
    
    Ok(())
}