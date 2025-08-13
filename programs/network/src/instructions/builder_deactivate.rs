use {
    crate::state::*,
    anchor_lang::prelude::*,
};

#[derive(Accounts)]
pub struct BuilderDeactivate<'info> {
    #[account(
        mut,
        has_one = signatory,
        address = builder.pubkey(),
    )]
    pub builder: Account<'info, Builder>,

    #[account(mut)]
    pub signatory: Signer<'info>,
}

pub fn handler(ctx: Context<BuilderDeactivate>) -> Result<()> {
    let builder: &mut Account<Builder> = &mut ctx.accounts.builder;

    // Deactivate the builder
    builder.is_active = false;
    
    Ok(())
}