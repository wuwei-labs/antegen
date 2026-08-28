use crate::{constants::*, state::*};
use anchor_lang::prelude::*;

/// Accounts required by the `config_initialize` instruction.
#[derive(Accounts)]
pub struct ConfigInit<'info> {
    /// The admin initializing the config
    #[account(mut)]
    pub admin: Signer<'info>,

    /// The config account to initialize
    #[account(
        init,
        payer = admin,
        space = ThreadConfig::space(),
        seeds = [SEED_CONFIG],
        bump
    )]
    pub config: Account<'info, ThreadConfig>,

    /// System program
    pub system_program: Program<'info, System>,
}

pub fn config_init(ctx: Context<ConfigInit>) -> Result<()> {
    let config = &mut ctx.accounts.config;
    let admin = &ctx.accounts.admin;

    // The starting fee policy lives on the type; only the two fields that
    // depend on this instruction's accounts are filled in here.
    **config = ThreadConfig {
        bump: ctx.bumps.config,
        admin: admin.key(),
        ..Default::default()
    };

    // `init` assigns ownership as part of account creation. Assert it rather
    // than assume it, so the invariant the rest of the program relies on is
    // stated where the account is created.
    require!(
        config.to_account_info().owner == &crate::ID,
        crate::errors::AntegenThreadError::InvalidAccountOwner
    );

    msg!("Thread config initialized with admin: {}", admin.key());

    Ok(())
}
